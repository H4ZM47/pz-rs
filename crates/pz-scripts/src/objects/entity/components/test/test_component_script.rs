//! `zombie.scripting.entity.components.test.TestComponentScript`.

use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{log_error, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;

#[derive(Debug)]
pub struct TestComponentScript {
    pub component: ComponentBase,
}

impl Default for TestComponentScript {
    fn default() -> Self {
        Self::new()
    }
}

impl TestComponentScript {
    pub fn new() -> Self {
        TestComponentScript { component: ComponentBase::new(ComponentType::TestComponent) }
    }
}

impl ComponentScript for TestComponentScript {
    component_accessors!("zombie.scripting.entity.components.test.TestComponentScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    /// `load`: reads no values; any child other than `someType` is reported.
    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "someType") {
                continue;
            }
            log_error(
                sm,
                &format!("Unknown block '{}' in entity script: {}", child.type_str(), self.component.name(sm)?),
            );
        }
        Ok(())
    }
}
