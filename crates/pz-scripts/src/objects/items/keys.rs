//! The registry key classes items refer to (`ItemType`, `ItemTag`,
//! `ItemBodyLocation`, `AmmoType`, `WeaponCategory`, `BookSubject`,
//! `MagazineSubject`), the character ones (`CharacterTrait`,
//! `CharacterProfession`) and `GameMode`: thin accessors over `sm.registries`.
//!
//! The original names some built-in keys through static fields
//! (`ItemType.FOOD`, `ItemTag.FAKE_WEAPON`, ...). Their ids are read from the
//! installed game's key classes by [`load_builtin_constants`], so code can
//! refer to a key by its Java field name without the repository carrying
//! the game's ids.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use pz_classfile::{load_class, static_inits, Jar, Value};

use crate::base::{ScriptError, ScriptResult};
use crate::manager::ScriptManager;
use crate::registry::builtin::KEY_CLASSES;
use crate::registry::{create_location, RegistryKey, ResourceLocation};

/// `Java field name -> registered id` for the built-in keys of each
/// registry, as the key classes' static initializers declare them.
#[derive(Clone, Debug, Default)]
pub struct BuiltinConstants {
    ids: HashMap<(String, String), String>,
    /// `(registry, field)` in declaration order.
    order: Vec<(String, String)>,
}

impl BuiltinConstants {
    /// Reads the `registerBase` constants of every key class in
    /// [`KEY_CLASSES`] from `jar`.
    pub fn read(jar: &Jar) -> ScriptResult<Self> {
        let err = |e: pz_classfile::Error| ScriptError::new(format!("projectzomboid.jar: {e}"));
        let mut out = BuiltinConstants::default();
        for (registry, class) in KEY_CLASSES {
            let cf = load_class(jar, class).map_err(err)?;
            for init in static_inits(&cf).map_err(err)? {
                let Value::Call { name, args, .. } = &init.value else {
                    continue;
                };
                if name != "registerBase" {
                    continue;
                }
                if let Some(id) = args.first().and_then(Value::as_str) {
                    out.insert(registry, &init.field, id);
                }
            }
        }
        Ok(out)
    }

    /// Adds one constant (for callers that build the table themselves).
    pub fn insert(&mut self, registry: &str, field: &str, id: &str) {
        let key = (registry.to_string(), field.to_string());
        if self.ids.insert(key.clone(), id.to_string()).is_none() {
            self.order.push(key);
        }
    }

    /// `(field, id)` of every constant of `registry`, in declaration order.
    pub fn constants(&self, registry: &str) -> Vec<(&str, &str)> {
        self.order
            .iter()
            .filter(|(r, _)| r == registry)
            .map(|k| (k.1.as_str(), self.ids[k].as_str()))
            .collect()
    }

    /// The id the key class's static field `field` registers.
    pub fn id(&self, registry: &str, field: &str) -> Option<&str> {
        self.ids
            .get(&(registry.to_string(), field.to_string()))
            .map(String::as_str)
    }
}

/// Reads the built-in key constants from the installed `projectzomboid.jar`
/// into this group's statics (alongside `ScriptManager.load_builtin_registries`).
pub fn load_builtin_constants(sm: &ScriptManager, jar: &Path) -> ScriptResult {
    let jar = Jar::open(jar).map_err(|e| ScriptError::new(format!("{}: {e}", jar.display())))?;
    sm.statics.items.borrow_mut().constants = BuiltinConstants::read(&jar)?;
    Ok(())
}

fn registry_get(
    sm: &ScriptManager,
    registry: &str,
    id: &ResourceLocation,
) -> Option<Rc<RegistryKey>> {
    sm.registries
        .get(registry)
        .expect("known registry")
        .borrow()
        .get(id)
}

fn registry_constant(sm: &ScriptManager, registry: &str, field: &str) -> Option<Rc<RegistryKey>> {
    let id = sm
        .statics
        .items
        .borrow()
        .constants
        .id(registry, field)?
        .to_string();
    registry_get(sm, registry, &ResourceLocation::of(&id).ok()?)
}

fn registry_values(sm: &ScriptManager, registry: &str) -> Vec<Rc<RegistryKey>> {
    sm.registries
        .get(registry)
        .expect("known registry")
        .borrow()
        .values()
        .to_vec()
}

