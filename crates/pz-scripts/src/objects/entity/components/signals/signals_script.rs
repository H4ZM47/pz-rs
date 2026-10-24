//! `zombie.scripting.entity.components.signals.SignalsScript`: a component without script data.

use crate::base::ScriptResult;
use crate::objects::entity::{ComponentBase, ComponentScript, ComponentType};

#[derive(Debug)]
pub struct SignalsScript {
    pub component: ComponentBase,
}

impl Default for SignalsScript {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalsScript {
    pub fn new() -> Self {
        SignalsScript { component: ComponentBase::new(ComponentType::Signals) }
    }
}

impl ComponentScript for SignalsScript {
    component_accessors!("zombie.scripting.entity.components.signals.SignalsScript");

    /// `copyFrom`: copies nothing.
    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }
}
