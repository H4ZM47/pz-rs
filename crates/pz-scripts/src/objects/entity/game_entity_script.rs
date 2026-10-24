//! `zombie.scripting.entity.GameEntityScript`.

use std::rc::Rc;

use super::components::attributes::attribute_type_from_name;
use super::components::crafting::CraftRecipeComponentScript;
use super::components::ui::UiConfigScript;
use super::{log_error, null_pointer, runtime_exception, ComponentScript, ComponentType, GameEntityTemplate};
use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::parser::Block;
use crate::script_type::{ScriptLoadMode, ScriptType};

#[derive(Debug)]
pub struct GameEntityScript {
    pub base: BaseScriptObject,
    name: Option<String>,
    component_scripts: Vec<Box<dyn ComponentScript>>,
    registry_id: i16,
    exists_as_vanilla: bool,
    mod_id: Option<String>,
    file_abs_path: Option<String>,
}

impl Default for GameEntityScript {
    fn default() -> Self {
        Self::new()
    }
}

impl GameEntityScript {
    pub fn new() -> Self {
        GameEntityScript {
            base: BaseScriptObject::new(ScriptType::Entity),
            name: None,
            component_scripts: Vec::new(),
            registry_id: -1,
            exists_as_vanilla: false,
            mod_id: None,
            file_abs_path: None,
        }
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `getModuleName`.
    pub fn module_name(&self) -> Option<String> {
        self.base.module_name()
    }

    /// `getFullName`.
    pub fn full_name(&self) -> ScriptResult<String> {
        self.base.full_type()
    }

    /// `getComponentScripts`.
    pub fn component_scripts(&self) -> &[Box<dyn ComponentScript>] {
        &self.component_scripts
    }

    /// `hasComponents`.
    pub fn has_components(&self) -> bool {
        !self.component_scripts.is_empty()
    }

    /// `containsComponent`.
    pub fn contains_component(&self, t: ComponentType) -> bool {
        self.component_scripts.iter().any(|c| c.component_type() == t)
    }

    /// `getComponentScriptFor`, untyped.
    pub fn component_script_for(&self, t: ComponentType) -> Option<&dyn ComponentScript> {
        self.component_scripts.iter().find(|c| c.component_type() == t).map(|c| c.as_ref())
    }

    /// `getComponentScriptFor`, cast to the component's class.
    pub fn component<T: ComponentScript>(&self, t: ComponentType) -> Option<&T> {
        self.component_script_for(t)?.as_any().downcast_ref::<T>()
    }

    /// `getDisplayNameDebug`: the UI config's skin display name.
    /// TODO(XuiManager): skins and `GameEntity.getDefaultEntityDisplayName`.
    pub fn display_name_debug(&self) -> Option<String> {
        self.component::<UiConfigScript>(ComponentType::UiConfig).and_then(|ui| ui.display_name_debug())
    }

    fn component_index(&self, t: ComponentType) -> Option<usize> {
        self.component_scripts.iter().position(|c| c.component_type() == t)
    }

    /// `getOrCreateComponentScript`.
    fn get_or_create_component_script(&mut self, t: ComponentType) -> ScriptResult<usize> {
        let idx = match self.component_index(t) {
            Some(i) => i,
            None => {
                let mut c = t.create_component_script()?;
                c.component_mut().set_parent(self.name.as_deref(), self.base.module().as_ref());
                self.component_scripts.push(c);
                self.component_scripts.len() - 1
            }
        };
        let module: Option<Rc<ScriptModule>> = self.base.module();
        let c = self.component_scripts[idx].component_mut();
        match &self.name {
            Some(n) => c.base.init_load_pp(n),
            // BaseScriptObject cannot hold a null name, which Java sets here.
            None => c.base.init_load_pp(""),
        }
        if let Some(m) = module {
            c.base.set_module(&m);
        }
        Ok(idx)
    }

    /// `copyFrom`.
    pub fn copy_from(&mut self, other: &GameEntityScript) -> ScriptResult {
        for script in &other.component_scripts {
            let idx = self.get_or_create_component_script(script.component_type())?;
            self.component_scripts[idx].copy_from(script.as_ref())?;
        }
        Ok(())
    }

    /// `Load(String, String)` for a body parsed elsewhere (the template's
    /// `getScript`).
    pub(crate) fn load_body(&mut self, sm: &ScriptManager, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() || !java::eq_ignore_case(key, "entitytemplate") {
                continue;
            }
            let template = sm.get::<GameEntityTemplate>(ScriptType::EntityTemplate, val).ok_or_else(null_pointer)?;
            let mut template =
                template.try_borrow_mut().map_err(|_| ScriptError::new("java.lang.StackOverflowError"))?;
            let script = template.get_script(sm)?;
            self.copy_from(script)?;
        }
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "component") {
                self.load_component_block(sm, child)?;
                continue;
            }
            log_error(
                sm,
                &format!(
                    "Unknown block '{}' in entity  script: {}",
                    child.type_str(),
                    self.name.as_deref().unwrap_or("null")
                ),
            );
        }
        Ok(())
    }

    /// `LoadAttribute`: an item's attribute key goes to its `Attributes` component.
    pub fn load_attribute(&mut self, sm: &ScriptManager, k: &str, v: &str) -> ScriptResult<bool> {
        if attribute_type_from_name(java::trim(k)).is_some() {
            let idx = self.get_or_create_component_script(ComponentType::Attributes)?;
            return self.component_scripts[idx].parse_key_value(sm, k, v);
        }
        Ok(false)
    }

    /// `LoadComponentBlock`.
    pub fn load_component_block(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        if !java::eq_ignore_case(block.type_str(), "component") || java::is_null_or_whitespace(block.id.as_deref()) {
            return Ok(());
        }
        let id = block.id.as_deref().unwrap_or("");
        let t = match ComponentType::value_of(id) {
            Ok(t) => t,
            Err(e) => {
                sm.log_exception(&e);
                ComponentType::Undefined
            }
        };
        if t != ComponentType::Undefined {
            let idx = self.get_or_create_component_script(t)?;
            self.component_scripts[idx].load(sm, block)?;
        } else {
            sm.warn(&format!("Could not parse component block id = {id}"));
        }
        Ok(())
    }

    /// `getRegistry_id`.
    pub fn registry_id(&self) -> i16 {
        self.registry_id
    }

    /// `setRegistry_id`. TODO(WorldDictionary): the original prints the
    /// dictionary entry before throwing.
    pub fn set_registry_id(&mut self, id: i16) -> ScriptResult {
        if self.registry_id != -1 {
            return Err(runtime_exception(format!(
                "Cannot override existing registry id ({}) to new id ({id}), item: {}",
                self.registry_id,
                self.full_name().unwrap_or_else(|_| "unknown".to_string())
            )));
        }
        self.registry_id = id;
        Ok(())
    }

    /// `getModID`.
    pub fn mod_id(&self) -> Option<&str> {
        self.mod_id.as_deref()
    }

    /// `getExistsAsVanilla`.
    pub fn exists_as_vanilla(&self) -> bool {
        self.exists_as_vanilla
    }

    /// `getFileAbsPath`.
    pub fn file_abs_path(&self) -> Option<&str> {
        self.file_abs_path.as_deref()
    }

    /// `setModID`: only game clients (`GameClient.client`) take the mod id
    /// sent by the server; debug builds reject a different one.
    /// TODO(WorldDictionary): the original prints the entry before throwing.
    pub fn set_mod_id(&mut self, sm: &ScriptManager, client: bool, modid: &str) -> ScriptResult {
        if client {
            match &self.mod_id {
                None => self.mod_id = Some(modid.to_string()),
                Some(m) if m != modid && sm.debug => {
                    return Err(runtime_exception(format!("Cannot override modID. ModID={modid}")));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }
}

impl ScriptObject for GameEntityScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.entity.GameEntityScript"
    }

    /// `InitLoadPP`. TODO(WorldDictionary): `WorldDictionary.onLoadEntity`.
    fn init_load_pp(&mut self, sm: &ScriptManager, name: &str) {
        self.base.init_load_pp(name);
        self.name = Some(name.to_string());
        self.mod_id = sm.current_load_file_mod();
        if self.mod_id.as_deref() == Some("pz-vanilla") {
            self.exists_as_vanilla = true;
        }
        self.file_abs_path = sm.current_load_file_abs_path();
    }

    /// `Load`.
    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        self.load_body(sm, body)
    }

    fn pre_reload(&mut self) {
        self.component_scripts.clear();
    }

    fn on_scripts_loaded(&mut self, sm: &ScriptManager, mode: ScriptLoadMode) -> ScriptResult {
        for c in &mut self.component_scripts {
            c.on_scripts_loaded(sm, mode)?;
        }
        Ok(())
    }

    /// `OnLoadedAfterLua`. The craft recipe component also reads its
    /// sibling UI and sprite configs (`getTranslationName`/`getIconTexture`).
    fn on_loaded_after_lua(&mut self, sm: &ScriptManager) -> ScriptResult {
        let ui = self
            .component::<UiConfigScript>(ComponentType::UiConfig)
            .map(|u| (u.xui_skin_name().map(str::to_string), u.entity_style().map(str::to_string)));
        for i in 0..self.component_scripts.len() {
            let c = &mut self.component_scripts[i];
            c.on_loaded_after_lua(sm)?;
            if let Some(crc) = c.as_any_mut().downcast_mut::<CraftRecipeComponentScript>() {
                crc.apply_entity_ui(ui.as_ref())?;
            }
        }
        Ok(())
    }

    fn on_post_world_dictionary_init(&mut self, sm: &ScriptManager) -> ScriptResult {
        for c in &mut self.component_scripts {
            c.on_post_world_dictionary_init(sm)?;
        }
        Ok(())
    }
}
