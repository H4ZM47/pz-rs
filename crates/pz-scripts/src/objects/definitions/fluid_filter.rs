//! `zombie.scripting.objects.FluidFilterScript` and the data of the
//! `zombie.entity.components.fluids.FluidFilter` it builds.

use super::{exception, FluidCategory};
use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::{ScriptLoadMode, ScriptType};

/// `FluidFilter.FilterType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterType {
    Whitelist,
    Blacklist,
}

/// What `createFilter` puts into a `FluidFilter`. Fluids stay names here:
/// resolving them with `Fluid.Get` (throwing "Cannot add fluid '<s>' in
/// filter script." when missing) belongs to the fluid registry. TODO(Fluid)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FluidFilter {
    pub filter_type: FilterType,
    pub categories: Vec<FluidCategory>,
    pub fluids: Vec<String>,
}

#[derive(Debug)]
pub struct FluidFilterScript {
    pub base: BaseScriptObject,
    fluids: Vec<String>,
    categories: Vec<String>,
    is_whitelist: bool,
    filter: Option<FluidFilter>,
    name: Option<String>,
    anonymous: bool,
}

impl FluidFilterScript {
    pub fn new() -> Self {
        Self::with_anonymous(false)
    }

    fn with_anonymous(anonymous: bool) -> Self {
        FluidFilterScript {
            base: BaseScriptObject::new(ScriptType::FluidFilter),
            fluids: Vec::new(),
            categories: Vec::new(),
            is_whitelist: true,
            filter: None,
            name: None,
            anonymous,
        }
    }

    /// `GetAnonymous()`.
    pub fn get_anonymous() -> Self {
        Self::with_anonymous(true)
    }

    /// `GetAnonymous(boolean)`.
    pub fn get_anonymous_with(is_whitelist: bool) -> Self {
        let mut ffs = Self::with_anonymous(true);
        ffs.is_whitelist = is_whitelist;
        ffs
    }

    /// `copy`.
    pub fn copy(&self) -> Self {
        let mut ffs = Self::with_anonymous(self.anonymous);
        ffs.fluids.extend_from_slice(&self.fluids);
        ffs.categories.extend_from_slice(&self.categories);
        ffs.is_whitelist = self.is_whitelist;
        ffs.name = self.name.clone();
        ffs
    }

    /// `isSingleFluid`.
    pub fn is_single_fluid(&self) -> bool {
        self.fluids.len() == 1 && self.categories.is_empty()
    }

    pub fn fluids(&self) -> &[String] {
        &self.fluids
    }

    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    pub fn is_whitelist(&self) -> bool {
        self.is_whitelist
    }

    pub fn is_anonymous(&self) -> bool {
        self.anonymous
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `getFilter`.
    pub fn filter(&self) -> Option<&FluidFilter> {
        self.filter.as_ref()
    }

    /// `createFilter`. The original first throws "Fluids not yet
    /// initialized." in debug builds when the fluid registry is not ready.
    /// TODO(Fluid)
    pub fn create_filter(&mut self) -> ScriptResult<&FluidFilter> {
        let filter_type = if self.is_whitelist { FilterType::Whitelist } else { FilterType::Blacklist };
        let filter = self.filter.insert(FluidFilter { filter_type, categories: Vec::new(), fluids: Vec::new() });
        for s in &self.categories {
            filter.categories.push(FluidCategory::value_of(s)?);
        }
        filter.fluids.extend(self.fluids.iter().cloned());
        Ok(filter)
    }

    /// `LoadAnonymousFromBlock`.
    pub fn load_anonymous_from_block(&mut self, block: &Block) -> ScriptResult {
        if !self.anonymous {
            return Err(exception("Cannot load, is not anonymous"));
        }
        self.name = Some("anonymous".to_string());
        self.read_block(block);
        Ok(())
    }

    /// `LoadAnonymousSingleFluid`.
    pub fn load_anonymous_single_fluid(&mut self, fluid_name: &str) -> ScriptResult {
        if !self.anonymous {
            return Err(exception("Cannot load, is not anonymous"));
        }
        self.name = Some("anonymous".to_string());
        self.fluids.push(fluid_name.to_string());
        Ok(())
    }

    /// `readBlock`.
    fn read_block(&mut self, block: &Block) {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "fluid") {
                parse_input_string(&mut self.fluids, val);
            } else if java::eq_ignore_case(key, "category") {
                parse_input_string(&mut self.categories, val);
            } else if java::eq_ignore_case(key, "filterType") {
                self.is_whitelist = java::eq_ignore_case(val, "whitelist");
            } else if java::eq_ignore_case(key, "whitelist") {
                self.is_whitelist = java::parse_bool(val);
            } else if java::eq_ignore_case(key, "blacklist") {
                self.is_whitelist = !java::parse_bool(val);
            }
        }
        for child in &block.children {
            if java::eq_ignore_case("fluids", child.type_str()) {
                read_filter_block(child, &mut self.fluids);
            } else if java::eq_ignore_case("categories", child.type_str()) {
                read_filter_block(child, &mut self.categories);
            }
        }
    }
}

/// `readFilterBlock`: bare values only.
fn read_filter_block(block: &Block, list: &mut Vec<String>) {
    for value in &block.values {
        let s = java::trim(&value.string);
        if s.is_empty() || s.contains('=') {
            continue;
        }
        parse_input_string(list, s);
    }
}

/// `parseInputString`: `/`-separated names, deduplicated.
fn parse_input_string(list: &mut Vec<String>, input: &str) {
    for s in java::split_char(input, '/') {
        let s = java::trim(s);
        if !list.iter().any(|e| e == s) {
            list.push(s.to_string());
        }
    }
}

impl Default for FluidFilterScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for FluidFilterScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.FluidFilterScript"
    }

    /// `Load`.
    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        if self.anonymous {
            return Err(exception("Cannot load, is anonymous"));
        }
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.name = Some(name.to_string());
        self.base.load_common_block(block);
        self.read_block(block);
        Ok(())
    }

    /// `PreReload`.
    fn pre_reload(&mut self) {
        self.fluids.clear();
        self.categories.clear();
        self.filter = None;
        self.is_whitelist = true;
    }

    /// `OnScriptsLoaded`: non-anonymous filters register their full type
    /// with `StringDictionary.Generic` on init. TODO(WorldDictionary)
    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        Ok(())
    }

    /// `OnPostWorldDictionaryInit`.
    fn on_post_world_dictionary_init(&mut self, _sm: &ScriptManager) -> ScriptResult {
        self.create_filter()?;
        Ok(())
    }
}
