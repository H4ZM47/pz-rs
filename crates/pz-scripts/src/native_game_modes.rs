//! Class-global GameMode registry and the actual owning static key fields.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_classfile::{load_class, static_inits, Jar, StaticInit, Value};

use crate::native_item_keys::ScriptAccessError;
use crate::registry::{Registry, RegistryKey, ResourceLocation};

pub type NativeGameModeStaticsRef = Rc<NativeGameModeStatics>;

pub struct NativeGameModeStatics {
    registry: Rc<RefCell<Registry>>,
    fields: HashMap<String, Rc<RegistryKey>>,
}

impl NativeGameModeStatics {
    pub fn from_class_initializers(initializers: &[StaticInit]) -> Result<Self, ScriptAccessError> {
        let mut out = Self {
            registry: Rc::new(RefCell::new(Registry::native("game_mode"))),
            fields: HashMap::new(),
        };
        for init in initializers {
            let key = match &init.value {
                Value::Call { name, args, .. } if name == "registerBase" => {
                    let id = args.first().and_then(Value::as_str).ok_or(
                        ScriptAccessError::Unsupported("unresolved GameMode initializer"),
                    )?;
                    out.registry
                        .borrow_mut()
                        .register_id(id, true)
                        .map_err(ScriptAccessError::Native)?
                }
                Value::Static { class, name } if class == "zombie/core/GameMode" => out
                    .fields
                    .get(name)
                    .cloned()
                    .ok_or(ScriptAccessError::Unsupported(
                        "unresolved GameMode static alias",
                    ))?,
                _ => continue,
            };
            out.fields.insert(init.field.clone(), key);
        }
        for field in ["SANDBOX", "TUTORIAL"] {
            if !out.fields.contains_key(field) {
                return Err(ScriptAccessError::Unsupported(
                    "missing required GameMode static",
                ));
            }
        }
        Ok(out)
    }

    pub fn registry(&self) -> &Rc<RefCell<Registry>> {
        &self.registry
    }
    pub fn constant(&self, field: &str) -> Option<&Rc<RegistryKey>> {
        self.fields.get(field)
    }
    pub fn constants(&self) -> impl Iterator<Item = (&str, &Rc<RegistryKey>)> {
        self.fields.iter().map(|(field, key)| (field.as_str(), key))
    }

    pub fn get_id(&self, id: &str) -> Result<Option<Rc<RegistryKey>>, ScriptAccessError> {
        let location = ResourceLocation::of(id).map_err(ScriptAccessError::Native)?;
        Ok(self
            .registry
            .try_borrow()
            .map_err(|_| ScriptAccessError::Borrowed("GameMode registry lookup"))?
            .get(&location))
    }
}

struct Loaded {
    bytes: Vec<u8>,
    statics: NativeGameModeStaticsRef,
}
thread_local! { static GLOBALS: RefCell<Option<Loaded>> = const { RefCell::new(None) }; }

/// Decode the installed class once; Lua resets retain these same allocations.
pub fn native_game_mode_statics(jar: &Jar) -> Result<NativeGameModeStaticsRef, ScriptAccessError> {
    let unavailable = |_| ScriptAccessError::Unsupported("unreadable GameMode class");
    let bytes = jar
        .read("zombie/core/GameMode.class")
        .map_err(unavailable)?;
    GLOBALS.with(|slot| {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| ScriptAccessError::Borrowed("GameMode class globals"))?;
        if let Some(loaded) = slot.as_ref() {
            if loaded.bytes != bytes {
                return Err(ScriptAccessError::Unsupported(
                    "different live GameMode class definition",
                ));
            }
            return Ok(loaded.statics.clone());
        }
        let class = load_class(jar, "zombie/core/GameMode").map_err(unavailable)?;
        let initializers = static_inits(&class).map_err(unavailable)?;
        let statics = Rc::new(NativeGameModeStatics::from_class_initializers(
            &initializers,
        )?);
        *slot = Some(Loaded {
            bytes,
            statics: statics.clone(),
        });
        Ok(statics)
    })
}

#[cfg(test)]
mod tests;
