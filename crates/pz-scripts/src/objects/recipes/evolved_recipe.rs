//! `zombie.scripting.objects.EvolvedRecipe`.
//!
//! The inventory-side methods (`needToBeCooked`, `getItemsCanBeUse`,
//! `addItem`, `isSpiceAdded`, `hasMinimumWater`, ...) operate on
//! `InventoryItem`/`Food` and are left to the inventory port.
// TODO(InventoryItem): port the item interaction methods.

use crate::base::{BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::script_type::ScriptType;

use super::{at, parse_float, parse_int, split_body, ItemRecipe};

#[derive(Debug)]
pub struct EvolvedRecipe {
    pub base: BaseScriptObject,
    pub name: String,
    pub display_name: Option<String>,
    originalname: Option<String>,
    pub max_items: i32,
    /// Filled by `Item.OnScriptsLoaded`, keyed by item name.
    pub items_list: JHashMap<ItemRecipe>,
    pub result_item: Option<String>,
    pub base_item: Option<String>,
    pub cookable: bool,
    pub add_ingredient_if_cooked: bool,
    pub can_add_spices_empty: bool,
    pub add_ingredient_sound: Option<String>,
    pub hidden: bool,
    pub allow_frozen_item: bool,
    pub template: Option<String>,
    pub minimum_water: f32,
}

impl EvolvedRecipe {
    pub fn new(name: &str) -> Self {
        EvolvedRecipe {
            base: BaseScriptObject::new(ScriptType::EvolvedRecipe),
            name: name.to_string(),
            display_name: None,
            originalname: None,
            max_items: 0,
            items_list: JHashMap::new(),
            result_item: None,
            base_item: None,
            cookable: false,
            add_ingredient_if_cooked: false,
            can_add_spices_empty: false,
            add_ingredient_sound: None,
            hidden: false,
            allow_frozen_item: false,
            template: None,
            minimum_water: 0.0,
        }
    }

    /// `Load(String, String[])`: `key = value` entries.
    fn load_values(&mut self, name: &str, values: &[&str]) -> ScriptResult {
        // TODO(Translator): displayName = Translator.getRecipeName(...);
        // untranslated names fall back to the name itself.
        self.display_name = Some(name.to_string());
        self.originalname = Some(name.to_string());
        for s in values {
            if java::trim(s).is_empty() || !s.contains('=') {
                continue;
            }
            let (key, value) = s.split_once('=').unwrap();
            let key = java::trim(key);
            let value = java::trim(value);
            match key {
                "BaseItem" => self.base_item = Some(value.to_string()),
                "Name" => {
                    self.display_name = Some(value.to_string());
                    self.originalname = Some(value.to_string());
                }
                "ResultItem" => self.result_item = Some(value.to_string()),
                "Cookable" => self.cookable = true,
                "MaxItems" => self.max_items = parse_int(value)?,
                "AddIngredientIfCooked" => self.add_ingredient_if_cooked = java::parse_bool(value),
                "AddIngredientSound" => {
                    self.add_ingredient_sound = (!java::is_null_or_whitespace(Some(value))).then(|| value.to_string());
                }
                "CanAddSpicesEmpty" => self.can_add_spices_empty = java::parse_bool(value),
                "IsHidden" => self.hidden = java::parse_bool(value),
                "AllowFrozenItem" => self.allow_frozen_item = java::parse_bool(value),
                "Template" => self.template = Some(value.to_string()),
                "MinimumWater" => self.minimum_water = parse_float(value)?,
                _ => {}
            }
        }
        if self.template.is_none() {
            self.template = Some(name.to_string());
        }
        Ok(())
    }

    /// `getName`: the display name.
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    /// `getOriginalname`.
    pub fn originalname(&self) -> Option<&str> {
        self.originalname.as_deref()
    }

    /// `getUntranslatedName`.
    pub fn untranslated_name(&self) -> &str {
        &self.name
    }

    /// `getBaseItem`.
    pub fn base_item(&self) -> Option<&str> {
        self.base_item.as_deref()
    }

    /// `getMinimumWater`.
    pub fn minimum_water(&self) -> f32 {
        self.minimum_water
    }

    /// `getItemsList`.
    pub fn items_list(&self) -> &JHashMap<ItemRecipe> {
        &self.items_list
    }

    /// `getItemRecipe(item)` by the item's type.
    pub fn item_recipe(&self, item_type: &str) -> Option<&ItemRecipe> {
        self.items_list.get(item_type)
    }

    /// `getPossibleItems`: `itemsList.values()` in `HashMap` order.
    pub fn possible_items(&self) -> Vec<&ItemRecipe> {
        self.items_list.values().collect()
    }

    /// `getResultItem`: the type part of `resultItem`.
    pub fn result_item(&self) -> ScriptResult<&str> {
        let r = self.result_item.as_deref().ok_or_else(|| ScriptError::new("java.lang.NullPointerException"))?;
        if !r.contains('.') {
            return Ok(r);
        }
        at(&java::split_char(r, '.'), 1)
    }

    /// `getFullResultItem`.
    pub fn full_result_item(&self) -> Option<&str> {
        self.result_item.as_deref()
    }

    /// `isCookable`.
    pub fn is_cookable(&self) -> bool {
        self.cookable
    }

    /// `getMaxItems`.
    pub fn max_items(&self) -> i32 {
        self.max_items
    }

    /// `isResultItem(item)` by the item's type.
    pub fn is_result_item_type(&self, item_type: Option<&str>) -> ScriptResult<bool> {
        match item_type {
            None => Ok(false),
            Some(t) => Ok(self.result_item()? == t),
        }
    }

    /// `getAddIngredientSound`.
    pub fn add_ingredient_sound(&self) -> Option<&str> {
        self.add_ingredient_sound.as_deref()
    }

    /// `setIsHidden`.
    pub fn set_is_hidden(&mut self, hide: bool) {
        self.hidden = hide;
    }

    /// `isHidden`.
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// `isAllowFrozenItem`.
    pub fn is_allow_frozen_item(&self) -> bool {
        self.allow_frozen_item
    }

    /// `setAllowFrozenItem`.
    pub fn set_allow_frozen_item(&mut self, allow: bool) {
        self.allow_frozen_item = allow;
    }
}

impl ScriptObject for EvolvedRecipe {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.EvolvedRecipe"
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
    fn loads_values_and_defaults_template() {
        let sm = ScriptManager::new();
        let mut r = EvolvedRecipe::new("Broth");
        r.load(
            &sm,
            "Broth",
            "evolvedrecipe Broth { BaseItem = Base.Kettle, MaxItems = 3, ResultItem = Base.KettleBroth, \
             Cookable = false, Name = Make Broth, AddIngredientSound = , MinimumWater = 0.5, IsHidden = TRUE, }",
        )
        .unwrap();
        assert_eq!(r.base_item(), Some("Base.Kettle"));
        assert_eq!(r.max_items(), 3);
        assert!(r.is_cookable());
        assert!(r.is_hidden());
        assert_eq!(r.display_name(), Some("Make Broth"));
        assert_eq!(r.originalname(), Some("Make Broth"));
        assert_eq!(r.template.as_deref(), Some("Broth"));
        assert_eq!(r.add_ingredient_sound(), None);
        assert_eq!(r.minimum_water(), 0.5);
        assert_eq!(r.result_item().unwrap(), "KettleBroth");
        assert!(r.is_result_item_type(Some("KettleBroth")).unwrap());
    }

    #[test]
    fn empty_body_has_no_values_segment() {
        let sm = ScriptManager::new();
        let mut r = EvolvedRecipe::new("x");
        let e = r.load(&sm, "x", "evolvedrecipe x {}").unwrap_err();
        assert_eq!(e.0, "java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length 1");
    }
}
