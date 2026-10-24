//! `zombie.scripting.objects.Fixing`.
//!
//! `usedInFixer`, `haveGlobalItem`, `haveThisFixer`, `countUses`,
//! `getRequiredFixerItems` and `getRequiredItems` search a character's
//! inventory and are left to the inventory port.
// TODO(InventoryItem): port the inventory queries.

use crate::base::{BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::script_type::ScriptType;

use super::{at, parse_float, parse_int, split_body};

/// `Fixing.FixerSkill`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixerSkill {
    skill_name: String,
    skill_lvl: i32,
}

impl FixerSkill {
    pub fn new(skill_name: &str, skill_lvl: i32) -> Self {
        FixerSkill { skill_name: skill_name.to_string(), skill_lvl }
    }

    /// `getSkillName`.
    pub fn skill_name(&self) -> &str {
        &self.skill_name
    }

    /// `getSkillLevel`.
    pub fn skill_level(&self) -> i32 {
        self.skill_lvl
    }
}

/// `Fixing.Fixer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fixer {
    fixer_name: String,
    skills: Option<Vec<FixerSkill>>,
    number_of_use: i32,
}

impl Fixer {
    pub fn new(name: &str, skills: Option<Vec<FixerSkill>>, number_of_use: i32) -> Self {
        Fixer { fixer_name: name.to_string(), skills, number_of_use }
    }

    /// `getFixerName`.
    pub fn fixer_name(&self) -> &str {
        &self.fixer_name
    }

    /// `getFixerSkills`.
    pub fn fixer_skills(&self) -> Option<&[FixerSkill]> {
        self.skills.as_deref()
    }

    /// `getNumberOfUse`.
    pub fn number_of_use(&self) -> i32 {
        self.number_of_use
    }
}

#[derive(Debug)]
pub struct Fixing {
    pub base: BaseScriptObject,
    name: Option<String>,
    require: Option<Vec<String>>,
    fixers: Vec<Fixer>,
    global_item: Option<Fixer>,
    condition_modifier: f32,
}

impl Fixing {
    pub fn new() -> Self {
        Fixing {
            base: BaseScriptObject::new(ScriptType::Fixing),
            name: None,
            require: None,
            fixers: Vec::new(),
            global_item: None,
            condition_modifier: 1.0,
        }
    }

    /// `Load(String, String[])`.
    fn load_values(&mut self, name: &str, values: &[&str]) -> ScriptResult {
        self.name = Some(name.to_string());
        for s in values {
            if java::trim(s).is_empty() || !s.contains('=') {
                continue;
            }
            let (key, value) = s.split_once('=').unwrap();
            let key = java::trim(key);
            let value = java::trim(value);
            match key {
                "Require" => {
                    for r in java::split_char(value, ';') {
                        self.add_required_item(java::trim(r));
                    }
                }
                "Fixer" => {
                    if value.contains(';') {
                        let skill_list = java::split_char(value, ';');
                        let mut final_list = Vec::new();
                        for skill in skill_list.iter().skip(1) {
                            let ss = java::split_char(java::trim(skill), '=');
                            final_list
                                .push(FixerSkill::new(java::trim(at(&ss, 0)?), parse_int(java::trim(at(&ss, 1)?))?));
                        }
                        let first = java::trim(at(&skill_list, 0)?);
                        if first.contains('=') {
                            let ss = java::split_char(first, '=');
                            self.fixers.push(Fixer::new(at(&ss, 0)?, Some(final_list), parse_int(at(&ss, 1)?)?));
                        } else {
                            self.fixers.push(Fixer::new(first, Some(final_list), 1));
                        }
                    } else {
                        self.fixers.push(parse_fixer(value)?);
                    }
                }
                "GlobalItem" => self.global_item = Some(parse_fixer(value)?),
                "ConditionModifier" => self.condition_modifier = parse_float(value)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `setName`.
    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// `getRequiredItem`.
    pub fn required_item(&self) -> Option<&[String]> {
        self.require.as_deref()
    }

    /// `addRequiredItem`.
    pub fn add_required_item(&mut self, require: &str) {
        self.require.get_or_insert_with(Vec::new).push(require.to_string());
    }

    /// `getFixers`.
    pub fn fixers(&self) -> &[Fixer] {
        &self.fixers
    }

    /// `getGlobalItem`.
    pub fn global_item(&self) -> Option<&Fixer> {
        self.global_item.as_ref()
    }

    /// `setGlobalItem`.
    pub fn set_global_item(&mut self, global_item: Option<Fixer>) {
        self.global_item = global_item;
    }

    /// `getConditionModifier`.
    pub fn condition_modifier(&self) -> f32 {
        self.condition_modifier
    }

    /// `setConditionModifier`.
    pub fn set_condition_modifier(&mut self, v: f32) {
        self.condition_modifier = v;
    }
}

/// A `Fixer`/`GlobalItem` value without skills: `Item` or `Item=uses`
/// (neither part trimmed).
fn parse_fixer(value: &str) -> ScriptResult<Fixer> {
    if value.contains('=') {
        let ss = java::split_char(value, '=');
        return Ok(Fixer::new(at(&ss, 0)?, None, parse_int(at(&ss, 1)?)?));
    }
    Ok(Fixer::new(value, None, 1))
}

impl Default for Fixing {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for Fixing {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.Fixing"
    }

    /// `Load`: unlike the other recipe types, no common block is read.
    fn load(&mut self, _sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        let values = split_body(body)?;
        self.load_values(name, &values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_fixers_and_skills() {
        let sm = ScriptManager::new();
        let mut f = Fixing::new();
        f.load(
            &sm,
            "Fix Lamp",
            "fixing Fix Lamp { Require = Base.Lamp; Base.Lantern, Fixer = Base.Wire=2; Electricity=3;Tinkering=1, \
             Fixer = Base.Tape, Fixer = Base.Glue=3, GlobalItem = Base.Pliers=1, ConditionModifier = 0.8, enabled = false, }",
        )
        .unwrap();
        assert_eq!(f.name(), Some("Fix Lamp"));
        assert_eq!(f.required_item().unwrap(), ["Base.Lamp", "Base.Lantern"]);
        assert_eq!(f.fixers().len(), 3);
        let w = &f.fixers()[0];
        assert_eq!((w.fixer_name(), w.number_of_use()), ("Base.Wire", 2));
        let skills = w.fixer_skills().unwrap();
        assert_eq!(skills[1].skill_name(), "Tinkering");
        assert_eq!(skills[0].skill_level(), 3);
        assert_eq!(f.fixers()[1].fixer_skills(), None);
        assert_eq!(f.fixers()[2].number_of_use(), 3);
        assert_eq!(f.global_item().unwrap().fixer_name(), "Base.Pliers");
        assert_eq!(f.condition_modifier(), 0.8);
        assert!(f.base.enabled);
    }

    #[test]
    fn untrimmed_use_count_fails_like_java() {
        let sm = ScriptManager::new();
        let mut f = Fixing::new();
        let e = f.load(&sm, "x", "fixing x { GlobalItem = Base.Pliers = 1, }").unwrap_err();
        assert_eq!(e.0, "java.lang.NumberFormatException: For input string: \" 1\"");
    }
}
