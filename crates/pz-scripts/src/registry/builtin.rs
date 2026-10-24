//! The game's built-in registry keys (`registerBase` calls in each key
//! class's static initializer), read from the installed
//! `projectzomboid.jar` with `pz-classfile`. Nothing is shipped: the ids come
//! from the user's own copy of the game at runtime.

use std::collections::HashMap;

use pz_classfile::{load_class, static_inits, Jar, StaticInit, Value};

use super::Registries;
use crate::base::{ScriptError, ScriptResult};

/// Registry name -> key class whose initializer registers its base keys.
pub const KEY_CLASSES: &[(&str, &str)] = &[
    ("ammo_type", "zombie/scripting/objects/AmmoType"),
    ("book_subject", "zombie/scripting/objects/BookSubject"),
    ("character_profession", "zombie/scripting/objects/CharacterProfession"),
    ("character_trait", "zombie/scripting/objects/CharacterTrait"),
    ("item_body_location", "zombie/scripting/objects/ItemBodyLocation"),
    ("item_tag", "zombie/scripting/objects/ItemTag"),
    ("item_type", "zombie/scripting/objects/ItemType"),
    ("magazine_subject", "zombie/scripting/objects/MagazineSubject"),
    ("weapon_category", "zombie/scripting/objects/WeaponCategory"),
];

fn class_error(e: pz_classfile::Error) -> ScriptError {
    ScriptError::new(format!("projectzomboid.jar: {e}"))
}

/// Resolves `ItemKey.X.CONST` to `ItemKey.toString()` (`Base.<id>`).
struct ItemKeys<'a> {
    jar: &'a Jar,
    classes: HashMap<String, Vec<StaticInit>>,
}

impl ItemKeys<'_> {
    fn resolve(&mut self, class: &str, field: &str) -> ScriptResult<Option<String>> {
        if !self.classes.contains_key(class) {
            let cf = load_class(self.jar, class).map_err(class_error)?;
            self.classes.insert(class.to_string(), static_inits(&cf).map_err(class_error)?);
        }
        let inits = &self.classes[class];
        Ok(inits.iter().find(|s| s.field == field).and_then(|s| match &s.value {
            Value::Call { args, .. } | Value::New { args, .. } => args.first().and_then(Value::as_str).map(|id| format!("Base.{id}")),
            _ => None,
        }))
    }
}

/// Registers every built-in key of [`KEY_CLASSES`] into `registries`, in the
/// order the original's static initializers run.
pub fn register_builtin(registries: &Registries, jar: &Jar) -> ScriptResult {
    let mut item_keys = ItemKeys { jar, classes: HashMap::new() };
    for (registry, class) in KEY_CLASSES {
        let cf = load_class(jar, class).map_err(class_error)?;
        let mut reg = registries.get(registry).expect("known registry").borrow_mut();
        for init in static_inits(&cf).map_err(class_error)? {
            let Value::Call { name, args, .. } = &init.value else { continue };
            if name != "registerBase" {
                continue;
            }
            let Some(id) = args.first().and_then(Value::as_str) else {
                return Err(ScriptError::new(format!("{class}.{}: unreadable registerBase id", init.field)));
            };
            let item_key = match args.get(1) {
                Some(Value::Static { class, name }) => item_keys.resolve(class, name)?,
                _ => None,
            };
            let loc = super::create_location(id, true)?;
            reg.register(loc, id, item_key.as_deref())?;
        }
    }
    Ok(())
}
