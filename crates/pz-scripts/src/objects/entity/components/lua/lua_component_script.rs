//! `zombie.scripting.entity.components.lua.LuaComponentScript`: a component without script data.

use crate::base::ScriptResult;
use crate::objects::entity::{ComponentBase, ComponentScript, ComponentType};

#[derive(Debug)]
pub struct LuaComponentScript {
    pub component: ComponentBase,
}

impl Default for LuaComponentScript {
    fn default() -> Self {
        Self::new()
    }
}

impl LuaComponentScript {
    pub fn new() -> Self {
        LuaComponentScript { component: ComponentBase::new(ComponentType::Lua) }
    }
}

impl ComponentScript for LuaComponentScript {
    component_accessors!("zombie.scripting.entity.components.lua.LuaComponentScript");

    /// `copyFrom`: copies nothing.
    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }
}
