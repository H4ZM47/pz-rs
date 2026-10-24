//! Script classes of the `recipes` group.

use std::rc::Rc;

use crate::base::ScriptRef;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

/// `createInstance` for the script types this group ports; `None` for
/// types it does not (yet) handle.
pub fn create_instance(_type: ScriptType, _module: &Rc<ScriptModule>, _name: &str, _token: &str) -> Option<ScriptRef> {
    None
}

/// Java static fields of this group's classes (reset by `ScriptManager.Reset`).
#[derive(Default)]
pub struct Statics {}

/// Group steps run at the end of `ScriptManager.Load`.
pub fn post_load(_sm: &crate::manager::ScriptManager) {}
