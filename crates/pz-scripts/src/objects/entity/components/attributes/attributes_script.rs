//! `zombie.scripting.entity.components.attributes.AttributesScript` and the
//! attribute names `zombie.entity.components.attributes.Attribute` registers.

use crate::base::ScriptResult;
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::objects::entity::component_script::cast;
use crate::objects::entity::{log_error, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;

/// Names of the attribute types `Attribute` registers statically.
/// TODO(AttributeType): value types, ids and bounds live with the runtime
/// attribute system.
const ATTRIBUTE_TYPE_NAMES: &[&str] = &[
    "TestString",
    "TestString2",
    "TestQuality",
    "TestCondition",
    "TestBool",
    "TestUses",
    "TestItemType",
    "TestCategories",
    "TestTags",
    "Sharpness",
    "HeadCondition",
    "HeadConditionMax",
    "Quality",
    "TimesHeadRepaired",
    "OriginX",
    "OriginY",
    "OriginZ",
];

/// `Attribute.TypeFromName`: case-insensitive lookup; returns the
/// registered name.
pub fn attribute_type_from_name(name: &str) -> Option<&'static str> {
    let lower = java::to_lower(name);
    ATTRIBUTE_TYPE_NAMES.iter().copied().find(|n| java::to_lower(n) == lower)
}

#[derive(Debug)]
pub struct AttributesScript {
    pub component: ComponentBase,
    kv_pairs: JHashMap<String>,
    has_created_container: bool,
}

impl Default for AttributesScript {
    fn default() -> Self {
        Self::new()
    }
}

impl AttributesScript {
    pub fn new() -> Self {
        AttributesScript {
            component: ComponentBase::new(ComponentType::Attributes),
            kv_pairs: JHashMap::new(),
            has_created_container: false,
        }
    }

    /// The `kvPairs` map: attribute key as written to its value.
    pub fn kv_pairs(&self) -> &JHashMap<String> {
        &self.kv_pairs
    }

    /// `createTemplateContainer`. TODO(AttributeContainer): building the
    /// template container (`putFromScript` per pair) belongs to the runtime
    /// attribute system.
    fn create_template_container(&mut self) {
        self.has_created_container = true;
    }
}

impl ComponentScript for AttributesScript {
    component_accessors!("zombie.scripting.entity.components.attributes.AttributesScript");

    fn pre_reload(&mut self) {
        self.kv_pairs.clear();
        self.has_created_container = false;
    }

    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: crate::script_type::ScriptLoadMode) -> ScriptResult {
        if !self.has_created_container {
            self.create_template_container();
        }
        Ok(())
    }

    /// `copyFrom`: `kvPairs.putAll(other.kvPairs)`.
    fn copy_from(&mut self, other: &dyn ComponentScript) -> ScriptResult {
        let other = cast::<AttributesScript>(other, self)?;
        for (k, v) in other.kv_pairs.iter() {
            self.kv_pairs.insert(k.to_string(), v.clone());
        }
        Ok(())
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            self.parse_key_value(sm, k, v)?;
        }
        Ok(())
    }

    fn parse_key_value(&mut self, sm: &ScriptManager, k: &str, v: &str) -> ScriptResult<bool> {
        if attribute_type_from_name(k).is_some() {
            self.kv_pairs.insert(k.to_string(), v.to_string());
            return Ok(true);
        }
        log_error(sm, &format!("Unknown attribute, key = {k}, value = {v}"));
        Ok(false)
    }
}