macro_rules! registry_key {
    ($(#[$m:meta])* $name:ident, $registry:literal) => {
        $(#[$m])*
        pub struct $name;

        impl $name {
            pub const REGISTRY: &'static str = $registry;

            /// `get(ResourceLocation)`.
            pub fn get(sm: &ScriptManager, id: &ResourceLocation) -> Option<Rc<RegistryKey>> {
                registry_get(sm, $registry, id)
            }

            /// `get(ResourceLocation.of(id))`, the form script loaders use.
            pub fn get_id(sm: &ScriptManager, id: &str) -> ScriptResult<Option<Rc<RegistryKey>>> {
                Ok(Self::get(sm, &ResourceLocation::of(id)?))
            }

            /// The built-in key the class's static field `field` holds.
            pub fn constant(sm: &ScriptManager, field: &str) -> Option<Rc<RegistryKey>> {
                registry_constant(sm, $registry, field)
            }

            /// `Registries.<X>.values()`: registration order.
            pub fn values(sm: &ScriptManager) -> Vec<Rc<RegistryKey>> {
                registry_values(sm, $registry)
            }

            /// `register(id)`: a mod key, outside the `base` namespace.
            pub fn register(sm: &ScriptManager, id: &str) -> ScriptResult<Rc<RegistryKey>> {
                sm.registries.get($registry).expect("known registry").borrow_mut().register_id(id, false)
            }
        }
    };
}

registry_key!(
    /// `zombie.scripting.objects.ItemType`.
    ItemType,
    "item_type"
);
registry_key!(
    /// `zombie.scripting.objects.ItemTag`.
    ItemTag,
    "item_tag"
);
registry_key!(
    /// `zombie.scripting.objects.ItemBodyLocation`.
    ItemBodyLocation,
    "item_body_location"
);
registry_key!(
    /// `zombie.scripting.objects.AmmoType`.
    AmmoType,
    "ammo_type"
);
registry_key!(
    /// `zombie.scripting.objects.WeaponCategory`.
    WeaponCategory,
    "weapon_category"
);
registry_key!(
    /// `zombie.scripting.objects.BookSubject`.
    BookSubject,
    "book_subject"
);
registry_key!(
    /// `zombie.scripting.objects.MagazineSubject`.
    MagazineSubject,
    "magazine_subject"
);

registry_key!(
    /// `zombie.scripting.objects.CharacterTrait`.
    CharacterTrait,
    "character_trait"
);
registry_key!(
    /// `zombie.scripting.objects.CharacterProfession`.
    CharacterProfession,
    "character_profession"
);

registry_key!(
    /// `zombie.core.GameMode`.
    GameMode,
    "game_mode"
);

impl GameMode {
    /// `getTitle`: `UI_NewGame_<id>`.
    pub fn title(key: &RegistryKey) -> String {
        format!("UI_NewGame_{}", key.name)
    }

    /// `getDescription`: the title with `_desc`.
    pub fn description(key: &RegistryKey) -> String {
        format!("{}_desc", Self::title(key))
    }

    /// `getThumbnail`.
    pub fn thumbnail(key: &RegistryKey) -> String {
        format!("media/ui/playstyleIcons/{}.png", key.name)
    }

    /// `getVideo`.
    pub fn video(key: &RegistryKey) -> String {
        format!("{}.bik", key.name)
    }
}

impl ItemType {
    /// `getTranslationName`.
    pub fn translation_name(key: &RegistryKey) -> String {
        format!("IGUI_ItemType_{}", key.name)
    }
}

impl ItemTag {
    /// `getTranslationName`.
    pub fn translation_name(key: &RegistryKey) -> &str {
        &key.name
    }
}

impl ItemBodyLocation {
    /// `getTranslationName`.
    pub fn translation_name(key: &RegistryKey) -> &str {
        &key.name
    }
}

impl WeaponCategory {
    /// `getTranslationName`.
    pub fn translation_name(key: &RegistryKey) -> &str {
        &key.name
    }
}

impl MagazineSubject {
    /// `key()`.
    pub fn key(key: &RegistryKey) -> &str {
        &key.name
    }
}

impl AmmoType {
    /// `getTranslationName`.
    pub fn translation_name(key: &RegistryKey) -> &str {
        &key.name
    }

    /// `getItemKey`.
    pub fn item_key(key: &RegistryKey) -> Option<&str> {
        key.item_key.as_deref()
    }

    /// `getByItemKey`: the first ammo type (registration order) whose item
    /// key equals `item_key`.
    pub fn get_by_item_key(sm: &ScriptManager, item_key: &str) -> Option<Rc<RegistryKey>> {
        Self::values(sm)
            .into_iter()
            .find(|a| a.item_key.as_deref() == Some(item_key))
    }

    /// `register(id, itemKey)`.
    pub fn register_with_item(
        sm: &ScriptManager,
        id: &str,
        item_key: &str,
    ) -> ScriptResult<Rc<RegistryKey>> {
        let loc = create_location(id, false)?;
        sm.registries
            .get(Self::REGISTRY)
            .expect("known registry")
            .borrow_mut()
            .register(loc, id, Some(item_key))
    }
}

/// Whether `key` is the built-in constant `field` of `registry`
/// (`item.itemType == ItemType.FOOD` and the like).
pub(crate) fn is_constant(
    sm: &ScriptManager,
    key: Option<&Rc<RegistryKey>>,
    registry: &str,
    field: &str,
) -> bool {
    match (key, registry_constant(sm, registry, field)) {
        (Some(k), Some(c)) => k.location == c.location,
        _ => false,
    }
}
