//! `zombie.scripting.entity.GameEntityTemplate`: an `entity` template whose
//! body is loaded into a `GameEntityScript` the first time an entity
//! includes it (`entitytemplate = Name`).

use std::rc::Rc;

use super::GameEntityScript;
use crate::base::{BaseScriptObject, ScriptObject, ScriptResult};
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

#[derive(Debug)]
pub struct GameEntityTemplate {
    pub base: BaseScriptObject,
    pub name: String,
    pub body: String,
    pub script: Option<GameEntityScript>,
}

impl GameEntityTemplate {
    /// `GameEntityTemplate(module, name, body)`.
    pub fn new(module: &Rc<ScriptModule>, name: &str, body: &str) -> Self {
        let mut base = BaseScriptObject::new(ScriptType::EntityTemplate);
        base.set_module(module);
        GameEntityTemplate { base, name: name.to_string(), body: body.to_string(), script: None }
    }

    /// `getScript`: loads the template body on first use.
    pub fn get_script(&mut self, sm: &ScriptManager) -> ScriptResult<&GameEntityScript> {
        if self.script.is_none() {
            let mut script = GameEntityScript::new();
            if let Some(m) = self.base.module() {
                script.base.set_module(&m);
            }
            // Java keeps the half-loaded script when Load throws.
            let r = script.load_body(sm, &self.body);
            self.script = Some(script);
            r?;
        }
        Ok(self.script.as_ref().unwrap())
    }
}

impl ScriptObject for GameEntityTemplate {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.entity.GameEntityTemplate"
    }

    fn load(&mut self, _sm: &ScriptManager, _name: &str, _body: &str) -> ScriptResult {
        Ok(())
    }
}
