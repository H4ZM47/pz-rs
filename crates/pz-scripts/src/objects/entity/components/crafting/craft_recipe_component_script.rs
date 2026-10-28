//! `zombie.scripting.entity.components.crafting.CraftRecipeComponentScript`:
//! a craft recipe embedded in an entity (its build recipe).

use super::{craft_recipe_tag, CraftRecipe};
use crate::base::{ScriptObject, ScriptResult};
use crate::manager::ScriptManager;
use crate::objects::entity::{null_pointer, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;
use crate::script_type::ScriptLoadMode;

#[derive(Debug)]
pub struct CraftRecipeComponentScript {
    pub component: ComponentBase,
    craft_recipe: Option<CraftRecipe>,
}

impl Default for CraftRecipeComponentScript {
    fn default() -> Self {
        Self::new()
    }
}

impl CraftRecipeComponentScript {
    pub fn new() -> Self {
        CraftRecipeComponentScript { component: ComponentBase::new(ComponentType::CraftRecipe), craft_recipe: None }
    }

    /// `getCraftRecipe`.
    pub fn craft_recipe(&self) -> Option<&CraftRecipe> {
        self.craft_recipe.as_ref()
    }

    pub fn craft_recipe_mut(&mut self) -> Option<&mut CraftRecipe> {
        self.craft_recipe.as_mut()
    }

    fn recipe_mut(&mut self) -> ScriptResult<&mut CraftRecipe> {
        self.craft_recipe.as_mut().ok_or_else(null_pointer)
    }

    /// `getBuildCategory`: the recipe's first tag.
    pub fn build_category(&self) -> Option<&str> {
        self.craft_recipe.as_ref()?.tags().first().map(String::as_str)
    }

    /// The rest of `OnLoadedAfterLua`: `getTranslationName` and
    /// `getIconTexture` read the parent entity's UI config (skin name and
    /// entity style). Without a UI config the recipe keeps its own name.
    /// TODO(XuiManager): with one, the skin style's display name (or the
    /// `EC_Entity_DisplayName_Default` translation) replaces it, and the
    /// skin style's icon, or the sprite config's first tile, the icon.
    pub(crate) fn apply_entity_ui(&mut self, ui_config: Option<&(Option<String>, Option<String>)>) -> ScriptResult {
        let recipe = self.recipe_mut()?;
        if ui_config.is_none() {
            let name = recipe.translation_name().map(str::to_string);
            recipe.override_translation_name(name);
        }
        Ok(())
    }
}

impl ComponentScript for CraftRecipeComponentScript {
    component_accessors!("zombie.scripting.entity.components.crafting.CraftRecipeComponentScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn pre_reload(&mut self) {
        self.craft_recipe = None;
    }

    fn on_scripts_loaded(&mut self, sm: &ScriptManager, mode: ScriptLoadMode) -> ScriptResult {
        self.recipe_mut()?.on_scripts_loaded(sm, mode)
    }

    fn on_loaded_after_lua(&mut self, sm: &ScriptManager) -> ScriptResult {
        self.recipe_mut()?.on_loaded_after_lua(sm)
    }

    fn on_post_world_dictionary_init(&mut self, sm: &ScriptManager) -> ScriptResult {
        self.recipe_mut()?.on_post_world_dictionary_init(sm)
    }

    /// `load`: a new recipe named after the entity, tagged as an entity recipe.
    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        let name = self.component.base.script_object_name().to_string();
        let mut recipe = CraftRecipe::new();
        recipe.init_load_pp(sm, &name);
        if let Some(m) = self.component.base.module() {
            recipe.base.set_module(&m);
        }
        let mut block = block.clone();
        block.add_value("tags", craft_recipe_tag::ENTITY_RECIPE);
        let recipe = self.craft_recipe.insert(recipe);
        recipe.load_block(sm, &name, &block)
    }
}
