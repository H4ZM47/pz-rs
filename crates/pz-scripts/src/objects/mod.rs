//! Script object types (`zombie.scripting.objects.*` and the other classes
//! `ScriptModule` instantiates). Types not yet ported load as
//! [`GenericScript`], which keeps the parsed body for later consumers.

mod generic;
mod string_list;

use std::rc::Rc;

pub use generic::GenericScript;
pub use string_list::StringListScript;

use crate::base::ScriptRef;
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

/// State the original keeps in static fields of script classes.
#[derive(Default)]
pub struct Statics {}

/// `ScriptBucket.createInstance` for each module bucket.
pub fn create_instance(type_: ScriptType, module: &Rc<ScriptModule>, _name: &str, _token: &str) -> ScriptRef {
    match type_ {
        ScriptType::StringList => ScriptRef::new(StringListScript::new()),
        ScriptType::VehicleTemplate | ScriptType::EntityTemplate => {
            let mut g = GenericScript::new(type_);
            g.base.set_module(module);
            g.template_body = Some(_token.to_string());
            ScriptRef::new(g)
        }
        _ => ScriptRef::new(GenericScript::new(type_)),
    }
}

/// The type-specific steps at the end of `ScriptManager.Load` that depend on
/// ported script classes (`debugItems`, `resolveItemTypes`,
/// `createClothingItemMap`, `createZedDmgMap`).
pub fn post_load(_sm: &ScriptManager) {}

/// Static state cleared by `ScriptManager.Reset`.
pub fn reset_statics(_sm: &ScriptManager) {}
