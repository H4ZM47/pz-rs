//! `zombie.scripting.objects.Recipe` (the legacy recipe format).

use std::collections::HashMap;

use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::BlockElement;
use crate::script_type::ScriptType;

use super::{at, parse_float, parse_int};

/// `Recipe.Result`.
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeResult {
    pub module: Option<String>,
    pub type_: Option<String>,
    pub count: i32,
    pub drainable_count: i32,
}

impl Default for RecipeResult {
    fn default() -> Self {
        RecipeResult { module: None, type_: None, count: 1, drainable_count: 0 }
    }
}

impl RecipeResult {
    /// `getFullType`: Java string concatenation, so a missing part is "null".
    pub fn full_type(&self) -> String {
        format!("{}.{}", self.module.as_deref().unwrap_or("null"), self.type_.as_deref().unwrap_or("null"))
    }
}

/// `Recipe.Source`.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub keep: bool,
    items: Vec<String>,
    original_items: Vec<String>,
    pub destroy: bool,
    pub count: f32,
    pub use_: f32,
}

impl Default for Source {
    fn default() -> Self {
        Source { keep: false, items: Vec::new(), original_items: Vec::new(), destroy: false, count: 1.0, use_: 0.0 }
    }
}

impl Source {
    /// `getItems`.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    pub fn items_mut(&mut self) -> &mut Vec<String> {
        &mut self.items
    }

    /// `getOriginalItems`.
    pub fn original_items(&self) -> &[String] {
        &self.original_items
    }

    /// `getOnlyItem`.
    pub fn only_item(&self) -> ScriptResult<&str> {
        if self.items.len() != 1 {
            return Err(ScriptError::new(format!("java.lang.RuntimeException: items.size() == {}", self.items.len())));
        }
        Ok(&self.items[0])
    }
}

/// `Recipe.RequiredSkill`. The perk is kept by its `PerkFactory.Perk` id.
// TODO(PerkFactory): hold a perk handle once perks are ported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredSkill {
    perk: String,
    level: i32,
}

impl RequiredSkill {
    pub fn new(perk: impl Into<String>, level: i32) -> Self {
        RequiredSkill { perk: perk.into(), level }
    }

    /// `getPerk`.
    pub fn perk(&self) -> &str {
        &self.perk
    }

    /// `getLevel`.
    pub fn level(&self) -> i32 {
        self.level
    }
}

/// `Recipe.LuaCall`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LuaCall {
    LuaAttributes,
    LuaTest,
    LuaCreate,
    LuaGrab,
    LuaCanPerform,
    LuaGiveXP,
}

#[derive(Debug)]
pub struct Recipe {
    pub base: BaseScriptObject,
    pub time_to_make: f32,
    pub sound: Option<String>,
    pub anim_node: Option<String>,
    pub prop1: Option<String>,
    pub prop2: Option<String>,
    pub source: Vec<Source>,
    /// `result`: the first of `results`.
    pub result: Option<RecipeResult>,
    pub results: Vec<RecipeResult>,
    pub allow_destroyed_item: bool,
    pub allow_frozen_item: bool,
    pub allow_rotten_item: bool,
    pub allow_only_one: bool,
    pub in_same_inventory: bool,
    pub name: String,
    originalname: Option<String>,
    required_near_object: Option<String>,
    tooltip: Option<String>,
    pub skill_required: Option<Vec<RequiredSkill>>,
    pub category: Option<String>,
    pub remove_result_item: bool,
    pub stop_on_walk: bool,
    pub stop_on_run: bool,
    pub hidden: bool,
    recipe_file_text: Option<String>,
    lua_calls: HashMap<LuaCall, String>,
}

impl Recipe {
    pub fn new() -> Self {
        Recipe {
            base: BaseScriptObject::new(ScriptType::Recipe),
            time_to_make: 0.0,
            sound: None,
            anim_node: None,
            prop1: None,
            prop2: None,
            source: Vec::new(),
            result: None,
            results: Vec::new(),
            allow_destroyed_item: false,
            allow_frozen_item: false,
            allow_rotten_item: false,
            allow_only_one: false,
            in_same_inventory: false,
            name: "recipe".to_string(),
            originalname: Some("recipe".to_string()),
            required_near_object: None,
            tooltip: None,
            skill_required: None,
            category: None,
            remove_result_item: false,
            stop_on_walk: true,
            stop_on_run: true,
            hidden: false,
            recipe_file_text: None,
            lua_calls: HashMap::new(),
        }
    }

