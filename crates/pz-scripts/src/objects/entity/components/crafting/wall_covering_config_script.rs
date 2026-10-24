//! `zombie.scripting.entity.components.crafting.WallCoveringConfigScript`.

use super::{SignType, WallCoveringType};
use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{null_pointer, parse_int, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;

#[derive(Debug)]
pub struct WallCoveringConfigScript {
    pub component: ComponentBase,
    type_: Option<WallCoveringType>,
    name: Option<String>,
    sign: Option<SignType>,
}

impl Default for WallCoveringConfigScript {
    fn default() -> Self {
        Self::new()
    }
}

impl WallCoveringConfigScript {
    pub fn new() -> Self {
        WallCoveringConfigScript {
            component: ComponentBase::new(ComponentType::WallCoveringConfig),
            type_: None,
            name: None,
            sign: None,
        }
    }

    /// `getType`.
    pub fn covering_type(&self) -> Option<WallCoveringType> {
        self.type_
    }

    /// `getTypeString`.
    pub fn type_string(&self) -> ScriptResult<&'static str> {
        self.type_.map(WallCoveringType::type_string).ok_or_else(null_pointer)
    }

    /// `getName`: this class overrides `ComponentScript.getName` with its own `name` value.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The `sign` value.
    pub fn sign(&self) -> Option<SignType> {
        self.sign
    }

    /// `getSignIndex`.
    pub fn sign_index(&self) -> ScriptResult<i32> {
        self.sign.map(SignType::sprite_index).ok_or_else(null_pointer)
    }

    /// `getSignSpriteName`.
    pub fn sign_sprite_name(&self) -> ScriptResult<String> {
        self.sign.map(SignType::sprite_name).ok_or_else(null_pointer)
    }
}

impl ComponentScript for WallCoveringConfigScript {
    component_accessors!("zombie.scripting.entity.components.crafting.WallCoveringConfigScript");

    fn copy_from(&mut self, _other: &dyn ComponentScript) -> ScriptResult {
        Ok(())
    }

    fn load(&mut self, _sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "type") {
                self.type_ = Some(WallCoveringType::type_of(val)?);
            } else if java::eq_ignore_case(key, "name") {
                self.name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "sign") {
                self.sign = Some(SignType::type_of(parse_int(val)?)?);
            }
        }
        Ok(())
    }
}
