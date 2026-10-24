//! `zombie.scripting.entity.components.crafting.MashingLogicScript`.

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{format_recipe_tag_query, log_error, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;
use crate::script_type::ScriptLoadMode;

#[derive(Debug)]
pub struct MashingLogicScript {
    pub component: ComponentBase,
    recipe_tag_query: Option<String>,
    resource_fluid_id: Option<String>,
    inputs_group_name: Option<String>,
}

impl Default for MashingLogicScript {
    fn default() -> Self {
        Self::new()
    }
}

impl MashingLogicScript {
    pub fn new() -> Self {
        MashingLogicScript {
            component: ComponentBase::new(ComponentType::MashingLogic),
            recipe_tag_query: None,
            resource_fluid_id: None,
            inputs_group_name: None,
        }
    }

    /// `getRecipeTagQuery`.
    pub fn recipe_tag_query(&self) -> Option<&str> {
        self.recipe_tag_query.as_deref()
    }

    /// `getResourceFluidID`.
    pub fn resource_fluid_id(&self) -> Option<&str> {
        self.resource_fluid_id.as_deref()
    }

    /// `getInputsGroupName`.
    pub fn inputs_group_name(&self) -> Option<&str> {
        self.inputs_group_name.as_deref()
    }
}

impl ComponentScript for MashingLogicScript {
    component_accessors!("zombie.scripting.entity.components.crafting.MashingLogicScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn pre_reload(&mut self) {
        self.recipe_tag_query = None;
        self.inputs_group_name = None;
        self.resource_fluid_id = None;
    }

    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        let Some(q) = self.recipe_tag_query.as_deref().filter(|q| !java::is_null_or_whitespace(Some(q))) else {
            return Ok(());
        };
        self.recipe_tag_query = Some(format_recipe_tag_query(q)?);
        Ok(())
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
            } else if java::eq_ignore_case(key, "inputGroup") {
                self.inputs_group_name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "fluidID") {
                self.resource_fluid_id = Some(val.to_string());
            }
        }
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "someType") {
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
