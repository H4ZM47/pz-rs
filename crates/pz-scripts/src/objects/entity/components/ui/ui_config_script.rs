//! `zombie.scripting.entity.components.ui.UiConfigScript`.

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{log_error, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;

#[derive(Debug)]
pub struct UiConfigScript {
    pub component: ComponentBase,
    xui_skin_name: Option<String>,
    entity_style: Option<String>,
    ui_enabled: bool,
}

impl Default for UiConfigScript {
    fn default() -> Self {
        Self::new()
    }
}

impl UiConfigScript {
    pub fn new() -> Self {
        UiConfigScript {
            component: ComponentBase::new(ComponentType::UiConfig),
            xui_skin_name: None,
            entity_style: None,
            ui_enabled: true,
        }
    }

    /// `getXuiSkinName`.
    pub fn xui_skin_name(&self) -> Option<&str> {
        self.xui_skin_name.as_deref()
    }

    /// `getEntityStyle`.
    pub fn entity_style(&self) -> Option<&str> {
        self.entity_style.as_deref()
    }

    /// `isUiEnabled`.
    pub fn is_ui_enabled(&self) -> bool {
        self.ui_enabled
    }

    /// `getDisplayNameDebug`: the skin's display name for the entity style.
    /// TODO(XuiManager): needs `XuiManager.GetSkin`; `None` stands for
    /// `GameEntity.getDefaultEntityDisplayName()`.
    pub fn display_name_debug(&self) -> Option<String> {
        None
    }
}

impl ComponentScript for UiConfigScript {
    component_accessors!("zombie.scripting.entity.components.ui.UiConfigScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn pre_reload(&mut self) {
        self.xui_skin_name = None;
        self.entity_style = None;
        self.ui_enabled = true;
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "xuiSkin") {
                self.xui_skin_name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "entityStyle") {
                self.entity_style = Some(val.to_string());
            } else if java::eq_ignore_case(key, "uiEnabled") {
                self.ui_enabled = java::parse_bool(val);
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
