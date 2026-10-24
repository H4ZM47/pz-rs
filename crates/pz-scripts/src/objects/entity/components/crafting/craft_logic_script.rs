//! `zombie.scripting.entity.components.crafting.CraftLogicScript`, which
//! also loads `DryingCraftLogicScript` (a subclass without changes).

use super::StartMode;
use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{format_recipe_tag_query, log_error, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;
use crate::script_type::ScriptLoadMode;

#[derive(Debug)]
pub struct CraftLogicScript {
    pub component: ComponentBase,
    recipe_tag_query: Option<String>,
    start_mode: StartMode,
    inputs_group_name: Option<String>,
    outputs_group_name: Option<String>,
    action_anim: Option<String>,
}

impl CraftLogicScript {
    /// `CraftLogicScript(ComponentType)`: `CraftLogic` or `DryingCraftLogic`.
    pub fn new(type_: ComponentType) -> Self {
        CraftLogicScript {
            component: ComponentBase::new(type_),
            recipe_tag_query: None,
            start_mode: StartMode::Manual,
            inputs_group_name: None,
            outputs_group_name: None,
            action_anim: None,
        }
    }

    /// `getRecipeTagQuery`.
    pub fn recipe_tag_query(&self) -> Option<&str> {
        self.recipe_tag_query.as_deref()
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

    /// `getActionAnim`.
    pub fn action_anim(&self) -> Option<&str> {
        self.action_anim.as_deref()
    }
}

impl ComponentScript for CraftLogicScript {
    fn component(&self) -> &ComponentBase {
        &self.component
    }

    fn component_mut(&mut self) -> &mut ComponentBase {
        &mut self.component
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn class_name(&self) -> &'static str {
        if self.component.component_type() == ComponentType::DryingCraftLogic {
            "zombie.scripting.entity.components.crafting.DryingCraftLogicScript"
        } else {
            "zombie.scripting.entity.components.crafting.CraftLogicScript"
        }
    }

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn pre_reload(&mut self) {
        self.recipe_tag_query = None;
        self.start_mode = StartMode::Manual;
        self.inputs_group_name = None;
        self.outputs_group_name = None;
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
            } else if java::eq_ignore_case(key, "startMode") {
                self.start_mode = StartMode::value_of(val)?;
            } else if java::eq_ignore_case(key, "inputGroup") {
                self.inputs_group_name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "outputGroup") {
                self.outputs_group_name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "actionAnim") {
                self.action_anim = Some(val.to_string());
            }
        }
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "craftProcessor") {
                sm.warn("Block craft processor is deprecated.");
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
