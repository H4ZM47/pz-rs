//! `zombie.scripting.itemConfig.ItemConfig`.

use std::collections::HashSet;

use super::BucketRootScript;
use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::objects::entity::{array_get, null_pointer};
use crate::parser::BlockElement;
use crate::script_type::{ScriptLoadMode, ScriptType};

const EXCEPTION_CLASS: &str = "zombie.scripting.itemConfig.ItemConfig$ItemConfigException";

/// `createErrorString`, from the error context statics.
fn create_error_string(sm: &ScriptManager) -> String {
    let st = sm.statics.entity.borrow();
    let mut e = format!("[itemConfig={}, ", st.item_config_error_item_config.as_deref().unwrap_or("unknown"));
    if let Some(b) = &st.item_config_error_bucket {
        e.push_str(&format!("bucket={b}, "));
    }
    if let Some(r) = &st.item_config_error_root {
        e.push_str(&format!("attribute={r}, "));
    }
    e.push_str(&format!("line=\"{}\"]]", st.item_config_error_line.as_deref().unwrap_or("null")));
    e
}

/// `new ItemConfigException(msg)`.
pub(crate) fn item_config_exception(sm: &ScriptManager, msg: &str) -> ScriptError {
    ScriptError::new(format!("{EXCEPTION_CLASS}: RecipeAttributes -> {msg} \n{}", create_error_string(sm)))
}

/// `Throwable.getMessage()` of a converted exception.
fn message(e: &ScriptError) -> &str {
    match e.0.find(": ") {
        Some(p) => &e.0[p + 2..],
        None => "null",
    }
}

#[derive(Debug)]
pub struct ItemConfig {
    pub base: BaseScriptObject,
    includes: Vec<String>,
    variables: JHashMap<String>,
    root_scripts: JHashMap<BucketRootScript>,
    name: Option<String>,
    has_been_parsed: bool,
    is_valid: bool,
}

