//! `zombie.scripting.objects.ItemFilterScript`: white/black lists of item
//! types, item kinds (`ItemType`) and tags.

use std::rc::Rc;

use super::item::Item;
use super::keys::ItemType;
use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::registry::{RegistryKey, ResourceLocation};
use crate::script_type::ScriptType;

/// `ItemFilterScript.FilterTypeInfo`.
#[derive(Clone, Debug, Default)]
pub struct FilterTypeInfo {
    pub loaded_items: Vec<String>,
    pub loaded_types: Vec<String>,
    pub items: Vec<String>,
    pub item_types: Vec<Option<Rc<RegistryKey>>>,
    pub tags: Vec<String>,
}

fn key_eq(a: Option<&Rc<RegistryKey>>, b: Option<&Rc<RegistryKey>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.location == b.location,
        (None, None) => true,
        _ => false,
    }
}

impl FilterTypeInfo {
    fn reset(&mut self) {
        *self = FilterTypeInfo::default();
    }

    /// `hasEntries`.
    pub fn has_entries(&self) -> bool {
        !self.items.is_empty() || !self.item_types.is_empty() || !self.tags.is_empty()
    }

    /// `containsItem(String, ItemType, Set<ItemTag>)`: tags match by path.
    pub fn contains(
        &self,
        full_type: Option<&str>,
        item_type: Option<&Rc<RegistryKey>>,
        tags: Option<&super::KeySet>,
    ) -> bool {
        let Some(full_type) = full_type else {
            return false;
        };
        if !self.has_entries() {
            return false;
        }
        if self.items.iter().any(|i| i == full_type) {
            return true;
        }
        if self
            .item_types
            .iter()
            .any(|t| key_eq(t.as_ref(), item_type))
        {
            return true;
        }
        if let Some(tags) = tags.filter(|t| !self.tags.is_empty() && !t.is_empty()) {
            // `Registries.ITEM_TAG.getLocation(null)` throws in the original.
            return tags
                .iter()
                .flatten()
                .any(|t| self.tags.iter().any(|p| p == t.location.path()));
        }
        false
    }

    /// `containsItem(Item)`.
    pub fn contains_item(&self, item: Option<&Item>) -> bool {
        item.is_some_and(|i| {
            self.contains(
                Some(&i.full_name()),
                i.item_type.as_ref(),
                Some(&i.item_tags),
            )
        })
    }
}

/// `ScriptManager.resolveGetItemTypes`: entries written `[function]` are
/// replaced by the items a Lua function returns.
pub fn resolve_get_item_types(source_items: &mut Vec<String>) -> ScriptResult {
    for i in (0..source_items.len()).rev() {
        let type1 = &source_items[i];
        if !type1.starts_with('[') {
            continue;
        }
        let end = java::index_of(type1, "]", 0);
        if end < 1 {
            return Err(ScriptError::new(format!(
                "java.lang.StringIndexOutOfBoundsException: begin 1, end {end}, length {}",
                java::utf16_len(type1)
            )));
        }
        source_items.remove(i);
        // TODO(Lua): call the function (`LuaManager.getFunctionObject`) and
        // insert the full names of the items it adds at `i`; a missing
        // function adds nothing.
    }
    Ok(())
}

/// `zombie.scripting.objects.ItemFilterScript`.
#[derive(Debug)]
pub struct ItemFilterScript {
    pub base: BaseScriptObject,
    pub whitelist: FilterTypeInfo,
    pub blacklist: FilterTypeInfo,
    pub has_parsed: bool,
    pub name: Option<String>,
}

impl Default for ItemFilterScript {
    fn default() -> Self {
        Self::new()
    }
}

/// `parseInputString`: `/`-separated, trimmed, without duplicates.
fn parse_input_string(list: &mut Vec<String>, input: &str) {
    for s in java::split_char(input, '/') {
        let s = java::trim(s);
        if !list.iter().any(|x| x == s) {
            list.push(s.to_string());
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

impl ItemFilterScript {
    pub fn new() -> Self {
        ItemFilterScript {
            base: BaseScriptObject::new(ScriptType::ItemFilter),
            whitelist: FilterTypeInfo::default(),
            blacklist: FilterTypeInfo::default(),
            has_parsed: false,
            name: None,
        }
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn info(&mut self, blacklist: bool) -> &mut FilterTypeInfo {
        if blacklist {
            &mut self.blacklist
        } else {
            &mut self.whitelist
        }
    }

    /// `readBlock`: a nested `blacklist` block reads into the blacklist.
    fn read_block(&mut self, block: &Block, blacklist: bool) {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            let info = self.info(blacklist);
            if java::eq_ignore_case(key, "items") {
                parse_input_string(&mut info.loaded_items, val);
            } else if java::eq_ignore_case(key, "types") {
                parse_input_string(&mut info.loaded_types, val);
            } else if java::eq_ignore_case(key, "tags") {
                parse_input_string(&mut info.tags, val);
            }
        }
        for child in &block.children {
            let t = child.type_str();
            let info = self.info(blacklist);
            if java::eq_ignore_case(t, "items") {
                read_filter_block(child, &mut info.loaded_items);
            } else if java::eq_ignore_case(t, "types") {
                read_filter_block(child, &mut info.loaded_types);
            } else if java::eq_ignore_case(t, "tags") {
                read_filter_block(child, &mut info.tags);
            } else if java::eq_ignore_case(t, "blacklist") {
                self.read_block(child, true);
            }
        }
    }

    /// `resolveItemTypes(FilterTypeInfo)`.
    fn resolve_types(sm: &ScriptManager, info: &mut FilterTypeInfo) -> ScriptResult {
        for s in info.loaded_types.clone() {
            let t = ItemType::get(sm, &ResourceLocation::of(&s)?);
            if !info
                .item_types
                .iter()
                .any(|x| key_eq(x.as_ref(), t.as_ref()))
            {
                info.item_types.push(t);
            }
        }
        Ok(())
    }

    /// `parseFilter`.
    fn parse_filter(&mut self, sm: &ScriptManager) -> ScriptResult {
        if self.has_parsed {
            sm.warn(&format!(
                "Already parsed filter: {}",
                self.name.as_deref().unwrap_or("null")
            ));
            return Ok(());
        }
        Self::resolve_types(sm, &mut self.whitelist)?;
        Self::resolve_types(sm, &mut self.blacklist)?;
        for info in [&mut self.whitelist, &mut self.blacklist] {
            info.items = info.loaded_items.clone();
            if !info.items.is_empty() {
                resolve_get_item_types(&mut info.items)?;
            }
        }
        self.has_parsed = true;
        Ok(())
    }

    /// `allowsItem(Item)`.
    pub fn allows_item(&self, item: Option<&Item>) -> bool {
        if self.blacklist.contains_item(item) {
            return false;
        }
        !self.whitelist.has_entries() || self.whitelist.contains_item(item)
    }
}

impl ScriptObject for ItemFilterScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.ItemFilterScript"
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.name = Some(name.to_string());
        self.base.load_common_block(block);
        self.read_block(block, false);
        Ok(())
    }

    fn pre_reload(&mut self) {
        self.has_parsed = false;
        self.whitelist.reset();
        self.blacklist.reset();
    }

    fn on_loaded_after_lua(&mut self, sm: &ScriptManager) -> ScriptResult {
        self.parse_filter(sm)
    }
}
