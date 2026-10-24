//! `zombie.scripting.entity.components.crafting.DryingLogicScript`.

use super::StartMode;
use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{format_recipe_tag_query, log_error, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;
use crate::script_type::ScriptLoadMode;

#[derive(Debug)]
pub struct DryingLogicScript {
    pub component: ComponentBase,
    drying_recipe_tag_query: Option<String>,
    fuel_recipe_tag_query: Option<String>,
    start_mode: StartMode,
    inputs_group_name: Option<String>,
    outputs_group_name: Option<String>,
    fuel_inputs_group_name: Option<String>,
    fuel_outputs_group_name: Option<String>,
}

impl Default for DryingLogicScript {
    fn default() -> Self {
        Self::new()
    }
}

impl DryingLogicScript {
    pub fn new() -> Self {
        DryingLogicScript {
            component: ComponentBase::new(ComponentType::DryingLogic),
            drying_recipe_tag_query: None,
            fuel_recipe_tag_query: None,
            start_mode: StartMode::Manual,
            inputs_group_name: None,
            outputs_group_name: None,
            fuel_inputs_group_name: None,
            fuel_outputs_group_name: None,
        }
    }

    /// `getDryingRecipeTagQuery`.
    pub fn drying_recipe_tag_query(&self) -> Option<&str> {
        self.drying_recipe_tag_query.as_deref()
    }

    /// `getFuelRecipeTagQuery`.
    pub fn fuel_recipe_tag_query(&self) -> Option<&str> {
        self.fuel_recipe_tag_query.as_deref()
    }

    /// `isUsesFuel`.
    pub fn is_uses_fuel(&self) -> bool {
        self.fuel_recipe_tag_query.is_some()
    }

    /// `getStartMode`.
    pub fn start_mode(&self) -> StartMode {
        self.start_mode
    }

    /// `getInputsGroupName`.
    pub fn inputs_group_name(&self) -> Option<&str> {
        self.inputs_group_name.as_deref()
    }

    /// `getOutputsGroupName`.
    pub fn outputs_group_name(&self) -> Option<&str> {
        self.outputs_group_name.as_deref()
    }

    /// `getFuelInputsGroupName`.
    pub fn fuel_inputs_group_name(&self) -> Option<&str> {
        self.fuel_inputs_group_name.as_deref()
    }

    /// `getFuelOutputsGroupName`.
    pub fn fuel_outputs_group_name(&self) -> Option<&str> {
        self.fuel_outputs_group_name.as_deref()
    }
}

impl ComponentScript for DryingLogicScript {
    component_accessors!("zombie.scripting.entity.components.crafting.DryingLogicScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn pre_reload(&mut self) {
        let component = std::mem::replace(&mut self.component, ComponentBase::new(ComponentType::DryingLogic));
        *self = DryingLogicScript { component, ..DryingLogicScript::new() };
    }

    /// `OnScriptsLoaded`: the fuel query is only formatted when there is a
    /// drying query.
    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        let Some(q) = self.drying_recipe_tag_query.as_deref().filter(|q| !java::is_null_or_whitespace(Some(q))) else {
            return Ok(());
        };
        self.drying_recipe_tag_query = Some(format_recipe_tag_query(q)?);
        let Some(q) = self.fuel_recipe_tag_query.as_deref().filter(|q| !java::is_null_or_whitespace(Some(q))) else {
            return Ok(());
        };
        self.fuel_recipe_tag_query = Some(format_recipe_tag_query(q)?);
        Ok(())
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            let v = Some(val.to_string());
            if java::eq_ignore_case(key, "dryingRecipes") {
                self.drying_recipe_tag_query = v.clone();
            }
            if java::eq_ignore_case(key, "fuelRecipes") {
                self.fuel_recipe_tag_query = v;
            } else if java::eq_ignore_case(key, "startMode") {
                self.start_mode = StartMode::value_of(val)?;
            } else if java::eq_ignore_case(key, "inputGroup") {
                self.inputs_group_name = v;
            } else if java::eq_ignore_case(key, "outputGroup") {
                self.outputs_group_name = v;
            } else if java::eq_ignore_case(key, "fuelInputGroup") {
                self.fuel_inputs_group_name = v;
            } else if java::eq_ignore_case(key, "fuelOutputGroup") {
                self.fuel_outputs_group_name = v;
            }
        }
        for child in &block.children {
            log_error(
                sm,
                &format!("Unknown block '{}' in entity script: {}", child.type_str(), self.component.name(sm)?),
            );
        }
        Ok(())
    }
}