impl Default for ItemConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl ItemConfig {
    pub fn new() -> Self {
        ItemConfig {
            base: BaseScriptObject::new(ScriptType::ItemConfig),
            includes: Vec::new(),
            variables: JHashMap::new(),
            root_scripts: JHashMap::new(),
            name: None,
            has_been_parsed: false,
            is_valid: true,
        }
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `isValid`.
    pub fn is_valid(&self) -> bool {
        self.is_valid
    }

    /// The `include` names.
    pub fn includes(&self) -> &[String] {
        &self.includes
    }

    /// The `variables` block (keys and values as written).
    pub fn variables(&self) -> &JHashMap<String> {
        &self.variables
    }

    /// The root scripts by `Type[:id]`.
    pub fn root_scripts(&self) -> &JHashMap<BucketRootScript> {
        &self.root_scripts
    }

    pub fn has_been_parsed(&self) -> bool {
        self.has_been_parsed
    }

    fn load_inner(&mut self, sm: &ScriptManager, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for element in &block.elements {
            let child = match element {
                BlockElement::Value(i) => {
                    let s = &block.values[*i].string;
                    sm.statics.entity.borrow_mut().item_config_error_line = Some(s.clone());
                    if java::is_null_or_whitespace(Some(s)) {
                        continue;
                    }
                    if s.contains('=') {
                        let split = java::split_char(s, '=');
                        let k = java::trim(array_get(&split, 0)?);
                        let v = java::trim(array_get(&split, 1)?);
                        if java::eq_ignore_case(k, "include") {
                            self.includes.push(v.to_string());
                        }
                    }
                    sm.statics.entity.borrow_mut().item_config_error_line = None;
                    continue;
                }
                BlockElement::Block(i) => &block.children[*i],
            };
            if java::eq_ignore_case("includes", child.type_str()) {
                for value in &child.values {
                    if !java::is_null_or_whitespace(Some(&value.string)) {
                        self.includes.push(java::trim(&value.string).to_string());
                    }
                }
                continue;
            }
            if java::eq_ignore_case("variables", child.type_str()) {
                for value in &child.values {
                    if !java::is_null_or_whitespace(Some(&value.string)) {
                        self.variables.insert(value.get_key().to_string(), value.get_value().to_string());
                    }
                }
                continue;
            }
            let root_script = BucketRootScript::try_load(sm, child)?;
            let mut root_id = root_script.root_type().name().to_string();
            if let Some(id) = root_script.id() {
                root_id = format!("{root_id}:{id}");
            }
            self.root_scripts.insert(root_id, root_script);
        }
        Ok(())
    }

    /// `Parse(includeSet)`: resolves the includes (parsing them first) and
    /// detects include cycles. The original merges the included roots and
    /// variables into local maps it then discards, so only the parse state
    /// and the errors are observable.
    fn parse(&mut self, sm: &ScriptManager, include_set: Option<&mut HashSet<String>>) -> ScriptResult {
        let name = self.name.clone().unwrap_or_else(|| "null".to_string());
        let mut include_set = include_set;
        if let Some(set) = include_set.as_deref_mut() {
            if !set.insert(name.clone()) {
                return Err(item_config_exception(sm, "Circular includes detected."));
            }
        }
        if self.has_been_parsed {
            return Ok(());
        }
        let mut own_set: HashSet<String>;
        for include in self.includes.clone() {
            let set: &mut HashSet<String> = match include_set.as_deref_mut() {
                Some(s) => s,
                None => {
                    own_set = HashSet::new();
                    &mut own_set
                }
            };
            set.insert(name.clone());
            let parent = sm.get::<ItemConfig>(ScriptType::ItemConfig, &include).ok_or_else(null_pointer)?;
            // A config already being parsed up the stack has its name in the set.
            let Ok(mut parent) = parent.try_borrow_mut() else {
                return Err(item_config_exception(sm, "Circular includes detected."));
            };
            if !parent.has_been_parsed {
                parent.parse(sm, Some(set))?;
            }
        }
        self.has_been_parsed = true;
        Ok(())
    }
    // TODO(ItemConfigurator): `BuildBuckets` turns the root scripts into
    // `BucketRoot`/`SelectorBucket`/`Randomizer` trees (selector ids from
    // the configurator, generators from attribute and fluid types), used by
    // `ConfigureEntitySpawned`/`ConfigureEntityOnCreate`.
}

impl ScriptObject for ItemConfig {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.itemConfig.ItemConfig"
    }

    /// `Load`: any failure is rethrown as an `ItemConfigException`.
    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.name = Some(name.to_string());
        {
            let mut st = sm.statics.entity.borrow_mut();
            st.item_config_error_line = None;
            st.item_config_error_bucket = None;
            st.item_config_error_root = None;
            st.item_config_error_item_config = Some(name.to_string());
        }
        if let Err(e) = self.load_inner(sm, body) {
            return Err(if e.0.starts_with(EXCEPTION_CLASS) {
                ScriptError::new(format!("{EXCEPTION_CLASS}: {}", message(&e)))
            } else {
                item_config_exception(sm, message(&e))
            });
        }
        let mut st = sm.statics.entity.borrow_mut();
        st.item_config_error_line = None;
        st.item_config_error_bucket = None;
        st.item_config_error_root = None;
        st.item_config_error_item_config = None;
        Ok(())
    }

    fn pre_reload(&mut self) {
        self.has_been_parsed = false;
        self.includes.clear();
        self.variables.clear();
        self.root_scripts.clear();
    }

    fn on_scripts_loaded(&mut self, sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        sm.statics.entity.borrow_mut().item_config_error_item_config = self.name.clone();
        if let Err(e) = self.parse(sm, None) {
            sm.log_exception(&e);
            return Err(ScriptError::new(format!("java.lang.Exception: {}", e.0)));
        }
        sm.statics.entity.borrow_mut().item_config_error_item_config = None;
        Ok(())
    }
}
