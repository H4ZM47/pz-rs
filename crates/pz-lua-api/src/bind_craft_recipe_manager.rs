//! `zombie.entity.components.crafting.recipe.CraftRecipeManager`: the static
//! `getUniqueRecipeItems`, which `ISInventoryPaneContextMenu.createMenu`
//! calls for each selected item, and the parts of the manager it needs:
//! the recipe tag query (`queryRecipes`, over `TaggedObjectManager`),
//! `isValidRecipeForCharacter`, `getValidInputScriptForItem` with the item
//! status tests (`item_status`), and the context-click part of
//! `HandcraftLogic` (`handcraft`).
//!
//! Not ported yet, and reported with `unsupported` when reached:
//! - `CraftRecipeData.createRecipeOutputs` (and after it the craft surface
//!   and workstation tests) for a recipe whose inputs the containers cover;
//! - `IsoPlayer.tooDarkToRead` for such a recipe;
//! - `CraftRecipe.validateBenefitFromRecipeAtHand` for a learned recipe
//!   whose skill requirement the character does not meet
//!   (`tooDarkToRead`, `hasRecipeAtHand`);
//! - the rarer item reads listed in `item_status`.

mod handcraft;
pub mod item_status;
mod logic;

use crate::bind_character_sim::CharacterObject;
use crate::bind_item_container::LuaItemContainer;
use crate::bind_items::ItemRef;
use crate::bind_java_util::JavaList;
use crate::bind_scripts::script_list;
use crate::expose::expose_class;
use item_status::{CraftItem, CraftItemClass, FoodState};
use pz_characters::Characters;
use pz_inventory::item::ItemKind;
use pz_inventory::{ItemClass, ItemOwner, ItemPlace};
use pz_kahlua::{LuaError, LuaResult, TableRef, Thread, Value};
use pz_scripts::java;
use pz_scripts::objects::entity::components::crafting::CraftRecipe;
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
    /// `CraftRecipeManager.itemGetQueuedAmount(chr, item, cache)`: how much
    /// of the item the queued `ISHandcraftAction`s use.
    fn item_queued_amount(&self, item: &dyn CraftItem) -> LuaResult<f32>;
    /// `IsoPlayer.tooDarkToRead()`.
    fn too_dark_to_read(&self) -> LuaResult<bool> {
        Err(crate::bind_world_object::unsupported("IsoPlayer.tooDarkToRead"))
    }
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

