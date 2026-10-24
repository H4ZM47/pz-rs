//! `zombie.scripting.entity.components.crafting.OutputScript`: one output
//! line of a craft recipe (`item 1 Base.Thing chance:0.5`).

use std::collections::BTreeSet;

use super::input_script::parse_amounts;
use super::{CraftRecipe, FluidMatchMode, ItemApplyMode, OutputFlag, OutputMapper};
use crate::base::{ScriptRef, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::components::resources::ResourceType;
use crate::objects::entity::{
    array_get, bracket_contents, exception, index_of, parse_float, parse_int, substring_from,
};

/// The output's `outputMapper`: one of the recipe's named `itemMapper`s
/// (`mapper:<name>`), or a private mapper for a single item.
#[derive(Clone, Debug)]
pub enum OutputMapperRef {
    Named(String),
    Own(OutputMapper),
}

#[derive(Clone, Debug)]
pub struct OutputScript {
    pub(crate) type_: ResourceType,
    pub(crate) loaded_fluid: Option<String>,
    pub(crate) loaded_energy: Option<String>,
    pub(crate) amount: f32,
    pub(crate) maxamount: f32,
    pub(crate) chance: f32,
    pub(crate) apply_on_tick: bool,
    pub(crate) shaped_index: i32,
    pub(crate) item_apply_mode: ItemApplyMode,
    pub(crate) fluid_match_mode: FluidMatchMode,
    pub(crate) original_line: String,
    /// Index into the recipe's `ioLines` of the `+` line.
    pub(crate) create_to_item_script: Option<usize>,
    pub(crate) flags: BTreeSet<OutputFlag>,
    pub(crate) output_mapper: Option<OutputMapperRef>,
    /// Whether `fluid`/`energy` were resolved. TODO(Fluid), TODO(Energy):
    /// `Fluid.Get` / `Energy.Get`.
    pub(crate) resolved: bool,
}

impl OutputScript {
    fn new(type_: ResourceType) -> Self {
        OutputScript {
            type_,
            loaded_fluid: None,
            loaded_energy: None,
            amount: 0.0,
            maxamount: 0.0,
            chance: 1.0,
            apply_on_tick: false,
            shaped_index: -1,
            item_apply_mode: ItemApplyMode::Normal,
            fluid_match_mode: FluidMatchMode::Exact,
            original_line: String::new(),
            create_to_item_script: None,
            flags: BTreeSet::new(),
            output_mapper: None,
            resolved: false,
        }
    }

    /// `Load(parentRecipe, line, isInternal)`.
    pub(crate) fn load(
        recipe: &CraftRecipe,
        sm: &ScriptManager,
        line: &str,
        is_internal: bool,
    ) -> ScriptResult<Option<OutputScript>> {
        if java::is_null_or_whitespace(Some(line)) {
            return Ok(None);
        }
        let elems: Vec<&str> = java::split_ws(java::trim(line)).into_iter().map(java::trim).collect();
        let e0 = array_get(&elems, 0)?;
        let type_ = if java::eq_ignore_case(e0, "item") {
            ResourceType::Item
        } else if java::eq_ignore_case(e0, "fluid") {
            ResourceType::Fluid
        } else if java::eq_ignore_case(e0, "energy") {
            ResourceType::Energy
        } else {
            return Err(exception(format!("unknown type in craftrecipe: {e0}")));
        };
        let mut output = OutputScript::new(type_);
        output.original_line = java::trim(line).to_string();
        let (baseamount, upperamount) = parse_amounts(array_get(&elems, 1)?)?;
        output.amount = baseamount;
        output.maxamount = upperamount;
        let result = array_get(&elems, 2)?;
        match type_ {
            ResourceType::Item if !is_internal => {
                if let Some(name) = result.strip_prefix("mapper:") {
                    if recipe.output_mapper(name).is_none() {
                        return Err(exception(format!("Could not find output mapper: {name}")));
                    }
                    output.output_mapper = Some(OutputMapperRef::Named(name.to_string()));
                } else {
                    let mut mapper = OutputMapper::new(result);
                    mapper.set_default_output_entree(result);
                    output.output_mapper = Some(OutputMapperRef::Own(mapper));
                }
            }
            ResourceType::Item => {
                if !java::eq_ignore_case("Uses", result) && sm.debug {
                    return Err(exception("Parameter with index=2 should be 'Uses'."));
                }
            }
            ResourceType::Fluid => output.loaded_fluid = Some(result.to_string()),
            ResourceType::Energy => output.loaded_energy = Some(result.to_string()),
            ResourceType::Any => {}
        }
        for &s in &elems[3..] {
            if s.starts_with("chance:") {
                output.chance = parse_float(substring_from(s, index_of(s, ":") + 1)?)?.clamp(0.0, 1.0);
            } else if s.starts_with("shapedIndex:") {
                output.shaped_index = parse_int(substring_from(s, index_of(s, ":") + 1)?)?;
            } else if s.starts_with("apply:") {
                if is_internal {
                    return Err(exception("Cannot apply 'onTick' on 'itemCreate' ('+' lines)."));
                }
                if !java::eq_ignore_case(substring_from(s, index_of(s, ":") + 1)?, "onTick") {
                    return Err(exception("Apply Error"));
                }
                match type_ {
                    ResourceType::Item => return Err(exception("Cannot apply 'onTick' on item.")),
                    ResourceType::Fluid => return Err(exception("Cannot apply 'onTick' on fluid.")),
                    _ => output.apply_on_tick = true,
                }
            } else if s.starts_with("mode:") {
                let m = substring_from(s, index_of(s, ":") + 1)?;
                match type_ {
                    ResourceType::Item => {}
                    ResourceType::Fluid => {
                        output.fluid_match_mode = if java::eq_ignore_case(m, "exact") {
                            FluidMatchMode::Exact
                        } else if java::eq_ignore_case(m, "primary") {
                            FluidMatchMode::Primary
                        } else if java::eq_ignore_case(m, "anything") {
                            FluidMatchMode::Anything
                        } else {
                            return Err(exception("Invalid fluid mode Error"));
                        };
                    }
                    _ => {
                        sm.warn(&format!("Cannot set mode for type = {}", type_.name()));
                        if sm.debug {
                            return Err(exception("Mode Error"));
                        }
                    }
                }
            } else if s.starts_with("flags") {
                for entry in java::split_char(bracket_contents(s)?, ';') {
                    output.flags.insert(OutputFlag::value_of(entry)?);
                }
            } else {
                return Err(exception(format!("unknown recipe param: {s}")));
            }
        }
        Ok(Some(output))
    }

    /// `hasCreateToItem`.
    pub fn has_create_to_item(&self) -> bool {
        self.create_to_item_script.is_some()
    }

    /// `getCreateToItemScript`, as an index into the recipe's `ioLines`.
    pub fn create_to_item_script(&self) -> Option<usize> {
        self.create_to_item_script
    }

    /// `hasFlag`.
    pub fn has_flag(&self, flag: OutputFlag) -> bool {
        self.flags.contains(&flag)
    }

    pub fn flags(&self) -> &BTreeSet<OutputFlag> {
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

    /// `getChance`.
    pub fn chance(&self) -> f32 {
        self.chance
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

    /// `isVariableAmount`.
    pub fn is_variable_amount(&self) -> bool {
        self.amount != self.maxamount
    }

    /// `getShapedIndex` (deprecated).
    pub fn shaped_index(&self) -> i32 {
        self.shaped_index
    }

    /// `isApplyOnTick`.
    pub fn is_apply_on_tick(&self) -> bool {
        self.apply_on_tick
    }

    /// `isHandcraftOnly`.
    pub fn is_handcraft_only(&self) -> bool {
        self.has_flag(OutputFlag::HandcraftOnly)
    }

    /// `isAutomationOnly`.
    pub fn is_automation_only(&self) -> bool {
        self.has_flag(OutputFlag::AutomationOnly)
    }

    /// The `outputMapper` field.
    pub fn output_mapper_ref(&self) -> Option<&OutputMapperRef> {
        self.output_mapper.as_ref()
    }

    /// The fluid name. TODO(Fluid): `getFluid`.
    pub fn loaded_fluid(&self) -> Option<&str> {
        self.loaded_fluid.as_deref()
    }

    /// The energy name. TODO(Energy): `getEnergy`.
    pub fn loaded_energy(&self) -> Option<&str> {
        self.loaded_energy.as_deref()
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
        self.type_ == ResourceType::Fluid && self.fluid_match_mode == FluidMatchMode::Primary
    }

    /// `isFluidAnything`.
    pub fn is_fluid_anything(&self) -> bool {
        self.type_ == ResourceType::Fluid && self.fluid_match_mode == FluidMatchMode::Anything
    }

    /// `containsItem`: always true in the original.
    pub fn contains_item(&self, _item: &ScriptRef) -> bool {
        true
    }
}
