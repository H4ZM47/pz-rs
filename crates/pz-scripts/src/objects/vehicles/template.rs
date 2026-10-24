//! `zombie.scripting.objects.VehicleTemplate`.

use std::cell::RefCell;
use std::rc::Rc;

use super::VehicleScript;
use crate::base::{BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

/// A `template vehicle <name> { ... }` block. Its body is loaded into a
/// [`VehicleScript`] the first time a vehicle copies from it.
#[derive(Debug)]
pub struct VehicleTemplate {
    pub base: BaseScriptObject,
    pub name: String,
    /// The whole token, `template vehicle <name> { ... }`.
    pub body: String,
    pub script: Option<Rc<RefCell<VehicleScript>>>,
}

impl VehicleTemplate {
    /// `new VehicleTemplate(module, name, body)`.
    pub fn new(module: &Rc<ScriptModule>, name: &str, body: &str) -> Self {
        let mut base = BaseScriptObject::new(ScriptType::VehicleTemplate);
        base.set_module(module);
        VehicleTemplate { base, name: name.to_string(), body: body.to_string(), script: None }
    }

    /// `getScript`: loads the body on first use. Load errors are printed and
    /// the partly loaded script is kept. The script is stored before it
    /// loads, so a template that refers to itself gets the same instance.
    pub fn get_script(this: &Rc<RefCell<Self>>, sm: &ScriptManager) -> Rc<RefCell<VehicleScript>> {
        if let Some(s) = &this.borrow().script {
            return s.clone();
        }
        let (module, name, body) = {
            let t = this.borrow();
            (t.base.module(), t.name.clone(), t.body.clone())
        };
        let script = Rc::new(RefCell::new(VehicleScript::new()));
        if let Some(m) = &module {
            script.borrow_mut().base.set_module(m);
        }
        this.borrow_mut().script = Some(script.clone());
        if let Err(e) = script.borrow_mut().load_body(sm, &name, &body) {
            sm.log_exception(&e);
        }
        script
    }
}

impl ScriptObject for VehicleTemplate {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.VehicleTemplate"
    }

    /// `BaseScriptObject.Load`, which templates do not override (their
    /// bucket never calls it).
    fn load(&mut self, sm: &ScriptManager, _name: &str, _body: &str) -> ScriptResult {
        let msg = "Load(name,totalFile) not overridden. [VehicleTemplate]";
        if sm.debug {
            return Err(ScriptError::new(format!("java.lang.RuntimeException: {msg}")));
        }
        sm.warn(msg);
        Ok(())
    }
}
