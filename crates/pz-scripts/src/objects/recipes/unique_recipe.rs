//! `zombie.scripting.objects.UniqueRecipe`.

use crate::base::{BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::script_type::ScriptType;

use super::{at, parse_int, split_body};

#[derive(Debug)]
pub struct UniqueRecipe {
    pub base: BaseScriptObject,
    name: Option<String>,
    base_recipe: Option<String>,
    items: Vec<String>,
    hunger_bonus: i32,
    hapiness_bonus: i32,
    boredom_bonus: i32,
}

impl UniqueRecipe {
    pub fn new(name: &str) -> Self {
        UniqueRecipe {
            base: BaseScriptObject::new(ScriptType::UniqueRecipe),
            name: Some(name.to_string()),
            base_recipe: None,
            items: Vec::new(),
            hunger_bonus: 0,
            hapiness_bonus: 0,
            boredom_bonus: 0,
        }
    }

    /// `Load(String, String[])`: `key : value` entries.
    pub fn load_values(&mut self, _name: &str, values: &[&str]) -> ScriptResult {
        for s in values {
            if java::trim(s).is_empty() || !s.contains(':') {
                continue;
            }
            let split = java::split_char(s, ':');
            let key = java::trim(at(&split, 0)?);
            let value = java::trim(at(&split, 1)?);
            match key {
                "BaseRecipeItem" => self.base_recipe = Some(value.to_string()),
                "Item" => self.items.push(value.to_string()),
                "Hunger" => self.hunger_bonus = parse_int(value)?,
                "Hapiness" => self.hapiness_bonus = parse_int(value)?,
                "Boredom" => self.boredom_bonus = parse_int(value)?,
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

    /// `getBaseRecipe`.
    pub fn base_recipe(&self) -> Option<&str> {
        self.base_recipe.as_deref()
    }

    /// `setBaseRecipe`.
    pub fn set_base_recipe(&mut self, base_recipe: Option<String>) {
        self.base_recipe = base_recipe;
    }

    /// `getItems`.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// `getHungerBonus`.
    pub fn hunger_bonus(&self) -> i32 {
        self.hunger_bonus
    }

    /// `setHungerBonus`.
    pub fn set_hunger_bonus(&mut self, v: i32) {
        self.hunger_bonus = v;
    }

    /// `getHapinessBonus`.
    pub fn hapiness_bonus(&self) -> i32 {
        self.hapiness_bonus
    }

    /// `setHapinessBonus`.
    pub fn set_hapiness_bonus(&mut self, v: i32) {
        self.hapiness_bonus = v;
    }

    /// `getBoredomBonus`.
    pub fn boredom_bonus(&self) -> i32 {
        self.boredom_bonus
    }

    /// `setBoredomBonus`.
    pub fn set_boredom_bonus(&mut self, v: i32) {
        self.boredom_bonus = v;
    }
}

impl ScriptObject for UniqueRecipe {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.UniqueRecipe"
    }

    fn load(&mut self, _sm: &ScriptManager, name: &str, token: &str) -> ScriptResult {
        let values = split_body(token)?;
        self.base.load_common_block_str(token)?;
        self.load_values(name, &values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_colon_values() {
        let sm = ScriptManager::new();
        let mut r = UniqueRecipe::new("Tasty Soup");
        r.load(
            &sm,
            "Tasty Soup",
            "uniquerecipe Tasty Soup { BaseRecipeItem : Pot, Item : Base.Carrot, Item : Base.Leek, Hunger : 5, Boredom:-3, Other=1, }",
        )
        .unwrap();
        assert_eq!(r.base_recipe(), Some("Pot"));
        assert_eq!(r.items(), ["Base.Carrot", "Base.Leek"]);
        assert_eq!(r.hunger_bonus(), 5);
        assert_eq!(r.boredom_bonus(), -3);
        assert_eq!(r.name(), Some("Tasty Soup"));
    }

    #[test]
    fn bad_int_is_an_error() {
        let sm = ScriptManager::new();
        let mut r = UniqueRecipe::new("x");
        let e = r.load(&sm, "x", "uniquerecipe x { Hunger : lots, }").unwrap_err();
        assert_eq!(e.0, "java.lang.NumberFormatException: For input string: \"lots\"");
    }
}
