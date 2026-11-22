//! `zombie.entity.components.crafting.recipe.HandcraftLogic` (with its
//! `BaseCraftingLogic` superclass) as a Lua object, for the crafting window
//! (`ISHandCraftPanel` and its widgets), with the recipe list it builds
//! (`CraftRecipeListNodeCollection` of `CraftRecipeListNode`s) and its
//! `CachedRecipeInfo`s.
//!
//! Ported for a logic without a craft bench: the constructor, the event
//! listeners, the manual input flags, the view settings kept in the
//! player's mod data, the containers and their items, the recipe list with
//! its category filter and sort, the cached recipe info, and (in `recipe`)
//! the selected recipe, the craft action and `performCurrentRecipe`. The
//! filter string, manual input selection and the input item nodes report
//! `unsupported`.

mod recipe;
pub(crate) use recipe::data_mapper_output_item;

use super::handcraft::DataState;
use super::{cached_recipe_info, container_items};
use crate::bind_character_sim::{CharacterObject, LuaGameCharacterRef};
use crate::bind_items::ItemRef;
use crate::bind_java_util::JavaList;
use crate::bind_scripts::{script_list, script_value, ScriptHandle};
use crate::bind_world_object::unsupported;
use crate::expose::{enum_value, expose_class, expose_enum, this};
use pz_characters::Characters;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_scripts::objects::entity::components::crafting::{CraftRecipe, CraftRecipeGroup};
use pz_scripts::{ScriptManager, ScriptRef};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::rc::{Rc, Weak};

pub const BASE_CRAFTING_LOGIC_CLASS: &str = "zombie.entity.components.crafting.BaseCraftingLogic";
pub const HANDCRAFT_LOGIC_CLASS: &str = "zombie.entity.components.crafting.recipe.HandcraftLogic";
pub const CRAFT_RECIPE_DATA_CLASS: &str = "zombie.entity.components.crafting.recipe.CraftRecipeData";
pub const CACHED_RECIPE_INFO_CLASS: &str = "zombie.entity.components.crafting.BaseCraftingLogic$CachedRecipeInfo";
pub const NODE_COLLECTION_CLASS: &str = "zombie.entity.components.crafting.recipe.CraftRecipeListNodeCollection";
pub const NODE_CLASS: &str = "zombie.entity.components.crafting.recipe.CraftRecipeListNode";
const NODE_TYPE_CLASS: &str = "zombie.entity.components.crafting.recipe.CraftRecipeListNode$CraftRecipeListNodeType";
const EXPANDED_STATE_CLASS: &str =
    "zombie.entity.components.crafting.recipe.CraftRecipeListNode$CraftRecipeListNodeExpandedState";

/// The `panel` argument of the view settings in `HandcraftLogic`.
const PANEL: &str = "handcraft";

/// `BaseCraftingLogic.CachedRecipeComparator.CompareMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompareMode {
    Name,
    LastUsed,
    MostUsed,
}

/// `CraftRecipeListNode.CraftRecipeListNodeExpandedState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expanded {
    Closed,
    Partial,
    Open,
}

impl Expanded {
    fn name(self) -> &'static str {
        match self {
            Expanded::Closed => "CLOSED",
            Expanded::Partial => "PARTIAL",
            Expanded::Open => "OPEN",
        }
    }
}

/// `CraftRecipeListNode`: a recipe, or a recipe group with its recipes.
pub struct Node {
    recipe: Option<ScriptRef>,
    group: Option<CraftRecipeGroup>,
    title: Option<String>,
    parent: Option<Weak<Node>>,
    expanded: Cell<Expanded>,
    children: RefCell<Vec<Rc<Node>>>,
}

impl Node {
    fn is_recipe(&self) -> bool {
        self.group.is_none()
    }
}

/// `CraftRecipeListNodeCollection`.
#[derive(Default)]
pub struct NodeCollection {
    nodes: RefCell<Vec<Rc<Node>>>,
    group_nodes: RefCell<Vec<(CraftRecipeGroup, Rc<Node>)>>,
}

impl NodeCollection {
    fn clear(&self) {
        self.nodes.borrow_mut().clear();
        self.group_nodes.borrow_mut().clear();
    }

    /// `add(recipe)`: a recipe with a group goes under that group's node,
    /// made (with `PARTIAL` state) on its first recipe.
    fn add(&self, script: &ScriptRef, recipe: &CraftRecipe) {
        let Some(group) = recipe.recipe_group() else {
            self.nodes.borrow_mut().push(Rc::new(Node {
                recipe: Some(script.clone()),
                group: None,
                title: None,
                parent: None,
                expanded: Cell::new(Expanded::Closed),
                children: RefCell::new(Vec::new()),
            }));
            return;
        };
        let existing = self.group_nodes.borrow().iter().find(|(g, _)| *g == group).map(|(_, n)| n.clone());
        let parent = existing.unwrap_or_else(|| {
            // `group.getTranslationName()`.
            let key = group.translation_key();
            let title = crate::bind_text::with_translator_mut(|t| t.get_recipe_group_name(&key)).unwrap_or(key);
            let node = Rc::new(Node {
                recipe: None,
                group: Some(group),
                title: Some(title),
                parent: None,
                expanded: Cell::new(Expanded::Partial),
                children: RefCell::new(Vec::new()),
            });
            self.group_nodes.borrow_mut().push((group, node.clone()));
            self.nodes.borrow_mut().push(node.clone());
            node
        });
        let child = Rc::new(Node {
            recipe: Some(script.clone()),
            group: None,
            title: None,
            parent: Some(Rc::downgrade(&parent)),
            expanded: Cell::new(Expanded::Closed),
            children: RefCell::new(Vec::new()),
        });
        parent.children.borrow_mut().push(child);
    }

