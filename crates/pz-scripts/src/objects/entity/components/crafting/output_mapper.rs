//! `zombie.entity.components.crafting.recipe.OutputMapper`: picks a recipe
//! output item from the items used for its registered inputs.

use crate::base::{ScriptRef, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::exception;
use crate::script_type::ScriptType;

/// `OutputMapper.OutputEntree`: a result item and the input items that select it.
#[derive(Clone, Debug, Default)]
pub struct OutputEntree {
    full_type: Option<String>,
    pub result: Option<ScriptRef>,
    pattern_full_types: Vec<String>,
    pub pattern: Vec<ScriptRef>,
}

impl OutputEntree {
    pub fn full_type(&self) -> Option<&str> {
        self.full_type.as_deref()
    }

    pub fn pattern_full_types(&self) -> &[String] {
        &self.pattern_full_types
    }
}

#[derive(Clone, Debug)]
pub struct OutputMapper {
    name: String,
    result_items: Vec<ScriptRef>,
    entrees: Vec<OutputEntree>,
    /// `entreeMap`: result item to index into `entrees`.
    entree_map: Vec<(ScriptRef, usize)>,
    default_output_entree: Option<OutputEntree>,
    /// Indices into the recipe's `ioLines` of the registered inputs.
    input_scripts: Vec<usize>,
}

impl OutputMapper {
    pub fn new(name: &str) -> Self {
        OutputMapper {
            name: name.to_string(),
            result_items: Vec::new(),
            entrees: Vec::new(),
            entree_map: Vec::new(),
            default_output_entree: None,
            input_scripts: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `isEmpty`.
    pub fn is_empty(&self) -> bool {
        self.default_output_entree.is_none() && self.entrees.is_empty()
    }

    /// `getResultItems`.
    pub fn result_items(&self) -> &[ScriptRef] {
        &self.result_items
    }

    /// `getPatternForResult`.
    pub fn pattern_for_result(&self, result: &ScriptRef) -> &[ScriptRef] {
        match self.entree_map.iter().find(|(item, _)| item.ptr_eq(result)) {
            Some((_, i)) => &self.entrees[*i].pattern,
            None => &[],
        }
    }

    /// `registerInputScript`.
    pub fn register_input_script(&mut self, io_index: usize) {
        self.input_scripts.push(io_index);
    }

    /// The registered inputs, as indices into the recipe's `ioLines`.
    pub fn input_scripts(&self) -> &[usize] {
        &self.input_scripts
    }

    /// `setDefaultOutputEntree`.
    pub fn set_default_output_entree(&mut self, item: &str) {
        self.default_output_entree = Some(OutputEntree { full_type: Some(item.to_string()), ..Default::default() });
    }

    pub fn default_output_entree(&self) -> Option<&OutputEntree> {
        self.default_output_entree.as_ref()
    }

    /// `addOutputEntree`.
    pub fn add_output_entree(&mut self, result: &str, items: &[&str]) {
        self.entrees.push(OutputEntree {
            full_type: Some(result.to_string()),
            pattern_full_types: items.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        });
    }

    /// `getEntrees`.
    pub fn entrees(&self) -> &[OutputEntree] {
        &self.entrees
    }

    /// `getItem(fullType, recipe)`. TODO(Item): the original also marks the
    /// item as a craft recipe product and registers researchable recipes.
    fn get_item(sm: &ScriptManager, full_type: Option<&str>, recipe: Option<&str>) -> ScriptResult<ScriptRef> {
        let recipe = recipe.unwrap_or("null");
        let Some(full_type) = full_type.filter(|t| !java::is_null_or_whitespace(Some(t))) else {
            return Err(exception(format!(
                "{recipe} item not found, full type invalid: {}",
                full_type.unwrap_or("null")
            )));
        };
        sm.get_script(ScriptType::Item, full_type)
            .ok_or_else(|| exception(format!("{recipe} item not found: {full_type}")))
    }

    /// `OnPostWorldDictionaryInit(recipe)`: resolves the default and pattern
    /// items. Calling it twice (two outputs sharing a mapper) adds the
    /// results twice, as in the original.
    pub fn on_post_world_dictionary_init(&mut self, sm: &ScriptManager, recipe: Option<&str>) -> ScriptResult {
        if let Some(d) = &mut self.default_output_entree {
            let item = Self::get_item(sm, d.full_type.as_deref(), recipe)?;
            d.result = Some(item.clone());
            self.result_items.push(item);
        }
        for i in 0..self.entrees.len() {
            let item = Self::get_item(sm, self.entrees[i].full_type.as_deref(), recipe)?;
            self.entrees[i].result = Some(item.clone());
            self.result_items.push(item.clone());
            match self.entree_map.iter_mut().find(|(k, _)| k.ptr_eq(&item)) {
                Some(e) => e.1 = i,
                None => self.entree_map.push((item, i)),
            }
            for j in 0..self.entrees[i].pattern_full_types.len() {
                let t = self.entrees[i].pattern_full_types[j].clone();
                let pattern_item = Self::get_item(sm, Some(&t), None)?;
                self.entrees[i].pattern.push(pattern_item);
            }
        }
        Ok(())
    }
    // TODO(CraftRecipeData): `getOutputItem` matches the items used for the
    // registered inputs at craft time.
}