    /// `DoSource`.
    pub fn do_source(&mut self, type_: &str) -> ScriptResult {
        let mut source = Source::default();
        let mut type_ = type_.to_string();
        if type_.contains('=') {
            let split = java::split_char(&type_, '=');
            source.count = parse_float(java::trim(at(&split, 1)?))?;
            type_ = java::trim(at(&split, 0)?).to_string();
        }
        if type_.starts_with("keep") {
            type_ = type_.replace("keep ", "");
            source.keep = true;
        }
        if type_.contains(';') {
            let type_use = java::split_char(&type_, ';');
            let first = at(&type_use, 0)?.to_string();
            source.use_ = parse_float(at(&type_use, 1)?)?;
            type_ = first;
        }
        if type_.starts_with("destroy") {
            type_ = type_.replace("destroy ", "");
            source.destroy = true;
        }
        if type_ == "null" {
            source.items.clear();
            source.original_items.clear();
        } else if type_.contains('/') {
            type_ = java::trim(&type_.replacen("keep ", "", 1)).to_string();
            let parts = java::split_char(&type_, '/');
            source.items.extend(parts.iter().map(|s| s.to_string()));
            source.original_items.extend(parts.iter().map(|s| s.to_string()));
        } else {
            source.items.push(type_.clone());
            source.original_items.push(type_.clone());
        }
        if !type_.is_empty() {
            self.source.push(source);
        }
        Ok(())
    }

    /// `DoResult`.
    pub fn do_result(&mut self, type_: &str) -> ScriptResult {
        let mut result = RecipeResult::default();
        let mut type_ = type_.to_string();
        if type_.contains('=') {
            let split = java::split_char(&type_, '=');
            let t = java::trim(at(&split, 0)?).to_string();
            result.count = parse_int(java::trim(at(&split, 1)?))?;
            type_ = t;
        }
        if type_.contains(';') {
            let split = java::split_char(&type_, ';');
            let t = java::trim(at(&split, 0)?).to_string();
            result.drainable_count = parse_int(java::trim(at(&split, 1)?))?;
            type_ = t;
        }
        if type_.contains('.') {
            let split = java::split_char(&type_, '.');
            result.type_ = Some(at(&split, 1)?.to_string());
            result.module = Some(at(&split, 0)?.to_string());
        } else {
            result.type_ = Some(type_);
        }
        if self.result.is_none() {
            self.result = Some(result.clone());
        }
        self.results.push(result);
        Ok(())
    }

    /// `getNumberOfNeededItem`.
    pub fn number_of_needed_item(&self) -> i32 {
        let mut result = 0i32;
        for source in &self.source {
            if source.items.is_empty() {
                continue;
            }
            result = (result as f32 + source.count) as i32;
        }
        result
    }

    /// `getRequiredSkills`: `"<perk> <level>"` per skill.
    // TODO(PerkFactory): the original prints the perk's display name.
    pub fn required_skills(&self) -> Option<Vec<String>> {
        self.skill_required.as_ref().map(|skills| skills.iter().map(|s| format!("{} {}", s.perk, s.level)).collect())
    }

    /// `getRequiredSkillCount`.
    pub fn required_skill_count(&self) -> usize {
        self.skill_required.as_ref().map_or(0, Vec::len)
    }

    /// `getRequiredSkill`.
    pub fn required_skill(&self, index: i32) -> Option<&RequiredSkill> {
        let skills = self.skill_required.as_ref()?;
        usize::try_from(index).ok().and_then(|i| skills.get(i))
    }

    /// `clearRequiredSkills`.
    pub fn clear_required_skills(&mut self) {
        if let Some(s) = &mut self.skill_required {
            s.clear();
        }
    }

    /// `addRequiredSkill`.
    pub fn add_required_skill(&mut self, perk: &str, level: i32) {
        self.skill_required.get_or_insert_with(Vec::new).push(RequiredSkill::new(perk, level));
    }

    /// `findSource`.
    pub fn find_source(&self, source_full_type: &str) -> Option<&Source> {
        self.source.iter().find(|s| s.items.iter().any(|i| i == source_full_type))
    }

    /// `getSource`.
    pub fn source(&self) -> &[Source] {
        &self.source
    }

    /// `getOriginalname`.
    pub fn originalname(&self) -> Option<&str> {
        self.originalname.as_deref()
    }

    /// `setOriginalname`.
    pub fn set_originalname(&mut self, name: Option<String>) {
        self.originalname = name;
    }

    /// `getFullType`: `module.name + "." + originalname`; `None` where the
    /// original throws for a missing module.
    pub fn full_type(&self) -> Option<String> {
        let module = self.base.module_name()?;
        Some(format!("{module}.{}", self.originalname.as_deref().unwrap_or("null")))
    }

