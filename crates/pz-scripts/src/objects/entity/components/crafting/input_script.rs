//! `zombie.scripting.entity.components.crafting.InputScript`: one input
//! line of a craft recipe (`item 2 [Base.A;Base.B] mode:keep flags[...]`).

use std::collections::BTreeSet;
use std::rc::Rc;

use super::{CraftRecipe, FluidMatchMode, InputFlag, ItemApplyMode};
use crate::base::{ScriptRef, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::components::resources::ResourceType;
use crate::objects::entity::{
    array_get, bracket_contents, exception, index_of, log_error, parse_float, parse_int, substring_from,
};
use crate::registry::RegistryKey;
use crate::script_type::ScriptType;

#[derive(Clone, Debug)]
pub struct InputScript {
    pub(crate) loaded_items: Vec<String>,
    pub(crate) loaded_fluids: Vec<String>,
    pub(crate) loaded_energies: Vec<String>,
    pub(crate) items: Vec<String>,
    /// `Set<ItemTag>`; `None` is a tag the registry does not know (Java's
    /// `null`). Kept in insertion order.
    pub(crate) item_tags: Vec<Option<Rc<RegistryKey>>>,
    pub(crate) categories: Vec<String>,
    pub(crate) accepts_any_item: bool,
    pub(crate) accepts_any_fluid: bool,
    pub(crate) accepts_any_energy: bool,
    pub(crate) type_: ResourceType,
    pub(crate) item_script_cache: Vec<ScriptRef>,
    pub(crate) item_apply_mode: ItemApplyMode,
    pub(crate) fluid_match_mode: FluidMatchMode,
    pub(crate) amount: f32,
    pub(crate) maxamount: f32,
    pub(crate) amounts: Vec<f32>,
    pub(crate) maxamounts: Vec<f32>,
    pub(crate) apply_on_tick: bool,
    pub(crate) shaped_index: i32,
    pub(crate) original_line: String,
    /// Index into the recipe's `ioLines` of the `+` output line.
    pub(crate) create_to_item_script: Option<usize>,
    /// Index into the recipe's `ioLines` of the `-` input line.
    pub(crate) consume_from_item_script: Option<usize>,
    /// For a `-` line: index of the input it belongs to.
    pub(crate) parent_script: Option<usize>,
    pub(crate) flags: BTreeSet<InputFlag>,
    /// `energies`: resolved energy ids. TODO(Energy): `Energy.Get`.
    pub(crate) energies: Vec<String>,
    /// Whether `fluidFilter` was sealed (in `OnPostWorldDictionaryInit`).
    /// TODO(Fluid): the filter itself and `filteredFluidCache`.
    pub(crate) fluid_filter_sealed: bool,
}

impl InputScript {
    fn new(type_: ResourceType) -> Self {
        InputScript {
            loaded_items: Vec::new(),
            loaded_fluids: Vec::new(),
            loaded_energies: Vec::new(),
            items: Vec::new(),
            item_tags: Vec::new(),
            categories: Vec::new(),
            accepts_any_item: false,
            accepts_any_fluid: false,
            accepts_any_energy: false,
            type_,
            item_script_cache: Vec::new(),
            item_apply_mode: ItemApplyMode::Normal,
            fluid_match_mode: FluidMatchMode::Exact,
            amount: 0.0,
            maxamount: 0.0,
            amounts: Vec::new(),
            maxamounts: Vec::new(),
            apply_on_tick: false,
            shaped_index: -1,
            original_line: String::new(),
            create_to_item_script: None,
            consume_from_item_script: None,
            parent_script: None,
            flags: BTreeSet::new(),
            energies: Vec::new(),
            fluid_filter_sealed: false,
        }
    }

    /// `Load(parentRecipe, line, isInternal)`; `io_index` is the position
    /// the line will take in the recipe's `ioLines` (mappers record it).
    pub(crate) fn load(
        recipe: &mut CraftRecipe,
        sm: &ScriptManager,
        line: &str,
        is_internal: bool,
        io_index: usize,
    ) -> ScriptResult<Option<InputScript>> {
        if java::is_null_or_whitespace(Some(line)) {
            return Ok(None);
        }
        let elems: Vec<&str> = java::split_ws(java::trim(line)).into_iter().map(java::trim).collect();
        let recipe_name = recipe.base.full_type()?;
        let e0 = array_get(&elems, 0)?;
        let type_ = if java::eq_ignore_case(e0, "item") || (is_internal && java::eq_ignore_case(e0, "uses")) {
            ResourceType::Item
        } else if java::eq_ignore_case(e0, "fluid") {
            ResourceType::Fluid
        } else if java::eq_ignore_case(e0, "energy") {
            ResourceType::Energy
        } else {
            return Err(exception(format!("unknown type in craftrecipe: {e0}")));
        };
        let mut input = InputScript::new(type_);
        input.original_line = java::trim(line).to_string();
        let input_amount = array_get(&elems, 1)?;
        let (baseamount, upperamount) = parse_amounts(input_amount)?;
        input.amount = baseamount;
        input.maxamount = upperamount;
        if is_internal {
            // The original sets acceptsAnyItem for every resource type.
            input.accepts_any_item = true;
        }
        for &s in &elems[2..] {
            if s.starts_with('[') {
                for entry in java::split_char(bracket_contents(s)?, ';') {
                    match type_ {
                        ResourceType::Item => {
                            if entry == "*" {
                                input.accepts_any_item = true;
                            } else if entry.contains(':') {
                                let split2 = java::split_char(entry, ':');
                                input.loaded_items.push(array_get(&split2, 1)?.to_string());
                                let itembaseamount = pz_max0(parse_float(array_get(&split2, 0)?)?);
                                input.amounts.push(itembaseamount);
                                input.maxamounts.push(itembaseamount * (upperamount / baseamount));
                            } else {
                                input.loaded_items.push(entry.to_string());
                                input.amounts.push(baseamount);
                                input.maxamounts.push(upperamount);
                            }
                        }
                        ResourceType::Fluid => {
                            if entry == "*" {
                                input.accepts_any_fluid = true;
                            } else {
                                input.loaded_fluids.push(entry.to_string());
                                input.amounts.push(baseamount);
                                input.maxamounts.push(upperamount);
                            }
                        }
                        ResourceType::Energy => {
                            if entry == "*" {
                                input.accepts_any_energy = true;
                            } else {
                                input.loaded_energies.push(entry.to_string());
                                input.amounts.push(baseamount);
                                input.maxamounts.push(upperamount);
                            }
                        }
                        ResourceType::Any => {}
                    }
                }
            } else if s.starts_with("shapedIndex:") {
                input.shaped_index = parse_int(substring_from(s, index_of(s, ":") + 1)?)?;
            } else if s.starts_with("apply:") {
                return Err(exception("OnTick currently disabled for inputs."));
            } else if s.starts_with("mode:") {
                let m = substring_from(s, index_of(s, ":") + 1)?;
                input.load_mode(sm, m, &recipe_name)?;
            } else if s.starts_with("tags") {
                if type_ != ResourceType::Item {
                    return Err(exception(format!("cannot set tags on non-item: {s}")));
                }
                for entry in java::split_char(bracket_contents(s)?, ';') {
                    let registry = sm.registries.get("item_tag").expect("item_tag registry").borrow();
                    let tag = registry.get_id(entry)?;
                    if !input.item_tags.contains(&tag) {
                        input.item_tags.push(tag);
                    }
                }
            } else if s.starts_with("categories") {
                if type_ != ResourceType::Fluid {
                    return Err(exception(format!("cannot set category on non-fluid: {s}")));
                }
                input.categories.extend(java::split_char(bracket_contents(s)?, ';').iter().map(|c| c.to_string()));
            } else if s.starts_with("flags") {
                for entry in java::split_char(bracket_contents(s)?, ';') {
                    input.flags.insert(InputFlag::value_of(entry)?);
                }
            } else if s.starts_with("mappers") {
                for entry in java::split_char(bracket_contents(s)?, ';') {
                    recipe.get_or_create_output_mapper(entry).register_input_script(io_index);
                }
            } else if s.starts_with("overlayMapper") {
                recipe.overlay_mapper.register_input_script(io_index);
            } else {
                return Err(exception(format!("unknown recipe param: {s}")));
            }
        }
        if input.amounts.len() > 1 && !input.is_variable_amount() && input.loaded_fluids.is_empty() {
            let mut distinct: Vec<u32> = input.amounts.iter().map(|a| float_key(*a)).collect();
            distinct.sort_unstable();
            distinct.dedup();
            if distinct.len() > 1 {
                input.amount = 1.0;
                input.maxamount = 1.0;
            }
        }
        if input.accepts_any_item {
            input.loaded_items.clear();
            input.item_tags.clear();
        }
        if input.accepts_any_fluid {
            input.loaded_fluids.clear();
            input.categories.clear();
        }
        if input.accepts_any_energy {
            input.loaded_energies.clear();
        }
        Ok(Some(input))
    }

    /// The `mode:` parameter.
    fn load_mode(&mut self, sm: &ScriptManager, s: &str, recipe_name: &str) -> ScriptResult {
        let eq = |m: &str| java::eq_ignore_case(s, m);
        match self.type_ {
            ResourceType::Item => {
                if eq("use") {
                } else if eq("keep") {
                    self.item_apply_mode = ItemApplyMode::Keep;
                } else if eq("destroy") {
                    self.item_apply_mode = ItemApplyMode::Destroy;
                } else if ["useprop1", "useprop2", "keepprop1", "keepprop2", "prop1", "prop2"].iter().any(|m| eq(m)) {
                    log_error(sm, &format!("Deprecated parameter in inputscript! for recipe {recipe_name}"));
                } else {
                    return Err(exception(format!("Invalid item mode Error - {s}")));
                }
            }
            ResourceType::Fluid => {
                self.fluid_match_mode = if eq("exact") {
                    FluidMatchMode::Exact
                } else if eq("primary") {
                    FluidMatchMode::Primary
                } else if eq("mixture") {
                    FluidMatchMode::Mixture
                } else if eq("anything") {
                    FluidMatchMode::Anything
                } else {
                    return Err(exception("Invalid fluid mode Error"));
                };
            }
            ResourceType::Energy => {
                if !eq("keep") {
                    return Err(exception("Invalid energy mode Error"));
                }
                self.item_apply_mode = ItemApplyMode::Keep;
            }
            ResourceType::Any => {
                sm.warn(&format!("Cannot set mode for type = Any for recipe {recipe_name}"));
                if sm.debug {
                    return Err(exception("Mode Error"));
                }
            }
        }
        Ok(())
    }

    /// `isValid`.
    pub fn is_valid(&self) -> bool {
        match self.type_ {
            ResourceType::Item => self.accepts_any_item || !self.items.is_empty(),
            ResourceType::Fluid => self.accepts_any_fluid || self.fluid_filter_sealed,
            ResourceType::Energy => self.accepts_any_energy || !self.energies.is_empty(),
            ResourceType::Any => false,
        }
    }

    /// `getPossibleInputItems`.
    pub fn possible_input_items(&self) -> &[ScriptRef] {
        &self.item_script_cache
    }

    /// `hasCreateToItem`.
    pub fn has_create_to_item(&self) -> bool {
        self.create_to_item_script.is_some()
    }

    /// `getCreateToItemScript`, as an index into the recipe's `ioLines`.
    pub fn create_to_item_script(&self) -> Option<usize> {
        self.create_to_item_script
    }

    /// `hasConsumeFromItem`.
    pub fn has_consume_from_item(&self) -> bool {
        self.consume_from_item_script.is_some()
    }

    /// `getConsumeFromItemScript`, as an index into the recipe's `ioLines`.
    pub fn consume_from_item_script(&self) -> Option<usize> {
        self.consume_from_item_script
    }

    /// `hasParentScript`.
    pub fn has_parent_script(&self) -> bool {
        self.parent_script.is_some()
    }

    /// `getParentScript`, as an index into the recipe's `ioLines`.
    pub fn parent_script(&self) -> Option<usize> {
        self.parent_script
    }

    /// `hasFlag`.
    pub fn has_flag(&self, flag: InputFlag) -> bool {
        self.flags.contains(&flag)
    }

    pub fn flags(&self) -> &BTreeSet<InputFlag> {
        &self.flags
    }

    /// `getOriginalLine`.
    pub fn original_line(&self) -> &str {
        &self.original_line
    }

    /// `getResourceType`.
    pub fn resource_type(&self) -> ResourceType {
        self.type_
    }

    /// `isExclusive`.
    pub fn is_exclusive(&self) -> bool {
        self.has_flag(InputFlag::IsExclusive)
    }

    /// `isItemCount`.
    pub fn is_item_count(&self) -> bool {
        self.has_flag(InputFlag::ItemCount)
    }

    /// `isDestroy`.
    pub fn is_destroy(&self) -> bool {
        self.item_apply_mode == ItemApplyMode::Destroy
    }

    /// `isKeep`.
    pub fn is_keep(&self) -> bool {
        self.item_apply_mode == ItemApplyMode::Keep
    }

    /// `isTool`.
    pub fn is_tool(&self) -> bool {
        self.has_flag(InputFlag::ToolRight) || self.has_flag(InputFlag::ToolLeft)
    }

    /// `isProp1`.
    pub fn is_prop1(&self) -> bool {
        self.has_flag(InputFlag::Prop1)
    }

    /// `isProp2`.
    pub fn is_prop2(&self) -> bool {
        self.has_flag(InputFlag::Prop2)
    }

    /// `isHandcraftOnly`.
    pub fn is_handcraft_only(&self) -> bool {
        self.has_flag(InputFlag::HandcraftOnly)
    }

    /// `isAutomationOnly`.
    pub fn is_automation_only(&self) -> bool {
        self.has_flag(InputFlag::AutomationOnly)
    }

    /// `allowFavorites`.
    pub fn allow_favorites(&self) -> bool {
        self.is_keep() || self.has_flag(InputFlag::AllowFavorite)
    }

    /// `getShapedIndex` (deprecated).
    pub fn shaped_index(&self) -> i32 {
        self.shaped_index
    }

    /// `getItemApplyMode`.
    pub fn item_apply_mode(&self) -> ItemApplyMode {
        self.item_apply_mode
    }

    /// `getFluidMatchMode`.
    pub fn fluid_match_mode(&self) -> FluidMatchMode {
        self.fluid_match_mode
    }

    /// `isFluidExact`.
    pub fn is_fluid_exact(&self) -> bool {
        self.type_ == ResourceType::Fluid && self.fluid_match_mode == FluidMatchMode::Exact
    }

    /// `isFluidPrimary`.
    pub fn is_fluid_primary(&self) -> bool {
        self.fluid_match_mode == FluidMatchMode::Primary
    }

    /// `isFluidMixture`.
    pub fn is_fluid_mixture(&self) -> bool {
        self.fluid_match_mode == FluidMatchMode::Mixture
    }

    /// `isFluidAnything`.
    pub fn is_fluid_anything(&self) -> bool {
        self.fluid_match_mode == FluidMatchMode::Anything
    }

    /// `isVariableAmount`.
    pub fn is_variable_amount(&self) -> bool {
        self.amount != self.maxamount
    }

    /// `getAmount`.
    pub fn amount(&self) -> f32 {
        self.amount
    }

    /// `getIntAmount`.
    pub fn int_amount(&self) -> i32 {
        self.amount as i32
    }

    /// `getMaxAmount`.
    pub fn max_amount(&self) -> f32 {
        self.maxamount
    }

    /// `getIntMaxAmount`.
    pub fn int_max_amount(&self) -> i32 {
        self.maxamount as i32
    }

    /// `getAmount(int)`.
    pub fn amount_at(&self, idx: usize) -> f32 {
        self.amounts.get(idx).copied().unwrap_or(self.amount)
    }

    /// `getMaxAmount(int)`.
    pub fn max_amount_at(&self, idx: usize) -> f32 {
        self.maxamounts.get(idx).copied().unwrap_or(self.maxamount)
    }

    fn index_for_item(&self, item: &str) -> Option<usize> {
        let with_module = |s: &str| {
            if s.contains('.') {
                s.to_string()
            } else {
                format!("Base.{s}")
            }
        };
        let item = with_module(item);
        self.items.iter().position(|it| java::eq_ignore_case(&item, &with_module(it)))
    }

    /// `getAmount(String)`: the amount listed for the item, else 1.
    pub fn amount_for(&self, item: &str) -> f32 {
        self.index_for_item(item).map_or(1.0, |i| self.amount_at(i))
    }

    /// `getMaxAmount(String)`.
    pub fn max_amount_for(&self, item: &str) -> f32 {
        self.index_for_item(item).map_or(1.0, |i| self.max_amount_at(i))
    }

    /// `getRelativeScale`.
    pub fn relative_scale(&self, item: &str) -> f32 {
        let amt = self.amount_for(item);
        if self.amount == 0.0 || amt == 0.0 {
            return 1.0;
        }
        amt / self.amount
    }

    /// `isApplyOnTick`.
    pub fn is_apply_on_tick(&self) -> bool {
        self.apply_on_tick
    }

    /// `isAcceptsAnyItem`.
    pub fn is_accepts_any_item(&self) -> bool {
        self.accepts_any_item
    }

    /// `isAcceptsAnyFluid`.
    pub fn is_accepts_any_fluid(&self) -> bool {
        self.accepts_any_fluid
    }

    /// `isAcceptsAnyEnergy`.
    pub fn is_accepts_any_energy(&self) -> bool {
        self.accepts_any_energy
    }

    /// `getItemTags`.
    pub fn item_tags(&self) -> &[Option<Rc<RegistryKey>>] {
        &self.item_tags
    }

    /// `getItems`: the item names as loaded.
    pub fn loaded_items(&self) -> &[String] {
        &self.loaded_items
    }

    /// The resolved item names (`items`).
    pub fn resolved_items(&self) -> &[String] {
        &self.items
    }

    /// The fluid names as loaded. TODO(Fluid): resolved into the filter.
    pub fn loaded_fluids(&self) -> &[String] {
        &self.loaded_fluids
    }

    /// The fluid category names.
    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    /// The energy names as loaded.
    pub fn loaded_energies(&self) -> &[String] {
        &self.loaded_energies
    }

    /// `amounts`.
    pub fn amounts(&self) -> &[f32] {
        &self.amounts
    }

    /// `maxamounts`.
    pub fn max_amounts(&self) -> &[f32] {
        &self.maxamounts
    }

    /// The item side of `OnPostWorldDictionaryInit`: resolves the listed
    /// items and tags. TODO(Item): the any-item caches filtered by fluid
    /// container / drainable, `getUsedInRecipes` and `isUsedInBuildRecipes`.
    /// TODO(ItemTags): items for each tag (the original throws "Tag has no
    /// items" for an empty tag).
    /// `linked_filtered`: the `+`/`-` line is a fluid or energy line.
    pub(crate) fn resolve_items(
        &mut self,
        sm: &ScriptManager,
        recipe_name: Option<&str>,
        linked_filtered: bool,
    ) -> ScriptResult {
        if self.accepts_any_item {
            self.item_script_cache.clear();
            let filtered =
                linked_filtered || self.has_flag(InputFlag::ItemIsFluid) || self.has_flag(InputFlag::ItemIsEnergy);
            if !filtered {
                self.item_script_cache = sm.get_scripts_for_type(ScriptType::Item);
            }
            return Ok(());
        }
        for s in self.loaded_items.clone() {
            match sm.get_script(ScriptType::Item, &s) {
                Some(item) => {
                    if !self.items.contains(&s) {
                        self.items.push(s.clone());
                        self.item_script_cache.push(item);
                    }
                }
                None if sm.debug => log_error(
                    sm,
                    &format!("{} item not found: {s}. line: {}", recipe_name.unwrap_or("null"), self.original_line),
                ),
                None => {}
            }
        }
        Ok(())
    }
}

/// `Float.equals` identity for `distinct()`: same bits, all NaNs equal.
fn float_key(f: f32) -> u32 {
    if f.is_nan() {
        f32::NAN.to_bits()
    } else {
        f.to_bits()
    }
}

/// The amount element: `N` or `variable[min:max]`, each clamped to `>= 0`.
pub(crate) fn parse_amounts(s: &str) -> ScriptResult<(f32, f32)> {
    if s.contains("variable") {
        let values = java::split_char(bracket_contents(s)?, ':');
        let base = pz_max0(parse_float(array_get(&values, 0)?)?);
        let upper = pz_max0(parse_float(array_get(&values, 1)?)?);
        Ok((base, upper))
    } else {
        let v = pz_max0(parse_float(s)?);
        Ok((v, v))
    }
}

/// `PZMath.max(0.0f, v)`.
fn pz_max0(v: f32) -> f32 {
    if 0.0 > v {
        0.0
    } else {
        v
    }
}
