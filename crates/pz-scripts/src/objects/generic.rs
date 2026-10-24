//! Placeholder for script classes that have not been ported yet: keeps the
//! parsed block of the last loaded body so the data stays reachable.

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

#[derive(Debug)]
pub struct GenericScript {
    pub base: BaseScriptObject,
    /// The script's own block (`parse(body).children.get(0)`).
    pub block: Option<Block>,
    /// Templates keep their whole token, as `VehicleTemplate.body` does.
    pub template_body: Option<String>,
}

impl GenericScript {
    pub fn new(type_: ScriptType) -> Self {
        GenericScript { base: BaseScriptObject::new(type_), block: None, template_body: None }
    }
}

impl ScriptObject for GenericScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "GenericScript"
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let mut root = sm.parse(body);
        if root.children.is_empty() {
            return Err(index_error());
        }
        let block = root.children.swap_remove(0);
        // Every ported class except Fixing reads the common block.
        if self.base.script_object_type() != ScriptType::Fixing {
            self.base.load_common_block(&block);
        }
        self.block = Some(block);
        Ok(())
    }

    fn reset(&mut self) {
        self.block = None;
    }
}
