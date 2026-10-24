//! `zombie.scripting.entity.components.crafting.CraftBenchScript`.

use std::collections::BTreeSet;

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::components::resources::ResourceChannel;
use crate::objects::entity::{
    exception, format_recipe_tag_query, log_error, ComponentBase, ComponentScript, ComponentType,
};
use crate::parser::Block;
use crate::script_type::ScriptLoadMode;

#[derive(Debug)]
pub struct CraftBenchScript {
    pub component: ComponentBase,
    /// `EnumBitStore<ResourceChannel>`.
    fluid_input_channels: BTreeSet<ResourceChannel>,
    energy_input_channels: BTreeSet<ResourceChannel>,
    recipe_tag_query: Option<String>,
}

impl Default for CraftBenchScript {
    fn default() -> Self {
        Self::new()
    }
}

impl CraftBenchScript {
    pub fn new() -> Self {
        CraftBenchScript {
            component: ComponentBase::new(ComponentType::CraftBench),
            fluid_input_channels: BTreeSet::new(),
            energy_input_channels: BTreeSet::new(),
            recipe_tag_query: None,
        }
    }

    /// `getRecipeTagQuery`. TODO(CraftRecipeManager): `getRecipes` queries
    /// the recipe manager with it.
    pub fn recipe_tag_query(&self) -> Option<&str> {
        self.recipe_tag_query.as_deref()
    }

    /// `getFluidInputChannels`.
    pub fn fluid_input_channels(&self) -> &BTreeSet<ResourceChannel> {
        &self.fluid_input_channels
    }

    /// `getEnergyInputChannels`.
    pub fn energy_input_channels(&self) -> &BTreeSet<ResourceChannel> {
        &self.energy_input_channels
    }
}

impl ComponentScript for CraftBenchScript {
    component_accessors!("zombie.scripting.entity.components.crafting.CraftBenchScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        match self.recipe_tag_query.as_deref() {
            Some(q) if !java::is_null_or_whitespace(Some(q)) => {
                self.recipe_tag_query = Some(format_recipe_tag_query(q)?);
                Ok(())
            }
            _ => Err(exception("Recipe tag query null or whitespace.")),
        }
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "recipes") {
                self.recipe_tag_query = Some(val.to_string());
            } else if java::eq_ignore_case(key, "fluidInputChannels") {
                for s in java::split_char(val, ';') {
                    self.fluid_input_channels.insert(ResourceChannel::value_of(s)?);
                }
            } else if java::eq_ignore_case(key, "energyInputChannels") {
                for s in java::split_char(val, ';') {
                    self.energy_input_channels.insert(ResourceChannel::value_of(s)?);
                }
            }
        }
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "craftProcessor") {
                continue;
            }
            log_error(
                sm,
                &format!("Unknown block '{}' in entity script: {}", child.type_str(), self.component.name(sm)?),
            );
        }
        Ok(())
    }
}