/// `CraftRecipeManager.getValidInputScriptForItem(recipe, item, chr)`
/// (through `isItemValidForInputScript` and `consumeInputItem(input, item,
/// true, null, chr)`): the index (into `getInputs()`) of the first `Item`
/// input that accepts the item.
fn valid_input_script_for_item(
    recipe: &CraftRecipe,
    item: &dyn CraftItem,
    chr: &dyn RecipeCharacter,
    on_test: &mut dyn FnMut(&str, usize) -> bool,
) -> LuaResult<Option<usize>> {
    for (i, input) in recipe.inputs().enumerate() {
        if input.resource_type() != ResourceType::Item {
            continue;
        }
        if item_status::consume_input_item_test(input, recipe, item, chr, on_test)? {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// `CraftRecipeManager.getUniqueRecipeItems(item, chr, containers)`: the
/// `InHandCraft`/`AnySurfaceCraft` recipes that are valid for the character,
/// have an input that accepts the item, pass `OnTestItem`, and that a
/// `HandcraftLogic` set to the recipe from the context click (with the
/// containers' items) can perform.
///
/// `container_items` are the items of the containers in order
/// (`getAllItemsFromContainers`); `item` is the clicked item. The original
/// builds the `HandcraftLogic` before the loop with `findCraftSurface(chr,
/// 2)`, which only `canPerformCurrentRecipe` reads after the outputs (see
/// `handcraft`). It returns its one static `RecipeList`, cleared on each
/// call; this returns a new list.
pub fn unique_recipe_items(
    sm: &ScriptManager,
    item: &dyn CraftItem,
    chr: &dyn RecipeCharacter,
    container_items: &[&dyn CraftItem],
    on_test: &mut dyn FnMut(&str, usize) -> bool,
) -> LuaResult<Vec<ScriptRef>> {
    use pz_scripts::objects::entity::components::crafting::craft_recipe_tag::{ANY_SURFACE_CRAFT, IN_HAND_CRAFT};
    let recipes = query_recipes_for_tags(sm, &[IN_HAND_CRAFT, ANY_SURFACE_CRAFT])?;
    let mut items: Vec<&dyn CraftItem> = container_items.to_vec();
    let clicked = match items.iter().position(|i| i.key() == item.key()) {
        Some(i) => i,
        None => {
            items.push(item);
            items.len() - 1
        }
    };
    let mut out = Vec::new();
    for script in recipes {
        let Some(recipe) = script.downcast::<CraftRecipe>() else {
            continue;
        };
        let recipe = recipe.borrow();
        if !is_valid_recipe_for_character(&recipe, Some(chr), true)? {
            continue;
        }
        if valid_input_script_for_item(&recipe, item, chr, on_test)?.is_none() {
            continue;
        }
        if !item_status::on_test_item(&recipe, item.key(), on_test) {
            continue;
        }
        if handcraft::can_perform_from_context_click(&recipe, &items, container_items.len(), clicked, chr, on_test)? {
            out.push(script.clone());
        }
    }
    Ok(out)
}

/// `BaseCraftingLogic.createCachedRecipeInfo(recipe, null)` without the
/// `HandcraftLogic` surface test: `isValid` (`isValidRecipeForCharacter`
/// with no containers) and `canPerform` (see
/// `handcraft::can_perform_with_items`) for the logic's player and items.
pub(crate) fn cached_recipe_info(
    thread: &mut Thread,
    sm: &ScriptManager,
    chars: &RefCell<Characters>,
    chr_value: &Value,
    recipe: &CraftRecipe,
    items: &[ItemRef],
) -> LuaResult<(bool, bool)> {
    let character = match chr_value {
        Value::Object(o) => (&**o as &dyn Any)
            .downcast_ref::<CharacterObject>()
            .ok_or_else(|| LuaError::runtime("expected IsoGameCharacter"))?,
        _ => return Err(npe("IsoGameCharacter")),
    };
    let lua_chr = LuaRecipeCharacter { chars, character, value: chr_value.clone(), env: thread.globals().clone() };
    if !is_valid_recipe_for_character(recipe, Some(&lua_chr), false)? {
        return Ok((false, false));
    }
    let items = items.iter().map(|i| LuaCraftItem::new(sm, i.clone(), chr_value)).collect::<LuaResult<Vec<_>>>()?;
    let values: Vec<(usize, Value)> = items.iter().map(|i| (i.key(), i.value.clone())).collect();
    let mut on_test = |name: &str, key: usize| {
        let item = values.iter().find(|(k, _)| *k == key).map_or(Value::Nil, |(_, v)| v.clone());
        call_on_test(thread, name, &item, chr_value)
    };
    let refs: Vec<&dyn CraftItem> = items.iter().map(|i| i as &dyn CraftItem).collect();
    let can_perform = handcraft::can_perform_with_items(recipe, &refs, &lua_chr, &mut on_test)?;
    Ok((true, can_perform))
}

/// The `IsoGameCharacter` argument, read through the shared characters.
struct LuaRecipeCharacter<'a> {
    chars: &'a RefCell<Characters>,
    character: &'a CharacterObject,
    value: Value,
    /// `LuaManager.env`.
    env: TableRef,
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
    /// `ISTimedActionQueue.queues[chr].queue`: 0 without a queue; the
    /// `ISHandcraftAction` entries (their `manualInputs`) are not ported.
    fn item_queued_amount(&self, _item: &dyn CraftItem) -> LuaResult<f32> {
        let table = |v: Value| match v {
            Value::Table(t) => Some(t),
            _ => None,
        };
        let Some(queue) = table(self.env.rawget(&Value::str("ISTimedActionQueue")))
            .and_then(|q| table(q.rawget(&Value::str("queues"))))
            .and_then(|q| table(q.rawget(&self.value)))
            .and_then(|q| table(q.rawget(&Value::str("queue"))))
        else {
            return Ok(0.0);
        };
        for (_, action) in queue.entries() {
            let is_handcraft = table(action)
                .and_then(|a| a.metatable())
                .is_some_and(|m| matches!(m.get_str("Type"), Value::Str(s) if &*s == "ISHandcraftAction"));
            if is_handcraft {
                return Err(crate::bind_world_object::unsupported(
                    "CraftRecipeManager.itemGetQueuedAmount (ISHandcraftAction)",
                ));
            }
        }
        Ok(0.0)
    }
}

/// An `InventoryItem` argument or container item, read through its holder.
struct LuaCraftItem {
    item: ItemRef,
    value: Value,
    owner: Rc<dyn ItemOwner>,
    script: Option<Rc<RefCell<Item>>>,
    /// The character argument, for `isNoRecipes(player)`.
    player: Value,
}

impl LuaCraftItem {
    fn new(sm: &ScriptManager, item: ItemRef, player: &Value) -> LuaResult<Self> {
        let value = crate::bind_items::try_item_value(&item)?;
        let owner = crate::bind_items::item_env().owner(&item);
        let script = pz_characters::character::items::script_item(sm, &item.borrow());
        Ok(LuaCraftItem { item, value, owner, script, player: player.clone() })
    }
}

impl CraftItem for LuaCraftItem {
    fn key(&self) -> usize {
        Rc::as_ptr(&self.item) as *const () as usize
    }
    fn script_item(&self) -> Option<Rc<RefCell<Item>>> {
        self.script.clone()
    }
    fn full_type(&self) -> String {
        self.item.borrow().get_full_type()
    }
    fn current_uses(&self) -> i32 {
        self.item.borrow().get_current_uses()
    }
    fn max_uses(&self) -> i32 {
        self.item.borrow().get_max_uses()
    }
    fn class(&self) -> CraftItemClass {
        match self.item.borrow().template.class {
            ItemClass::Container => CraftItemClass::Container,
            ItemClass::Drainable => CraftItemClass::Drainable,
            ItemClass::Radio => CraftItemClass::Radio,
            ItemClass::WeaponPart => CraftItemClass::WeaponPart,
            _ => CraftItemClass::Other,
        }
    }
    fn is_no_recipes(&self) -> LuaResult<bool> {
        crate::bind_items::row_read_flags::is_no_recipes_value(&self.item, &self.player)
    }
    fn is_favorite(&self) -> bool {
        self.item.borrow().favorite
    }
    fn food(&self) -> Option<FoodState> {
        let i = self.item.borrow();
        let ItemKind::Food(f) = &i.kind else {
            return None;
        };
        Some(FoodState {
            rotten: i.is_rotten(),
            frozen: f.frozen,
            cookable: f.is_cookable,
            cooked: i.is_cooked(),
            burnt: i.is_burnt(),
        })
    }
    fn is_broken(&self) -> bool {
        self.item.borrow().is_broken()
    }
    fn in_dead_body(&self) -> bool {
        matches!(self.owner.place(&self.item.borrow()), ItemPlace::DeadBody { .. })
    }
    fn is_worn(&self) -> bool {
        let i = self.item.borrow();
        matches!(i.kind, ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)) && self.owner.is_worn(&i)
    }
    fn is_equipped(&self) -> bool {
        self.owner.is_equipped(&self.item.borrow())
    }
    fn condition(&self) -> i32 {
        self.item.borrow().condition
    }
    fn condition_max(&self) -> i32 {
        self.item.borrow().condition_max
    }
    fn is_full_uses(&self) -> bool {
        let i = self.item.borrow();
        i.uses >= i.get_max_uses()
    }
    fn is_empty_uses(&self) -> bool {
        self.item.borrow().get_current_uses_float() <= 0.0
    }
    fn container_is_empty(&self) -> bool {
        match &self.item.borrow().kind {
            ItemKind::Container(c) => c.container.slot().is_none_or(|c| c.borrow().is_empty()),
            _ => true,
        }
    }
    fn is_fluid_container(&self) -> LuaResult<bool> {
        Ok(crate::bind_world_object::fluid_component(&self.value)?.is_some())
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

fn npe(what: &str) -> LuaError {
    LuaError::Runtime { message: format!("{what} is null"), class: "java.lang.NullPointerException" }
}

/// `getAllItemsFromContainers(containers, allItems)`: each
/// `ItemContainer`'s items in order.
pub(crate) fn container_items(containers: &Value) -> LuaResult<Vec<ItemRef>> {
    let Value::Object(o) = containers else {
        return Err(npe("containers"));
    };
    let list = o.as_any().downcast_ref::<JavaList>().ok_or_else(|| LuaError::runtime("expected ArrayList"))?;
    let mut items = Vec::new();
    for c in list.snapshot() {
        let container = match &c {
            Value::Object(o) => o.as_any().downcast_ref::<LuaItemContainer>(),
            _ => None,
        };
        let container = container.ok_or_else(|| LuaError::runtime("ClassCastException: expected ItemContainer"))?;
        items.extend(container.container.borrow().items.iter().cloned());
    }
    Ok(items)
}

pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    logic::register(thread, sm.clone(), chars.clone());
    let b = expose_class(thread, CRAFT_RECIPE_MANAGER_CLASS, None);
    let query_sm = sm.clone();
    b.static_fn("queryRecipes", move |c| {
        let query = match c.arg(0) {
            Value::Str(s) => s.to_string(),
            Value::Nil => String::new(),
            _ => return Err(LuaError::runtime("expected String")),
        };
        let list = query_recipes(&query_sm, &query)?;
        Ok(c.push(script_list(&list)))
    });
    b.static_fn("getUniqueRecipeItems", move |c| {
        let item = crate::bind_world_object::item_argument(&c.arg(0))?;
        let chr_value = c.arg(1);
        // `findCraftSurface(chr, 2)` reads `chr.getCurrentSquare()`, and
        // `setContainers` copies the list.
        let character = match &chr_value {
            Value::Object(o) => (&**o as &dyn Any)
                .downcast_ref::<CharacterObject>()
                .ok_or_else(|| LuaError::runtime("expected IsoGameCharacter"))?,
            Value::Nil => return Err(npe("IsoGameCharacter")),
            _ => return Err(LuaError::runtime("expected IsoGameCharacter")),
        };
        let containers = container_items(&c.arg(2))?;
        let lua_chr =
            LuaRecipeCharacter { chars: &chars, character, value: chr_value.clone(), env: c.thread.globals().clone() };
        let item = LuaCraftItem::new(&sm, item, &chr_value)?;
        let containers =
            containers.into_iter().map(|i| LuaCraftItem::new(&sm, i, &chr_value)).collect::<LuaResult<Vec<_>>>()?;
        let values: Vec<(usize, Value)> =
            containers.iter().chain([&item]).map(|i| (i.key(), i.value.clone())).collect();
        let thread = &mut *c.thread;
        let mut on_test = |name: &str, key: usize| {
            let item = values.iter().find(|(k, _)| *k == key).map_or(Value::Nil, |(_, v)| v.clone());
            call_on_test(thread, name, &item, &chr_value)
        };
        let refs: Vec<&dyn CraftItem> = containers.iter().map(|i| i as &dyn CraftItem).collect();
        let list = unique_recipe_items(&sm, &item, &lua_chr, &refs, &mut on_test)?;
        Ok(c.push(script_list(&list)))
    });
}

#[cfg(test)]
mod tests;
