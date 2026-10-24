//! `zombie.scripting.entity.components.parts.PartsScript`: a component without script data.

use crate::base::ScriptResult;
use crate::objects::entity::{ComponentBase, ComponentScript, ComponentType};

#[derive(Debug)]
pub struct PartsScript {
    pub component: ComponentBase,
}

impl Default for PartsScript {
    fn default() -> Self {
        Self::new()
    }
}

impl PartsScript {
    pub fn new() -> Self {
        PartsScript { component: ComponentBase::new(ComponentType::Parts) }
    }
}

impl ComponentScript for PartsScript {
    component_accessors!("zombie.scripting.entity.components.parts.PartsScript");

    /// `copyFrom`: copies nothing.
    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }
}
