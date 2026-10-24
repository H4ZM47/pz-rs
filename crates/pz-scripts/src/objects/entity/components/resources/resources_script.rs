//! `zombie.scripting.entity.components.resources.ResourcesScript`.

use super::ResourceBlueprint;
use crate::base::{ScriptError, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::objects::entity::{exception, log_error, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;
use crate::script_type::ScriptLoadMode;

#[derive(Debug)]
pub struct ResourcesScript {
    pub component: ComponentBase,
    group_lines: JHashMap<Vec<String>>,
    group_names: Vec<String>,
    blueprints: Vec<ResourceBlueprint>,
    groups: JHashMap<Vec<ResourceBlueprint>>,
}

impl Default for ResourcesScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourcesScript {
    pub fn new() -> Self {
        ResourcesScript {
            component: ComponentBase::new(ComponentType::Resources),
            group_lines: JHashMap::new(),
            group_names: Vec::new(),
            blueprints: Vec::new(),
            groups: JHashMap::new(),
        }
    }

    /// `getGroupNames`.
    pub fn group_names(&self) -> &[String] {
        &self.group_names
    }

    /// `getBlueprintGroup`.
    pub fn blueprint_group(&self, group: &str) -> Option<&[ResourceBlueprint]> {
        self.groups.get(group).map(Vec::as_slice)
    }

    /// Every blueprint, in load order.
    pub fn blueprints(&self) -> &[ResourceBlueprint] {
        &self.blueprints
    }

    /// The raw blueprint lines per group (`groupLines`).
    pub fn group_lines(&self) -> &JHashMap<Vec<String>> {
        &self.group_lines
    }

    /// `loadResourceBlock`: lines containing `@`; `key = value` lines become
    /// `key@value`, others get a generated `<group>_<n>@` id.
    fn load_resource_block(&mut self, group_name: &str, block: &Block) -> ScriptResult {
        if java::is_null_or_whitespace(Some(group_name)) {
            return Err(exception("GroupName cannot be null or whitespace"));
        }
        if !self.group_lines.contains_key(group_name) {
            self.group_lines.insert(group_name.to_string(), Vec::new());
        }
        let list = self.group_lines.get_mut(group_name).unwrap();
        for value in &block.values {
            let line = java::trim(&value.string);
            if java::is_null_or_whitespace(Some(line)) || !line.contains('@') {
                continue;
            }
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            let line = if !key.is_empty() && !val.is_empty() {
                format!("{key}@{val}")
            } else {
                format!("{group_name}_{}@{line}", list.len())
            };
            list.push(line);
        }
        Ok(())
    }
}

impl ComponentScript for ResourcesScript {
    component_accessors!("zombie.scripting.entity.components.resources.ResourcesScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn pre_reload(&mut self) {
        self.group_lines.clear();
        self.group_names.clear();
        self.blueprints.clear();
        self.groups.clear();
    }

    /// `OnScriptsLoaded`: deserializes every line, group by group in
    /// `HashMap` order.
    fn on_scripts_loaded(&mut self, sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        let entries: Vec<(String, Vec<String>)> =
            self.group_lines.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        for (group_name, lines) in entries {
            if !self.groups.contains_key(&group_name) {
                self.groups.insert(group_name.clone(), Vec::new());
            }
            self.group_names.push(group_name.clone());
            for line in lines {
                match ResourceBlueprint::deserialize_from_script(&line) {
                    Ok(bp) => {
                        self.groups.get_mut(&group_name).unwrap().push(bp.clone());
                        self.blueprints.push(bp);
                    }
                    Err(e) => {
                        log_error(
                            sm,
                            &format!(
                                "Error in resource blueprint line: {line}, entity: {}",
                                self.component.name(sm).unwrap_or_default()
                            ),
                        );
                        sm.log_exception(&e);
                        return Err(ScriptError::new(format!("java.lang.Exception: {}", e.0)));
                    }
                }
            }
        }
        Ok(())
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        self.load_resource_block("resources", block)?;
        for child in &block.children {
            let t = child.type_str();
            if java::eq_ignore_case(t, "group") {
                let group = child.id.as_deref();
                if java::is_null_or_whitespace(group) {
                    return Err(exception("Group name cannot be null or whitespace."));
                }
                self.load_resource_block(group.unwrap(), child)?;
            } else if java::eq_ignore_case(t, "internal") {
                sm.warn("internal block is deprecated");
            } else if java::eq_ignore_case(t, "external") {
                sm.warn("external block is deprecated");
            } else {
                log_error(sm, &format!("Unknown block '{t}' in entity script: {}", self.component.name(sm)?));
            }
        }
        Ok(())
    }
}
