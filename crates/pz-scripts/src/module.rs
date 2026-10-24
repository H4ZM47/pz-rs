//! `zombie.scripting.objects.ScriptModule`: one `module Name { ... }`
//! namespace with a bucket per script type.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use crate::base::{ScriptError, ScriptRef, ScriptResult};
use crate::bucket::ScriptBucket;
use crate::java;
use crate::manager::ScriptManager;
use crate::parser;
use crate::script_type::{ScriptLoadMode, ScriptType};

/// `ScriptModule.scriptBucketList` order (field declaration order). Token
/// dispatch tries buckets in this order and stops at the first match.
pub const MODULE_BUCKET_ORDER: &[ScriptType] = &[
    ScriptType::VehicleTemplate,
    ScriptType::EntityTemplate,
    ScriptType::Item,
    ScriptType::Recipe,
    ScriptType::UniqueRecipe,
    ScriptType::EvolvedRecipe,
    ScriptType::Fixing,
    ScriptType::AnimationMesh,
    ScriptType::Clock,
    ScriptType::Mannequin,
    ScriptType::Model,
    ScriptType::PhysicsShape,
    ScriptType::SpriteModel,
    ScriptType::Sound,
    ScriptType::SoundTimeline,
    ScriptType::Vehicle,
    ScriptType::RuntimeAnimation,
    ScriptType::VehicleEngineRPM,
    ScriptType::ItemConfig,
    ScriptType::Entity,
    ScriptType::XuiConfig,
    ScriptType::XuiLayout,
    ScriptType::XuiStyle,
    ScriptType::XuiDefaultStyle,
    ScriptType::XuiColor,
    ScriptType::XuiSkin,
    ScriptType::ItemFilter,
    ScriptType::FluidFilter,
    ScriptType::CraftRecipe,
    ScriptType::StringList,
    ScriptType::EnergyDefinition,
    ScriptType::FluidDefinition,
    ScriptType::TimedAction,
    ScriptType::Ragdoll,
    ScriptType::CharacterTraitDefinition,
    ScriptType::CharacterProfessionDefinition,
    ScriptType::PhysicsHitReaction,
];

pub struct ScriptModule {
    name: RefCell<String>,
    value: RefCell<String>,
    pub imports: RefCell<Vec<String>>,
    pub disabled: Cell<bool>,
    buckets: Vec<ScriptBucket>,
    weak_self: Weak<ScriptModule>,
}

impl ScriptModule {
    pub fn new() -> Rc<ScriptModule> {
        Rc::new_cyclic(|weak_self| {
            let buckets: Vec<ScriptBucket> = MODULE_BUCKET_ORDER.iter().map(|&t| ScriptBucket::new(t)).collect();
            let m = ScriptModule {
                name: RefCell::new(String::new()),
                value: RefCell::new(String::new()),
                imports: RefCell::new(Vec::new()),
                disabled: Cell::new(false),
                buckets,
                weak_self: weak_self.clone(),
            };
            m.bucket(ScriptType::XuiLayout).set_verbose(false);
            m
        })
    }

    fn rc(&self) -> Rc<ScriptModule> {
        self.weak_self.upgrade().expect("module dropped")
    }

    /// `getName`.
    pub fn name(&self) -> String {
        self.name.borrow().clone()
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    pub fn buckets(&self) -> &[ScriptBucket] {
        &self.buckets
    }

    pub fn bucket(&self, type_: ScriptType) -> &ScriptBucket {
        self.buckets
            .iter()
            .find(|b| b.script_type() == type_)
            .unwrap_or_else(|| panic!("no bucket for {}", type_.name()))
    }

    /// `getItem`, `getRecipe`, `getModelScript`, ...: bucket lookup by name.
    pub fn get(&self, sm: &ScriptManager, type_: ScriptType, name: &str) -> Option<ScriptRef> {
        self.bucket(type_).get(sm, self, name)
    }

    /// `Load(loadMode, name, body)`.
    /// Errors are exceptions the original lets escape to `ScriptManager.Load`.
    pub fn load(&self, sm: &ScriptManager, mode: ScriptLoadMode, name: &str, body: &str) -> ScriptResult {
        *self.name.borrow_mut() = name.to_string();
        *self.value.borrow_mut() = java::trim(body).to_string();
        sm.set_current_loading_module(Some(self.rc()));
        let value = self.value.borrow().clone();
        self.parse_script_pp(sm, mode, &value)?;
        self.value.borrow_mut().clear();
        Ok(())
    }

    pub fn parse_script_pp(&self, sm: &ScriptManager, mode: ScriptLoadMode, total_file: &str) -> ScriptResult {
        for token in parser::parse_tokens(total_file) {
            self.create_from_token_pp(sm, mode, &token)?;
        }
        Ok(())
    }

    /// `GetTokenType`: the header word before the first space or tab.
    fn get_token_type(token: &str) -> Option<&str> {
        let p = token.find('{')?;
        let header = java::trim(&token[..p]);
        let p1 = header.find(' ');
        let p2 = header.find('\t');
        Some(match (p1, p2) {
            (Some(a), Some(b)) => &header[..a.min(b)],
            (Some(a), None) => &header[..a],
            (None, Some(b)) => &header[..b],
            (None, None) => header,
        })
    }

    fn create_from_token_pp(&self, sm: &ScriptManager, mode: ScriptLoadMode, token: &str) -> ScriptResult {
        let token = java::trim(token);
        let Some(type_) = Self::get_token_type(token) else { return Ok(()) };
        if type_ == "imports" {
            let waypoint = java::split(token, |c| c == '{' || c == '}', false);
            // `waypoint[1]` throws when the block is empty (`imports {}`).
            let list = waypoint.get(1).ok_or_else(|| {
                ScriptError::new(format!(
                    "java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length {}",
                    waypoint.len()
                ))
            })?;
            for coord in java::split_char(list, ',') {
                let module = java::trim(coord);
                if module.is_empty() {
                    continue;
                }
                if module == self.name() {
                    sm.script_error(&format!("ERROR: module \"{}\" imports itself", self.name()));
                    continue;
                }
                self.imports.borrow_mut().push(module.to_string());
            }
            return Ok(());
        }
        let me = self.rc();
        let found = self.buckets.iter().any(|b| b.create_from_token_pp(&me, sm, mode, type_, token));
        if !found {
            sm.script_warn(&format!(
                "unknown script object \"{type_}\" in '{}'",
                sm.current_file_name().as_deref().unwrap_or("null")
            ));
        }
        Ok(())
    }

    /// `CheckExitPoints`.
    pub fn check_exit_points(&self) -> bool {
        false
    }

    /// `Reset`.
    pub fn reset(&self) {
        self.imports.borrow_mut().clear();
        for b in &self.buckets {
            b.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_type_is_first_word() {
        assert_eq!(ScriptModule::get_token_type("item Axe { }"), Some("item"));
        assert_eq!(ScriptModule::get_token_type("template\tvehicle X { }"), Some("template"));
        assert_eq!(ScriptModule::get_token_type("imports{ Base }"), Some("imports"));
        assert_eq!(ScriptModule::get_token_type("trailing"), None);
    }

    #[test]
    fn every_type_but_entity_component_has_a_bucket() {
        assert_eq!(MODULE_BUCKET_ORDER.len(), ScriptType::ALL.len() - 1);
    }
}