    /// `getName`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `getResult`.
    pub fn result(&self) -> Option<&RecipeResult> {
        self.result.as_ref()
    }

    /// `getResults`.
    pub fn results(&self) -> &[RecipeResult] {
        &self.results
    }

    /// `getNearItem`.
    pub fn near_item(&self) -> Option<&str> {
        self.required_near_object.as_deref()
    }

    /// `setNearItem`.
    pub fn set_near_item(&mut self, near_item: Option<String>) {
        self.required_near_object = near_item;
    }

    /// `isRequiresWorkstation`.
    pub fn is_requires_workstation(&self) -> bool {
        false
    }

    /// `getStationMultiplier`.
    pub fn station_multiplier(&self) -> f32 {
        0.25
    }

    /// `getHeat`.
    pub fn heat(&self) -> f32 {
        0.0
    }

    pub fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    pub fn recipe_file_text(&self) -> Option<&str> {
        self.recipe_file_text.as_deref()
    }

    pub fn lua_calls(&self) -> &HashMap<LuaCall, String> {
        &self.lua_calls
    }
}

impl Default for Recipe {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for Recipe {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.Recipe"
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        // TODO(Translator): Translator.getRecipeName(name); untranslated
        // names fall back to the name itself.
        self.name = name.to_string();
        self.originalname = Some(name.to_string());
        self.recipe_file_text = Some(body.to_string());
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for element in &block.elements {
            let BlockElement::Value(i) = element else { continue };
            let s = &block.values[*i].string;
            if java::trim(s).is_empty() {
                continue;
            }
            if s.contains(':') {
                let split = java::split_char(s, ':');
                let key = java::trim(at(&split, 0)?);
                let value = java::trim(at(&split, 1)?);
                if java::eq_ignore_case(key, "Override") {
                    continue;
                }
                // DebugType.General.error
                sm.script_error(&format!("Could not assign [key]: '{key}' : '{value}'."));
                continue;
            }
            self.do_source(java::trim(s))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_follow_java_parsing() {
        let mut r = Recipe::new();
        r.do_source("keep Base.Saw/Base.Axe").unwrap();
        r.do_source("destroy Base.Plank=2").unwrap();
        r.do_source("Base.Water;5").unwrap();
        r.do_source("null").unwrap();
        r.do_source("keepsake").unwrap();
        assert_eq!(r.source.len(), 5);
        assert!(r.source[0].keep);
        assert_eq!(r.source[0].items(), ["Base.Saw", "Base.Axe"]);
        assert!(r.source[1].destroy);
        assert_eq!(r.source[1].count, 2.0);
        assert_eq!(r.source[1].items(), ["Base.Plank"]);
        assert_eq!(r.source[2].use_, 5.0);
        assert_eq!(r.source[2].items(), ["Base.Water"]);
        assert!(r.source[3].items().is_empty());
        assert!(r.source[4].keep);
        assert_eq!(r.source[4].items(), ["keepsake"]);
        assert_eq!(r.number_of_needed_item(), 5);
        assert_eq!(r.find_source("Base.Axe").map(|s| s.keep), Some(true));
    }

    #[test]
    fn source_errors_carry_java_text() {
        let mut r = Recipe::new();
        assert_eq!(
            r.do_source("Base.Plank=").unwrap_err().0,
            "java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length 1"
        );
        assert_eq!(
            r.do_source("Base.Plank=x").unwrap_err().0,
            "java.lang.NumberFormatException: For input string: \"x\""
        );
    }

    #[test]
    fn results_split_module_and_counts() {
        let mut r = Recipe::new();
        r.do_result("Base.Soup=2").unwrap();
        r.do_result("Jar;3").unwrap();
        assert_eq!(r.result().unwrap().full_type(), "Base.Soup");
        assert_eq!(r.results[0].count, 2);
        assert_eq!(r.results[1].full_type(), "null.Jar");
        assert_eq!(r.results[1].drainable_count, 3);
    }

    #[test]
    fn load_reads_sources_and_skips_keyed_values() {
        let sm = ScriptManager::new();
        let mut r = Recipe::new();
        r.load(&sm, "Make Thing", "recipe Make Thing { keep Base.Saw, Base.Plank=3, Override:true, Time:50.0, }")
            .unwrap();
        assert_eq!(r.name(), "Make Thing");
        assert_eq!(r.source.len(), 2);
        assert_eq!(r.source[1].count, 3.0);
        assert_eq!(sm.take_log().len(), 1);
    }
}
