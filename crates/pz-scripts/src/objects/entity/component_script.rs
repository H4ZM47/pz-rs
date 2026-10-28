//! `zombie.scripting.entity.ComponentScript`: the abstract base of every
//! component block an entity script can hold.

use std::any::Any;
use std::fmt::Debug;
use std::rc::Rc;

use super::{runtime_exception, ComponentType};
use crate::base::{BaseScriptObject, ScriptResult};
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::parser::Block;
use crate::script_type::{ScriptLoadMode, ScriptType};

/// The fields `ComponentScript` adds to `BaseScriptObject`, plus its parent
/// link (`BaseScriptObject.parentScript`), kept as the parent entity's
/// script name since that is all components read from it.
#[derive(Debug)]
pub struct ComponentBase {
    pub base: BaseScriptObject,
    type_: ComponentType,
    /// `Some` once `setParent` ran; holds the parent's `getScriptObjectName()`.
    parent_name: Option<Option<String>>,
}

impl ComponentBase {
    /// `ComponentScript(ComponentType)`: also runs `InitLoadPP(type.toString())`.
    pub fn new(type_: ComponentType) -> Self {
        let mut base = BaseScriptObject::new(ScriptType::EntityComponent);
        base.init_load_pp(type_.name());
        ComponentBase { base, type_, parent_name: None }
    }

    /// The `type` field.
    pub fn component_type(&self) -> ComponentType {
        self.type_
    }

    /// `setParent`: also takes the parent's module.
    pub fn set_parent(&mut self, parent_name: Option<&str>, module: Option<&Rc<ScriptModule>>) {
        self.parent_name = Some(parent_name.map(str::to_string));
        if let Some(m) = module {
            self.base.set_module(m);
        }
    }

    pub fn has_parent(&self) -> bool {
        self.parent_name.is_some()
    }

    /// `getName`: the parent entity's name (`"null"` when it has none, as
    /// string concatenation prints it). Debug builds throw without a parent.
    pub fn name(&self, sm: &ScriptManager) -> ScriptResult<String> {
        match &self.parent_name {
            Some(Some(n)) => Ok(n.clone()),
            Some(None) if !sm.debug => Ok("null".to_string()),
            None if !sm.debug => Ok("UnknownScriptName".to_string()),
            _ => Err(runtime_exception("Parent is null or parent name is null.")),
        }
    }
}

/// The overridable methods of `ComponentScript` (and the `BaseScriptObject`
/// hooks entities forward to their components).
pub trait ComponentScript: Any + Debug {
    fn component(&self) -> &ComponentBase;
    fn component_mut(&mut self) -> &mut ComponentBase;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Java class name, used in diagnostics.
    fn class_name(&self) -> &'static str;

    fn component_type(&self) -> ComponentType {
        self.component().component_type()
    }

    /// `isoMasterOnly`.
    fn iso_master_only(&self) -> bool {
        true
    }

    /// `copyFrom`: called with a component of the same type.
    fn copy_from(&mut self, other: &dyn ComponentScript) -> ScriptResult;

    /// `load(Block)`.
    fn load(&mut self, _sm: &ScriptManager, _block: &Block) -> ScriptResult {
        Ok(())
    }

    /// `parseKeyValue`.
    fn parse_key_value(&mut self, _sm: &ScriptManager, _k: &str, _v: &str) -> ScriptResult<bool> {
        Err(runtime_exception(format!("'parseKeyValue' not implemented for class {}", self.class_name())))
    }

    /// `PreReload`.
    fn pre_reload(&mut self) {}

    /// `OnScriptsLoaded`.
    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        Ok(())
    }

    /// `OnLoadedAfterLua`.
    fn on_loaded_after_lua(&mut self, _sm: &ScriptManager) -> ScriptResult {
        Ok(())
    }

    /// `OnPostWorldDictionaryInit`.
    fn on_post_world_dictionary_init(&mut self, _sm: &ScriptManager) -> ScriptResult {
        Ok(())
    }
}

/// Implements the accessor methods of [`ComponentScript`] for a struct with
/// a `component: ComponentBase` field.
macro_rules! component_accessors {
    ($class:literal) => {
        fn component(&self) -> &$crate::objects::entity::ComponentBase {
            &self.component
        }
        fn component_mut(&mut self) -> &mut $crate::objects::entity::ComponentBase {
            &mut self.component
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn class_name(&self) -> &'static str {
            $class
        }
    };
}

/// The `(T) other` cast in `copyFrom`.
pub(crate) fn cast<'a, T: ComponentScript>(
    other: &'a dyn ComponentScript,
    this: &dyn ComponentScript,
) -> ScriptResult<&'a T> {
    other.as_any().downcast_ref::<T>().ok_or_else(|| {
        crate::base::ScriptError::new(format!(
            "java.lang.ClassCastException: class {} cannot be cast to class {}",
            other.class_name(),
            this.class_name()
        ))
    })
}
