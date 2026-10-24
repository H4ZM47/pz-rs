//! `zombie.scripting.entity.components.contextmenuconfig.ContextMenuConfigScript`
//! and its nested `EntryScript`.

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{log_error, parse_int, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;

/// `ContextMenuConfigScript.EntryScript`: one `contextEntry` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryScript {
    pub menu: Option<String>,
    pub icon: Option<String>,
    pub timed_action: Option<String>,
    pub open_window: Option<String>,
    pub custom_submenu: Option<String>,
    pub custom_function: Option<String>,
    pub extra_param: Option<String>,
    pub allow_distance: bool,
    pub time: i32,
}

impl Default for EntryScript {
    fn default() -> Self {
        EntryScript {
            menu: None,
            icon: None,
            timed_action: None,
            open_window: None,
            custom_submenu: None,
            custom_function: None,
            extra_param: None,
            allow_distance: false,
            time: 5,
        }
    }
}

#[derive(Debug)]
pub struct ContextMenuConfigScript {
    pub component: ComponentBase,
    entries: Vec<EntryScript>,
}

impl Default for ContextMenuConfigScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenuConfigScript {
    pub fn new() -> Self {
        ContextMenuConfigScript { component: ComponentBase::new(ComponentType::ContextMenuConfig), entries: Vec::new() }
    }

    /// `getEntries`.
    pub fn entries(&self) -> &[EntryScript] {
        &self.entries
    }

    /// `LoadEntry`.
    fn load_entry(block: &Block) -> ScriptResult<EntryScript> {
        let mut script = EntryScript::default();
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            let s = Some(v.to_string());
            if java::eq_ignore_case(k, "menu") {
                script.menu = s;
            } else if java::eq_ignore_case(k, "icon") {
                script.icon = s;
            } else if java::eq_ignore_case(k, "customSubmenu") {
                script.custom_submenu = s;
            } else if java::eq_ignore_case(k, "customFunction") {
                script.custom_function = s;
            } else if java::eq_ignore_case(k, "openWindow") {
                script.open_window = s;
            } else if java::eq_ignore_case(k, "timedAction") {
                script.timed_action = s;
            } else if java::eq_ignore_case(k, "extraParam") {
                script.extra_param = s;
            } else if java::eq_ignore_case(k, "allowDistance") {
                script.allow_distance = java::eq_ignore_case(v, "true");
            } else if java::eq_ignore_case(k, "time") {
                script.time = parse_int(v)?;
            }
        }
        Ok(script)
    }
}

impl ComponentScript for ContextMenuConfigScript {
    component_accessors!("zombie.scripting.entity.components.contextmenuconfig.ContextMenuConfigScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "contextEntry") {
                let entry = Self::load_entry(child)?;
                self.entries.push(entry);
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