    /// `getAllRecipes()`: the recipes in node order, a group's in place.
    fn all_recipes(&self) -> Vec<ScriptRef> {
        fn walk(nodes: &[Rc<Node>], out: &mut Vec<ScriptRef>) {
            for n in nodes {
                match &n.recipe {
                    Some(r) if n.is_recipe() => out.push(r.clone()),
                    _ => walk(&n.children.borrow(), out),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes.borrow(), &mut out);
        out
    }
}

/// `BaseCraftingLogic.CachedRecipeInfo`.
#[derive(Clone, Copy)]
struct Info {
    valid: bool,
    can_perform: bool,
    available: bool,
}

/// `CraftEventHandler`.
struct Handler {
    function: Value,
    target: Value,
}

struct State {
    player: Value,
    character: LuaGameCharacterRef,
    iso_object: Value,
    /// `events`: the registered names, each with its listeners.
    events: Vec<(&'static str, Vec<Handler>)>,
    manual_select_inputs: bool,
    show_manual_select_inputs: bool,
    /// `manualInputAllowedItemTypes`: with it set, `populateInputs` only
    /// offers items of these full types.
    manual_input_allowed_item_types: Vec<String>,
    /// `multicraftConsumedItems` (item keys): what the last
    /// `getPossibleCraftCount(true)` counted.
    multicraft_consumed_items: Vec<usize>,
    manual_select_input_script_filter: Value,
    compare_mode: CompareMode,
    containers: Vec<Value>,
    all_items: Vec<ItemRef>,
    complete_recipe_list: Vec<ScriptRef>,
    filter_string: Option<Rc<str>>,
    category_filter_string: Option<Rc<str>>,
    filtered: Rc<NodeCollection>,
    cached_infos: Vec<(ScriptRef, Info)>,
    cached_infos_dirty: bool,
    /// `recipeData.getRecipe()`.
    recipe: Option<ScriptRef>,
    /// The state of `recipeData` (no inputs while `recipe` is null).
    data: DataState,
    /// The items `data` names, kept so that their keys stay theirs.
    held: Vec<ItemRef>,
    cached_possible_craft_count: i32,
    cached_can_perform: bool,
    cached_can_perform_dirty: bool,
    craft_action_in_progress: bool,
    craft_action_table: Value,
    /// `numCraftActionInProgress`.
    craft_action_count: i32,
}

pub struct HandcraftLogicObject {
    state: RefCell<State>,
    sm: Rc<ScriptManager>,
    chars: Rc<RefCell<Characters>>,
}

impl LuaObject for HandcraftLogicObject {
    fn class_name(&self) -> &str {
        HANDCRAFT_LOGIC_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[BASE_CRAFTING_LOGIC_CLASS]
    }
}

struct NodeCollectionObject(Rc<NodeCollection>);

impl LuaObject for NodeCollectionObject {
    fn class_name(&self) -> &str {
        NODE_COLLECTION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct NodeObject(Rc<Node>);

impl LuaObject for NodeObject {
    fn class_name(&self) -> &str {
        NODE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct InfoObject {
    recipe: ScriptRef,
    info: Info,
}

impl LuaObject for InfoObject {
    fn class_name(&self) -> &str {
        CACHED_RECIPE_INFO_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The events `BaseCraftingLogic` and then `HandcraftLogic` register.
const EVENTS: [&str; 10] = [
    "onShowManualSelectChanged",
    "onRebuildInputItemNodes",
    "onManualSelectChanged",
    "onInputsChanged",
    "onUpdateRecipeList",
    "onSetRecipeList",
    "onUpdateContainers",
    "onRecipeChanged",
    "onStartCraft",
    "onStopCraft",
];

fn npe() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

fn class_cast() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.ClassCastException" }
}

fn logic(c: &NativeCall<'_>) -> LuaResult<Rc<dyn LuaObject>> {
    this::<HandcraftLogicObject>(c)
}

fn object(o: &Rc<dyn LuaObject>) -> &HandcraftLogicObject {
    o.as_any().downcast_ref::<HandcraftLogicObject>().unwrap()
}

fn state(o: &Rc<dyn LuaObject>) -> &RefCell<State> {
    &object(o).state
}

/// A `CraftRecipe` argument (null allowed).
fn recipe_arg(v: &Value) -> LuaResult<Option<ScriptRef>> {
    match v {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<ScriptHandle>() {
            Some(h) if h.root.downcast::<CraftRecipe>().is_some() => Ok(Some(h.root.clone())),
            _ => Err(class_cast()),
        },
        _ => Err(class_cast()),
    }
}

/// An optional `String` argument.
fn string_arg(v: &Value) -> LuaResult<Option<Rc<str>>> {
    match v {
        Value::Nil => Ok(None),
        Value::Str(s) => Ok(Some(s.clone())),
        _ => Err(class_cast()),
    }
}

/// `StringUtils.isNullOrWhitespace`.
fn null_or_whitespace(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.trim().is_empty())
}

/// `String.compareTo`: UTF-16 code unit order.
fn java_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `String.compareToIgnoreCase`.
fn java_compare_ignore_case(a: &str, b: &str) -> Ordering {
    let fold = |c: char| c.to_uppercase().flat_map(char::to_lowercase);
    a.chars().flat_map(fold).cmp(b.chars().flat_map(fold))
}

impl State {
    fn mod_data(&self) -> TableRef {
        self.character.borrow_mut().get_mod_data()
    }

    /// `(String) player.getModData().rawget(panel + suffix)`.
    fn mod_data_string(&self, suffix: &str) -> LuaResult<Option<Rc<str>>> {
        match self.mod_data().get_own(&Value::str(&format!("{PANEL}{suffix}"))) {
            Value::Nil => Ok(None),
            Value::Str(s) => Ok(Some(s)),
            _ => Err(class_cast()),
        }
    }

    fn set_mod_data(&self, suffix: &str, value: Value) {
        self.mod_data().rawset(Value::str(&format!("{PANEL}{suffix}")), value);
    }

    /// `getRecipeSortMode(panel)`.
    fn recipe_sort_mode(&self) -> LuaResult<Rc<str>> {
        Ok(self.mod_data_string("RecipeSort")?.unwrap_or_else(|| Rc::from("RecipeName")))
    }

    /// `setSortModeInternal`.
    fn set_sort_mode_internal(&mut self, mode: &str) {
        self.compare_mode = match mode {
            "LastUsed" => CompareMode::LastUsed,
            "MostUsed" => CompareMode::MostUsed,
            _ => CompareMode::Name,
        };
    }

    /// `getLastManualInputMode(panel)`: the mod data value, if a string,
    /// equals "true".
    fn last_manual_input_mode(&self) -> bool {
        matches!(
            self.mod_data().get_own(&Value::str(&format!("{PANEL}ManualInputs"))),
            Value::Str(s) if &*s == "true"
        )
    }

    fn cached_info(&self, recipe: &ScriptRef) -> Option<Info> {
        self.cached_infos.iter().find(|(r, _)| r.ptr_eq(recipe)).map(|(_, i)| *i)
    }

    /// `player.isBuildCheat()` on the logic's `IsoPlayer`.
    fn build_cheat(&self) -> bool {
        let ctx = crate::bind_character_sim::current_sim_context();
        self.character.borrow().cheats.is_set(&*ctx, "BUILD")
    }
}

/// `triggerEvent(event, args...)`: each listener is called through
/// `LuaCaller.protectedCallVoid`, with its target first when it has one;
/// errors are logged. An unknown event logs a warning.
fn trigger_event(thread: &mut Thread, o: &Rc<dyn LuaObject>, event: &str, args: &[Value]) {
    let calls: Option<Vec<(Value, Vec<Value>)>> = {
        let st = state(o).borrow();
        st.events.iter().find(|(name, _)| *name == event).map(|(_, handlers)| {
            handlers
                .iter()
                .map(|h| {
                    let mut call_args = Vec::with_capacity(args.len() + 1);
                    if !h.target.is_nil() {
                        call_args.push(h.target.clone());
                    }
                    call_args.extend(args.iter().cloned());
                    (h.function.clone(), call_args)
                })
                .collect()
        })
    };
    let Some(calls) = calls else {
        (thread.log)(&format!("Event '{event}' is unknown."));
        return;
    };
    for (function, args) in calls {
        crate::global_objects::pcall_logged(thread, &function, &args);
    }
}

/// `setManualSelectInputs(b)`.
fn set_manual_select_inputs(thread: &mut Thread, o: &Rc<dyn LuaObject>, value: bool) {
    {
        let mut st = state(o).borrow_mut();
        if st.manual_select_inputs == value {
            return;
        }
        st.manual_select_inputs = value;
    }
    trigger_event(thread, o, "onManualSelectChanged", &[Value::Bool(value)]);
}

/// `HandcraftLogic.rebuildCachedRecipeInfo()`, when dirty: a
/// `createCachedRecipeInfo` for each recipe of the complete list.
fn rebuild_cached_recipe_info(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<()> {
    let (recipes, player, items, iso_object) = {
        let st = state(o).borrow();
        if !st.cached_infos_dirty {
            return Ok(());
        }
        (st.complete_recipe_list.clone(), st.player.clone(), st.all_items.clone(), st.iso_object.clone())
    };
    state(o).borrow_mut().cached_infos.clear();
    let obj = object(o);
    let mut infos = Vec::with_capacity(recipes.len());
    for script in recipes {
        let info = create_cached_recipe_info(thread, obj, &script, &player, &items, &iso_object)?;
        infos.push((script, info));
    }
    let mut st = state(o).borrow_mut();
    st.cached_infos = infos;
    st.cached_infos_dirty = false;
    Ok(())
}

/// `HandcraftLogic.createCachedRecipeInfo(recipe, containers)`.
fn create_cached_recipe_info(
    thread: &mut Thread,
    obj: &HandcraftLogicObject,
    script: &ScriptRef,
    player: &Value,
    items: &[ItemRef],
    iso_object: &Value,
) -> LuaResult<Info> {
    let recipe = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
    let recipe = recipe.borrow();
    let (valid, mut can_perform) = cached_recipe_info(thread, &obj.sm, &obj.chars, player, &recipe, items)?;
    // `HandcraftLogic`: an `AnySurfaceCraft` recipe also needs the craft
    // surface in range. That only reads state, so it is tested only when it
    // can change the result.
    if can_perform && recipe.is_any_surface_craft() {
        // `isCharacterInRangeOfWorkbench()`.
        can_perform = !iso_object.is_nil()
            && return_unsupported("HandcraftLogic.isCharacterInRangeOfWorkbench (IsoGridSquare.DistToProper)")?;
    }
    Ok(Info { valid, can_perform, available: valid && can_perform })
}

fn return_unsupported(what: &str) -> LuaResult<bool> {
    Err(unsupported(what))
}

/// `getCachedRecipeInfo(recipe)`.
fn get_cached_recipe_info(thread: &mut Thread, o: &Rc<dyn LuaObject>, script: &ScriptRef) -> LuaResult<Info> {
    rebuild_cached_recipe_info(thread, o)?;
    if let Some(info) = state(o).borrow().cached_info(script) {
        return Ok(info);
    }
    // `createCachedRecipeInfo(recipe, null)` also adds it to the cache.
    let (player, items, iso_object) = {
        let st = state(o).borrow();
        (st.player.clone(), st.all_items.clone(), st.iso_object.clone())
    };
    let info = create_cached_recipe_info(thread, object(o), script, &player, &items, &iso_object)?;
    state(o).borrow_mut().cached_infos.push((script.clone(), info));
    Ok(info)
}

/// `CachedRecipeComparator.compare(a, b)` with the cached infos.
fn compare_recipes(st: &State, cheat: bool, a: &ScriptRef, b: &ScriptRef) -> LuaResult<Ordering> {
    let (ia, ib) = (st.cached_info(a).ok_or_else(npe)?, st.cached_info(b).ok_or_else(npe)?);
    let (va, vb) = (ia.valid || cheat, ib.valid || cheat);
    if va != vb {
        return Ok(if va { Ordering::Less } else { Ordering::Greater });
    }
    let (ca, cb) = (ia.can_perform || cheat, ib.can_perform || cheat);
    if ca != cb {
        return Ok(if ca { Ordering::Less } else { Ordering::Greater });
    }
    let name = |s: &ScriptRef| -> LuaResult<String> {
        let r = s.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
        let r = r.borrow();
        r.translation_name().map(str::to_string).ok_or_else(npe)
    };
    let by_name = java_compare(&name(a)?, &name(b)?);
    match st.compare_mode {
        CompareMode::Name => Ok(by_name),
        // `IsoPlayer.getPlayerCraftHistory()`.
        CompareMode::LastUsed | CompareMode::MostUsed => {
            Err(unsupported("BaseCraftingLogic.CachedRecipeComparator (PlayerCraftHistory)"))
        }
    }
}

/// `List.sort` with a comparator that can fail: a stable sort, failing with
/// the first error.
fn try_sort<T>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> LuaResult<Ordering>) -> LuaResult<()> {
    let mut err = None;
    v.sort_by(|a, b| {
        if err.is_some() {
            return Ordering::Equal;
        }
        cmp(a, b).unwrap_or_else(|e| {
            err = Some(e);
            Ordering::Equal
        })
    });
    err.map_or(Ok(()), Err)
}

/// `CraftRecipeListNodeCollection.sort(comparator)`: a
/// `RecipeNodeComparator` over the top nodes; a group compares by its first
/// recipe after sorting its children.
fn sort_nodes(st: &State, cheat: bool, collection: &NodeCollection) -> LuaResult<()> {
    let first = |n: &Rc<Node>| -> LuaResult<Option<ScriptRef>> {
        if n.is_recipe() {
            return Ok(n.recipe.clone());
        }
        let mut children = n.children.borrow_mut();
        if children.is_empty() {
            return Ok(None);
        }
        try_sort(&mut children, |x, y| compare_nodes(st, cheat, x, y))?;
        Ok(children[0].recipe.clone())
    };
    let mut nodes = collection.nodes.borrow_mut();
    try_sort(&mut nodes, |a, b| {
        let (ra, rb) = (first(a)?, first(b)?);
        match (ra, rb) {
            (Some(ra), Some(rb)) => compare_recipes(st, cheat, &ra, &rb),
            // `getCachedRecipeInfo(null)`: `Objects.requireNonNull`.
            _ => Err(npe()),
        }
    })
}

fn compare_nodes(st: &State, cheat: bool, a: &Rc<Node>, b: &Rc<Node>) -> LuaResult<Ordering> {
    match (&a.recipe, &b.recipe) {
        (Some(ra), Some(rb)) => compare_recipes(st, cheat, ra, rb),
        _ => Err(npe()),
    }
}

/// `filterRecipeList(filter, category, force, player)` where `player` is
/// `getSpecificPlayer(0)`.
fn filter_recipe_list(
    thread: &mut Thread,
    o: &Rc<dyn LuaObject>,
    filter: Option<Rc<str>>,
    category: Option<Rc<str>>,
    force: bool,
) -> LuaResult<()> {
    let player0 = crate::bind_players::player(0);
    let known: Vec<ScriptRef> = {
        let st = state(o).borrow();
        match &player0 {
            None => st.complete_recipe_list.clone(),
            Some(p) => {
                let mut out = Vec::new();
                for script in &st.complete_recipe_list {
                    let recipe = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
                    if recipe.borrow().need_to_be_learn() && !has_player_learned(thread, &recipe.borrow(), p)? {
                        continue;
                    }
                    out.push(script.clone());
                }
                out
            }
        }
    };
    let changed = {
        let st = state(o).borrow();
        st.category_filter_string != category || st.filter_string != filter
    };
    if !(changed || force) {
        return Ok(());
    }
    {
        let mut st = state(o).borrow_mut();
        st.filter_string = filter.clone();
        st.category_filter_string = category.clone();
    }
    rebuild_cached_recipe_info(thread, o)?;
    let filtered = state(o).borrow().filtered.clone();
    filter_and_sort_recipe_list(o, filter.as_deref(), category.as_deref(), &filtered, &known, player0.as_ref())?;
    // `setInitialExpandedStates(this, player.isBuildCheat())`.
    let Some(player0) = player0 else {
        return Err(npe());
    };
    let ctx = crate::bind_character_sim::current_sim_context();
    let cheat0 = player0.borrow().cheats.is_set(&*ctx, "BUILD");
    set_initial_expanded_states(&state(o).borrow(), cheat0, None, &filtered.nodes.borrow());
    let value = Value::Object(Rc::new(NodeCollectionObject(filtered)));
    trigger_event(thread, o, "onUpdateRecipeList", &[value]);
    Ok(())
}

/// `CraftRecipe.hasPlayerLearned(chr)` for a recipe that needs to be
/// learned: `chr.isRecipeKnown(recipe)`.
fn has_player_learned(thread: &mut Thread, recipe: &CraftRecipe, chr: &LuaGameCharacterRef) -> LuaResult<bool> {
    // `isRecipeKnown(recipe, false)`: `SandboxOptions.seeNotLearntRecipe`
    // first.
    crate::bind_sandbox::instance(thread)?;
    let see_all = crate::bind_sandbox::current_bool("SeeNotLearntRecipe")
        .ok_or_else(|| unsupported("SandboxOptions.seeNotLearntRecipe"))?;
    let ctx = crate::bind_character_sim::current_sim_context();
    let chr = chr.borrow();
    if see_all || chr.cheats.is_set(&*ctx, "KNOW_ALL_RECIPES") {
        return Ok(true);
    }
    let known = |name: Option<&str>| name.is_some_and(|n| chr.known_recipes.iter().any(|k| k == n));
    Ok(known(recipe.name()) || known(recipe.meta_recipe()) || known(recipe.translation_name()))
}

/// `BaseCraftingLogic.filterAndSortRecipeList` without a filter string:
/// the known recipes (for a category, those of the category; for "*", the
/// player's favourites), sorted.
fn filter_and_sort_recipe_list(
    o: &Rc<dyn LuaObject>,
    filter: Option<&str>,
    category: Option<&str>,
    out: &NodeCollection,
    known: &[ScriptRef],
    _player: Option<&LuaGameCharacterRef>,
) -> LuaResult<()> {
    out.clear();
    let list: Vec<ScriptRef> = if null_or_whitespace(category) {
        known.to_vec()
    } else if category == Some("*") {
        return Err(unsupported("IsoPlayer.isFavouriteRecipe"));
    } else {
        let category = category.unwrap_or_default();
        known
            .iter()
            .filter(|s| {
                s.downcast::<CraftRecipe>()
                    .is_some_and(|r| java_compare_ignore_case(r.borrow().category(), category) == Ordering::Equal)
            })
            .cloned()
            .collect()
    };
    if !null_or_whitespace(filter) {
        return Err(unsupported("BaseCraftingLogic.filterAndSortRecipeList with a filter string"));
    }
    for script in &list {
        let recipe = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
        out.add(script, &recipe.borrow());
    }
    let st = state(o).borrow();
    let cheat = st.build_cheat();
    sort_nodes(&st, cheat, out)
}

/// `CraftRecipeListNodeCollection.setInitialExpandedStates(logic, cheat,
/// parent, nodes)`.
fn set_initial_expanded_states(st: &State, cheat: bool, parent: Option<&Rc<Node>>, nodes: &[Rc<Node>]) {
    for n in nodes {
        if let (true, Some(recipe)) = (n.is_recipe(), &n.recipe) {
            if let Some(info) = st.cached_info(recipe) {
                let open = cheat || (info.valid && info.can_perform);
                n.expanded.set(if open { Expanded::Open } else { Expanded::Closed });
            }
        }
        if !n.is_recipe() {
            set_initial_expanded_states(st, cheat, Some(n), &n.children.borrow());
        }
    }
    if let Some(parent) = parent {
        if nodes.iter().all(|n| n.expanded.get() == Expanded::Open) {
            parent.expanded.set(Expanded::Open);
        } else if nodes.iter().all(|n| n.expanded.get() == Expanded::Closed) {
            parent.expanded.set(Expanded::Closed);
        }
    }
}

/// `new HandcraftLogic(player, craftBench, isoObject)`.
fn construct(c: &mut NativeCall<'_>, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) -> LuaResult<usize> {
    let player = c.arg(0);
    if !c.arg(1).is_nil() {
        // `sourceResources.addAll(craftBench.getResources())` and the bench's
        // `CraftRecipeData`.
        return Err(unsupported("HandcraftLogic with a CraftBench"));
    }
    let character = match &player {
        // `getRecipeSortMode()` reads `player.getModData()`.
        Value::Nil => return Err(npe()),
        Value::Object(o) => match o.as_any().downcast_ref::<CharacterObject>() {
            Some(chr) => chr.0.clone(),
            None => return Err(class_cast()),
        },
        _ => return Err(class_cast()),
    };
    let mut st = State {
        player: player.clone(),
        character,
        iso_object: c.arg(2),
        events: EVENTS.iter().map(|&name| (name, Vec::new())).collect(),
        manual_select_inputs: false,
        show_manual_select_inputs: false,
        manual_input_allowed_item_types: Vec::new(),
        multicraft_consumed_items: Vec::new(),
        manual_select_input_script_filter: Value::Nil,
        compare_mode: CompareMode::Name,
        containers: Vec::new(),
        all_items: Vec::new(),
        complete_recipe_list: Vec::new(),
        filter_string: None,
        category_filter_string: None,
        filtered: Rc::new(NodeCollection::default()),
        cached_infos: Vec::new(),
        cached_infos_dirty: false,
        recipe: None,
        data: DataState::empty(Vec::new()),
        held: Vec::new(),
        cached_possible_craft_count: 0,
        cached_can_perform: false,
        cached_can_perform_dirty: true,
        craft_action_in_progress: false,
        craft_action_table: Value::Nil,
        craft_action_count: 0,
    };
    let mode = st.recipe_sort_mode()?;
    st.set_sort_mode_internal(&mode);
    let manual = st.last_manual_input_mode();
    let o: Rc<dyn LuaObject> =
        Rc::new(HandcraftLogicObject { state: RefCell::new(st), sm: sm.clone(), chars: chars.clone() });
    // No listener can exist yet, so the event reaches nobody.
    set_manual_select_inputs(c.thread, &o, manual);
    Ok(c.push(Value::Object(o)))
}

/// `HandcraftLogic.setRecipe`: a changed recipe is saved as the last
/// selected one, then `BaseCraftingLogic.setRecipe`.
fn set_recipe(thread: &mut Thread, o: &Rc<dyn LuaObject>, recipe: Option<ScriptRef>) -> LuaResult<()> {
    {
        let st = state(o).borrow();
        let same = match (&st.recipe, &recipe) {
            (None, None) => true,
            (Some(a), Some(b)) => a.ptr_eq(b),
            _ => false,
        };
        if same {
            return Ok(());
        }
        // `setLastSelectedRecipe("handcraft", recipe)`.
        let full_type = match &recipe {
            Some(r) => Value::str(&r.full_type().map_err(|e| LuaError::runtime(e.0))?),
            None => Value::Nil,
        };
        st.set_mod_data("LastRecipe", full_type);
    }
    recipe::base_set_recipe(thread, o, recipe)
}

/// Methods that need the parts of the crafting logic not ported yet.
const NOT_PORTED: &[&str] = &[
    "getSourceResources",
    "getResidualFluidFromInput",
    "getInputItemNodes",
    "getInputItemNodesForInput",
    "getSatisfiedInputItems",
    "getSatisfiedInputFluids",
    "offerInputItem",
    "removeInputItem",
    "clearManualInputsFor",
    "getResultTexture",
];

pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    expose_enum(thread, NODE_TYPE_CLASS, &["GROUP", "RECIPE"]);
    expose_enum(thread, EXPANDED_STATE_CLASS, &["CLOSED", "PARTIAL", "OPEN"]);
    register_nodes(thread);
    register_info(thread);
    recipe::register_recipe_data(thread);

    let base = expose_class(thread, BASE_CRAFTING_LOGIC_CLASS, None);
    // `getFavouriteModDataString(recipe)`: "recipeFavourite:" and the
    // recipe's full type.
    base.static_fn("getFavouriteModDataString", |c| {
        let s = match c.arg(0) {
            Value::Str(s) => s.to_string(),
            v => {
                let recipe = recipe_arg(&v)?.ok_or_else(npe)?;
                recipe.full_type().map_err(|e| LuaError::runtime(e.0))?
            }
        };
        Ok(c.push(Value::str(&format!("recipeFavourite:{s}"))))
    });
    let b = expose_class(thread, HANDCRAFT_LOGIC_CLASS, Some(BASE_CRAFTING_LOGIC_CLASS));
    b.ctor(move |c| construct(c, &sm, &chars));

    for name in NOT_PORTED {
        b.method(name, move |_| Err(unsupported(format!("HandcraftLogic.{name}"))));
    }
    recipe::register(&b);

    b.method("getPlayer", |c| {
        let v = state(&logic(c)?).borrow().player.clone();
        Ok(c.push(v))
    });
    // `craftBench` is always null here (see `construct`).
    b.method("getCraftBench", |c| {
        logic(c)?;
        Ok(c.push(Value::Nil))
    });
    b.method("getIsoObject", |c| {
        let v = state(&logic(c)?).borrow().iso_object.clone();
        Ok(c.push(v))
    });
    b.method("setIsoObject", |c| {
        let v = c.arg(1);
        state(&logic(c)?).borrow_mut().iso_object = v;
        Ok(0)
    });
    b.method("getRecipe", |c| {
        let v = state(&logic(c)?).borrow().recipe.clone();
        Ok(c.push(script_value(v.as_ref())))
    });
    b.method("setRecipe", |c| {
        let o = logic(c)?;
        let recipe = recipe_arg(&c.arg(1))?;
        set_recipe(c.thread, &o, recipe)?;
        Ok(0)
    });
    // `checkValidRecipeSelected()`: keep the recipe if the filtered list
    // contains it, else select the first recipe of the list, or none.
    b.method("checkValidRecipeSelected", |c| {
        let o = logic(c)?;
        let (contains, first) = {
            let st = state(&o).borrow();
            let recipes = st.filtered.all_recipes();
            let contains = st.recipe.as_ref().is_some_and(|r| recipes.iter().any(|x| x.ptr_eq(r)));
            (contains, recipes.into_iter().next())
        };
        if !contains {
            set_recipe(c.thread, &o, first)?;
        }
        Ok(0)
    });

    b.method("getContainers", |c| {
        let v = state(&logic(c)?).borrow().containers.clone();
        Ok(c.push(Value::Object(Rc::new(JavaList::new(v)))))
    });
    b.method("getAllItems", |c| {
        let items = state(&logic(c)?).borrow().all_items.clone();
        let values = items.iter().map(crate::bind_items::try_item_value).collect::<LuaResult<Vec<_>>>()?;
        Ok(c.push(Value::Object(Rc::new(JavaList::new(values)))))
    });
    b.method("setContainers", |c| {
        let o = logic(c)?;
        let list = c.arg(1);
        let items = container_items(&list)?;
        let containers = match &list {
            Value::Object(l) => l.as_any().downcast_ref::<JavaList>().map(JavaList::snapshot).unwrap_or_default(),
            _ => Vec::new(),
        };
        let changed = {
            let mut st = state(&o).borrow_mut();
            let changed = items.len() != st.all_items.len()
                || items.iter().any(|i| !st.all_items.iter().any(|old| Rc::ptr_eq(old, i)));
            st.containers = containers;
            st.all_items = items;
            changed
        };
        if changed {
            if state(&o).borrow().recipe.is_some() {
                recipe::set_containers_with_recipe(c.thread, &o)?;
            }
            // `inputItemNodeCollection.setItems(allItems)`: the nodes are
            // only read with manual input selection.
            {
                let mut st = state(&o).borrow_mut();
                st.cached_infos_dirty = true;
                st.cached_can_perform_dirty = true;
            }
            trigger_event(c.thread, &o, "onUpdateContainers", &[]);
            trigger_event(c.thread, &o, "onRebuildInputItemNodes", &[]);
        }
        Ok(c.push(changed))
    });

    b.method("getRecipeList", |c| {
        let v = state(&logic(c)?).borrow().filtered.clone();
        Ok(c.push(Value::Object(Rc::new(NodeCollectionObject(v)))))
    });
    b.method("setRecipes", |c| {
        let o = logic(c)?;
        let list = match c.arg(1) {
            Value::Object(l) => l.as_any().downcast_ref::<JavaList>().map(JavaList::snapshot).ok_or_else(class_cast)?,
            Value::Nil => return Err(npe()),
            _ => return Err(class_cast()),
        };
        let recipes = list.iter().map(|v| recipe_arg(v)?.ok_or_else(npe)).collect::<LuaResult<Vec<_>>>()?;
        let (filter, category) = {
            let mut st = state(&o).borrow_mut();
            st.complete_recipe_list = recipes;
            st.filtered.clear();
            st.cached_infos_dirty = true;
            (st.filter_string.clone(), st.category_filter_string.clone())
        };
        filter_recipe_list(c.thread, &o, filter, category, true)?;
        let filtered = state(&o).borrow().filtered.clone();
        trigger_event(c.thread, &o, "onSetRecipeList", &[Value::Object(Rc::new(NodeCollectionObject(filtered)))]);
        Ok(0)
    });
    b.method("filterRecipeList", |c| {
        let o = logic(c)?;
        let (filter, category) = (string_arg(&c.arg(1))?, string_arg(&c.arg(2))?);
        let force = matches!(c.arg(3), Value::Bool(true));
        filter_recipe_list(c.thread, &o, filter, category, force)?;
        Ok(0)
    });
    b.method("sortRecipeList", |c| {
        let o = logic(c)?;
        let (filter, category) = {
            let st = state(&o).borrow();
            (st.filter_string.clone(), st.category_filter_string.clone())
        };
        filter_recipe_list(c.thread, &o, filter, category, true)?;
        Ok(0)
    });
    // `getCategoryList()`: the complete list's categories, each once,
    // sorted with `String.compareToIgnoreCase`.
    b.method("getCategoryList", |c| {
        let o = logic(c)?;
        let mut categories: Vec<String> = Vec::new();
        for script in &state(&o).borrow().complete_recipe_list {
            let recipe = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
            let category = recipe.borrow().category().to_string();
            if !categories.contains(&category) {
                categories.push(category);
            }
        }
        categories.sort_by(|a, b| java_compare_ignore_case(a, b));
        Ok(c.push(crate::bind_scripts::string_list(categories)))
    });
    b.method("getCachedRecipeInfo", |c| {
        let o = logic(c)?;
        let recipe = recipe_arg(&c.arg(1))?.ok_or_else(npe)?;
        let info = get_cached_recipe_info(c.thread, &o, &recipe)?;
        Ok(c.push(Value::Object(Rc::new(InfoObject { recipe, info }))))
    });

    b.method("addEventListener", |c| {
        let o = logic(c)?;
        let Value::Str(event) = c.arg(1) else {
            return Err(class_cast());
        };
        let handler = Handler { function: c.arg(2), target: c.arg(3) };
        let count = {
            let mut st = state(&o).borrow_mut();
            match st.events.iter_mut().find(|(name, _)| **name == *event) {
                Some((_, handlers)) => {
                    handlers.push(handler);
                    Some(handlers.len())
                }
                None => None,
            }
        };
        match count {
            Some(n) if n > 10 && crate::bind_core::shared_core().borrow().debug => Err(LuaError::Runtime {
                message: format!("Sanity check, event '{event}' has >10 listeners"),
                class: "java.lang.RuntimeException",
            }),
            Some(_) => Ok(0),
            None => {
                (c.thread.log)(&format!("Event '{event}' is unknown."));
                Ok(0)
            }
        }
    });

    b.method("isManualSelectInputs", |c| {
        let v = state(&logic(c)?).borrow().manual_select_inputs;
        Ok(c.push(v))
    });
    b.method("setManualSelectInputs", |c| {
        let o = logic(c)?;
        let v = matches!(c.arg(1), Value::Bool(true));
        set_manual_select_inputs(c.thread, &o, v);
        Ok(0)
    });
    b.method("shouldShowManualSelectInputs", |c| {
        let o = logic(c)?;
        let v = {
            let st = state(&o).borrow();
            st.manual_select_inputs && st.show_manual_select_inputs
        };
        Ok(c.push(v))
    });
    b.method("setShowManualSelectInputs", |c| {
        let o = logic(c)?;
        let v = matches!(c.arg(1), Value::Bool(true));
        let show = {
            let mut st = state(&o).borrow_mut();
            if st.show_manual_select_inputs == v {
                return Ok(0);
            }
            st.show_manual_select_inputs = v;
            st.manual_select_inputs && st.show_manual_select_inputs
        };
        trigger_event(c.thread, &o, "onShowManualSelectChanged", &[Value::Bool(show)]);
        Ok(0)
    });
    b.method("getManualSelectInputScriptFilter", |c| {
        let v = state(&logic(c)?).borrow().manual_select_input_script_filter.clone();
        Ok(c.push(v))
    });
    b.method("setManualSelectInputScriptFilter", |c| {
        let o = logic(c)?;
        state(&o).borrow_mut().manual_select_input_script_filter = c.arg(1);
        trigger_event(c.thread, &o, "onRebuildInputItemNodes", &[]);
        Ok(0)
    });

    b.method("getSelectedRecipeStyle", |c| {
        let v = state(&logic(c)?).borrow().mod_data_string("RecipeView")?;
        Ok(c.push(v.map_or(Value::Nil, Value::Str)))
    });
    b.method("setSelectedRecipeStyle", |c| {
        let v = c.arg(1);
        state(&logic(c)?).borrow().set_mod_data("RecipeView", v);
        Ok(0)
    });
    b.method("getRecipeSortMode", |c| {
        let v = state(&logic(c)?).borrow().recipe_sort_mode()?;
        Ok(c.push(Value::Str(v)))
    });
    b.method("setRecipeSortMode", |c| {
        let v = c.arg(1);
        let o = logic(c)?;
        let mut st = state(&o).borrow_mut();
        st.set_mod_data("RecipeSort", v.clone());
        // `setSortModeInternal(mode)` switches on `mode.hashCode()`: null
        // throws.
        let Value::Str(mode) = v else {
            return Err(npe());
        };
        st.set_sort_mode_internal(&mode);
        Ok(0)
    });
    b.method("setLastManualInputMode", |c| {
        let v = if matches!(c.arg(1), Value::Bool(true)) { "true" } else { "false" };
        state(&logic(c)?).borrow().set_mod_data("ManualInputs", Value::str(v));
        Ok(0)
    });
}

fn register_nodes(thread: &mut Thread) {
    let b = expose_class(thread, NODE_COLLECTION_CLASS, None);
    let coll = |c: &NativeCall<'_>| -> LuaResult<Rc<NodeCollection>> {
        Ok(this::<NodeCollectionObject>(c)?.as_any().downcast_ref::<NodeCollectionObject>().unwrap().0.clone())
    };
    b.method("getNodes", move |c| {
        let nodes = coll(c)?.nodes.borrow().clone();
        let values = nodes.into_iter().map(|n| Value::Object(Rc::new(NodeObject(n)))).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::new(values)))))
    });
    b.method("getAllRecipes", move |c| {
        let recipes = coll(c)?.all_recipes();
        Ok(c.push(script_list(&recipes)))
    });
    b.method("isEmpty", move |c| {
        let v = coll(c)?.nodes.borrow().is_empty();
        Ok(c.push(v))
    });
    b.method("getFirstRecipe", move |c| {
        let first = coll(c)?.all_recipes().into_iter().next();
        Ok(c.push(script_value(first.as_ref())))
    });
    b.method("contains", move |c| {
        let recipe = recipe_arg(&c.arg(1))?;
        let v = recipe.is_some_and(|r| coll(c).is_ok_and(|n| n.all_recipes().iter().any(|x| x.ptr_eq(&r))));
        Ok(c.push(v))
    });

    let n = expose_class(thread, NODE_CLASS, None);
    let node = |c: &NativeCall<'_>| -> LuaResult<Rc<Node>> {
        Ok(this::<NodeObject>(c)?.as_any().downcast_ref::<NodeObject>().unwrap().0.clone())
    };
    n.method("getType", move |c| {
        let v = if node(c)?.is_recipe() { "RECIPE" } else { "GROUP" };
        Ok(c.push(enum_value(NODE_TYPE_CLASS, v)))
    });
    n.method("getRecipe", move |c| {
        let v = node(c)?.recipe.clone();
        Ok(c.push(script_value(v.as_ref())))
    });
    n.method("getTitle", move |c| {
        let v = node(c)?.title.clone();
        Ok(c.push(v.map_or(Value::Nil, |t| Value::str(&t))))
    });
    n.method("getParent", move |c| {
        let v = node(c)?.parent.as_ref().and_then(Weak::upgrade);
        Ok(c.push(v.map_or(Value::Nil, |p| Value::Object(Rc::new(NodeObject(p))))))
    });
    n.method("getChildren", move |c| {
        let children = node(c)?.children.borrow().clone();
        let values = children.into_iter().map(|n| Value::Object(Rc::new(NodeObject(n)))).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::new(values)))))
    });
    n.method("getExpandedState", move |c| {
        let v = node(c)?.expanded.get();
        Ok(c.push(enum_value(EXPANDED_STATE_CLASS, v.name())))
    });
    n.method("setExpandedState", move |c| {
        let n = node(c)?;
        let state = match c.arg(1) {
            Value::Object(o) => match o.as_any().downcast_ref::<crate::expose::JavaEnum>() {
                Some(e) if e.class == EXPANDED_STATE_CLASS => match e.name {
                    "CLOSED" => Expanded::Closed,
                    "PARTIAL" => Expanded::Partial,
                    _ => Expanded::Open,
                },
                _ => return Err(class_cast()),
            },
            _ => return Err(class_cast()),
        };
        n.expanded.set(state);
        Ok(0)
    });
    // `getIconTexture()`: what `createRecipeNode` takes from the recipe
    // (`CraftRecipe.getIconTexture()`), or what `createGroupNode` takes
    // from the group (`CraftRecipeGroup.getIconTexture()`, which is
    // `Texture.getTexture(iconPath)`).
    n.method("getIconTexture", move |c| {
        let n = node(c)?;
        let v = if let Some(recipe) = &n.recipe {
            let recipe = recipe.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
            let v = crate::bind_scripts::recipe_icon_texture(&recipe.borrow())?;
            v
        } else if let Some(group) = n.group {
            let texture = pz_render::try_get_texture_ref(group.icon_path()).map_err(crate::bind_render::texture_error)?;
            crate::bind_render::texture_value_ref(texture.as_ref())
        } else {
            Value::Nil
        };
        Ok(c.push(v))
    });
    n.method("getGroup", |_| Err(unsupported("CraftRecipeListNode.getGroup (CraftRecipeGroup)")));
    n.method("toggleExpandedState", |_| Err(unsupported("CraftRecipeListNode.toggleExpandedState")));
}

fn register_info(thread: &mut Thread) {
    let b = expose_class(thread, CACHED_RECIPE_INFO_CLASS, None);
    let info = |c: &NativeCall<'_>| -> LuaResult<Info> {
        Ok(this::<InfoObject>(c)?.as_any().downcast_ref::<InfoObject>().unwrap().info)
    };
    b.method("isValid", move |c| {
        let v = info(c)?.valid;
        Ok(c.push(v))
    });
    b.method("isCanPerform", move |c| {
        let v = info(c)?.can_perform;
        Ok(c.push(v))
    });
    b.method("isAvailable", move |c| {
        let v = info(c)?.available;
        Ok(c.push(v))
    });
    b.method("getRecipe", |c| {
        let o = this::<InfoObject>(c)?;
        let r = o.as_any().downcast_ref::<InfoObject>().unwrap().recipe.clone();
        Ok(c.push(script_value(Some(&r))))
    });
}

#[cfg(test)]
mod tests;
