//! Actual class-global ItemTag/ItemType registries and final field owners.
//! A production ScriptManager borrows these same globals across reloads;
//! the ordinary constructor remains an independent offline data profile.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::registry::{Registry, RegistryKey};
use crate::ScriptError;
use pz_classfile::{load_class, static_inits, Jar, StaticInit, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptAccessError {
    Borrowed(&'static str),
    Unsupported(&'static str),
    Native(ScriptError),
}
impl std::fmt::Display for ScriptAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ScriptAccessError {}
pub type NativeKeyError = ScriptAccessError;
pub type NativeItemKeyStaticsRef = Rc<NativeItemKeyStatics>;
type RegistryRef = Rc<RefCell<Registry>>;

pub struct NativeItemKeyStatics {
    tags: RegistryRef,
    types: RegistryRef,
    /// These are the physical public static final fields, not registry lookups.
    fields: HashMap<(String, String), Rc<RegistryKey>>,
}
impl NativeItemKeyStatics {
    /// Decode physical class initializer fields for an explicitly supplied
    /// class-loader profile. Production uses native_item_key_statics below.
    pub fn from_class_initializers(
        tag: &[StaticInit],
        kind: &[StaticInit],
    ) -> Result<Self, NativeKeyError> {
        let mut out = Self {
            tags: Rc::new(RefCell::new(Registry::native("item_tag"))),
            types: Rc::new(RefCell::new(Registry::native("item_type"))),
            fields: HashMap::new(),
        };
        for (registry, class, initializers) in [
            ("item_tag", "zombie/scripting/objects/ItemTag", tag),
            ("item_type", "zombie/scripting/objects/ItemType", kind),
        ] {
            for init in initializers {
                let key = match &init.value {
                    Value::Call { name, args, .. } if name == "registerBase" => {
                        let id = args.first().and_then(Value::as_str).ok_or(
                            NativeKeyError::Unsupported("unresolved native item key initializer"),
                        )?;
                        out.registry(registry)
                            .unwrap()
                            .borrow_mut()
                            .register_id(id, true)
                            .map_err(NativeKeyError::Native)?
                    }
                    Value::Static {
                        class: source_class,
                        name,
                    } if source_class == class => {
                        Rc::clone(out.fields.get(&(registry.to_owned(), name.clone())).ok_or(
                            NativeKeyError::Unsupported("unresolved native item key alias"),
                        )?)
                    }
                    // Non-key class statics (loggers/registry access) are not fields of T.
                    _ => continue,
                };
                out.fields
                    .insert((registry.to_owned(), init.field.clone()), key);
            }
        }
        for (registry, field) in [
            ("item_tag", "NO_DETECT"),
            ("item_tag", "SHOW_POISON"),
            ("item_type", "FOOD"),
        ] {
            if out.constant(registry, field).is_none() {
                return Err(NativeKeyError::Unsupported(
                    "missing required native item key static",
                ));
            }
        }
        Ok(out)
    }
    pub fn registry(&self, registry: &str) -> Option<&RegistryRef> {
        match registry {
            "item_tag" => Some(&self.tags),
            "item_type" => Some(&self.types),
            _ => None,
        }
    }
    pub fn constant(&self, registry: &str, field: &str) -> Option<&Rc<RegistryKey>> {
        self.fields.get(&(registry.to_owned(), field.to_owned()))
    }
    pub fn constants<'a>(
        &'a self,
        registry: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a Rc<RegistryKey>)> + 'a {
        self.fields
            .iter()
            .filter(move |((r, _), _)| r == registry)
            .map(|((_, field), key)| (field.as_str(), key))
    }
    pub fn try_reset(&self) -> Result<(), ScriptAccessError> {
        // Native resets registries sequentially. A host borrow failure preserves
        // any already completed reset; it is never reported as a native exception.
        self.tags
            .try_borrow_mut()
            .map_err(|_| ScriptAccessError::Borrowed("ItemTag registry reset"))?
            .reset();
        self.types
            .try_borrow_mut()
            .map_err(|_| ScriptAccessError::Borrowed("ItemType registry reset"))?
            .reset();
        Ok(())
    }
}

struct LoadedClassGlobals {
    tag_bytes: Vec<u8>,
    type_bytes: Vec<u8>,
    globals: NativeItemKeyStaticsRef,
}
thread_local! { static GLOBALS: RefCell<Option<LoadedClassGlobals>> = const { RefCell::new(None) }; }

/// Resolves the actual installed class initialization once. Different class
/// bytes within the same live process require an explicit class-loader model;
/// replacing globals behind retained items would fabricate static identity.
pub fn native_item_key_statics(jar: &Jar) -> Result<NativeItemKeyStaticsRef, NativeKeyError> {
    let class_error = |_| NativeKeyError::Unsupported("unreadable native item key classes");
    let tag_bytes = jar
        .read("zombie/scripting/objects/ItemTag.class")
        .map_err(class_error)?;
    let type_bytes = jar
        .read("zombie/scripting/objects/ItemType.class")
        .map_err(class_error)?;
    GLOBALS.with(|slot| {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| NativeKeyError::Borrowed("native item key globals"))?;
        if let Some(loaded) = slot.as_ref() {
            if loaded.tag_bytes != tag_bytes || loaded.type_bytes != type_bytes {
                return Err(NativeKeyError::Unsupported(
                    "different live native item key class definitions",
                ));
            }
            return Ok(Rc::clone(&loaded.globals));
        }
        let tags = static_inits(
            &load_class(jar, "zombie/scripting/objects/ItemTag").map_err(class_error)?,
        )
        .map_err(class_error)?;
        let types = static_inits(
            &load_class(jar, "zombie/scripting/objects/ItemType").map_err(class_error)?,
        )
        .map_err(class_error)?;
        let globals = Rc::new(NativeItemKeyStatics::from_class_initializers(
            &tags, &types,
        )?);
        *slot = Some(LoadedClassGlobals {
            tag_bytes,
            type_bytes,
            globals: Rc::clone(&globals),
        });
        Ok(globals)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn initializer(class: &str, field: &str, id: &str) -> StaticInit {
        StaticInit {
            class: class.into(),
            field: field.into(),
            value: Value::Call {
                class: class.into(),
                name: "registerBase".into(),
                descriptor: String::new(),
                args: vec![Value::Str(id.into())],
            },
        }
    }
    fn globals() -> NativeItemKeyStaticsRef {
        Rc::new(
            NativeItemKeyStatics::from_class_initializers(
                &[
                    initializer("zombie/scripting/objects/ItemTag", "NO_DETECT", "noDetect"),
                    initializer(
                        "zombie/scripting/objects/ItemTag",
                        "SHOW_POISON",
                        "showPoison",
                    ),
                ],
                &[initializer(
                    "zombie/scripting/objects/ItemType",
                    "FOOD",
                    "Food",
                )],
            )
            .unwrap(),
        )
    }
    #[test]
    fn statics_and_registry_owners_survive_managers_and_duplicate_prefix() {
        let globals = globals();
        let first = crate::ScriptManager::with_native_item_key_statics(&globals);
        let second = crate::ScriptManager::with_native_item_key_statics(&globals);
        let original = Rc::clone(globals.constant("item_tag", "NO_DETECT").unwrap());
        let registry = globals.registry("item_tag").unwrap();
        let mut body = registry.borrow_mut();
        let before = body.values().len();
        assert!(body.register_id("noDetect", true).is_err());
        let replacement = body.get_id("noDetect").unwrap().unwrap();
        assert!(!Rc::ptr_eq(&replacement, &original));
        assert_eq!(body.values().len(), before + 1);
        drop(body);
        assert!(Rc::ptr_eq(
            crate::objects::items::ItemTag::constant(&first, "NO_DETECT")
                .as_ref()
                .unwrap(),
            &original
        ));
        assert!(Rc::ptr_eq(
            crate::objects::items::ItemTag::constant(&second, "NO_DETECT")
                .as_ref()
                .unwrap(),
            &original
        ));
        registry
            .borrow_mut()
            .register_id("mod:removed", false)
            .unwrap();
        first.try_reset_native_item_key_registries().unwrap();
        assert!(registry.borrow().get_id("mod:removed").unwrap().is_none());
        assert!(Rc::ptr_eq(
            globals.constant("item_tag", "NO_DETECT").unwrap(),
            &original
        ));
        let _held = registry.borrow_mut();
        assert!(Rc::ptr_eq(
            crate::objects::items::ItemTag::constant(&first, "NO_DETECT")
                .as_ref()
                .unwrap(),
            &original
        ));
        assert!(matches!(
            first.try_reset_native_item_key_registries(),
            Err(ScriptAccessError::Borrowed(_))
        ));
    }
    #[test]
    fn reset_preserves_completed_tag_prefix_when_later_type_body_is_held() {
        let globals = globals();
        let tags = globals.registry("item_tag").unwrap();
        let types = globals.registry("item_type").unwrap();
        tags.borrow_mut().register_id("mod:tag", false).unwrap();
        types.borrow_mut().register_id("mod:type", false).unwrap();
        let held = types.borrow_mut();
        assert!(matches!(
            globals.try_reset(),
            Err(ScriptAccessError::Borrowed("ItemType registry reset"))
        ));
        assert!(tags.borrow().get_id("mod:tag").unwrap().is_none());
        assert!(held.get_id("mod:type").unwrap().is_some());
        drop(held);
        globals.try_reset().unwrap();
        assert!(types.borrow().get_id("mod:type").unwrap().is_none());
    }
    #[test]
    fn actual_recipe_lookup_keeps_identity_and_held_import_is_not_missing_recipe() {
        let manager = crate::ScriptManager::new();
        manager
            .parse_script(crate::ScriptLoadMode::Init, "module Base {} ")
            .unwrap();
        let module = manager.get_module("Base").unwrap();
        let recipe = crate::ScriptRef::new(crate::objects::recipes::Recipe::new());
        recipe.borrow_mut().init_load_pp(&manager, "Herbalist");
        module
            .bucket(crate::ScriptType::Recipe)
            .add_script(recipe.clone());
        let actual = crate::objects::recipes::try_get_recipe(&manager, "Herbalist")
            .unwrap()
            .unwrap();
        assert!(Rc::ptr_eq(
            &actual,
            &recipe
                .downcast::<crate::objects::recipes::Recipe>()
                .unwrap()
        ));
        let held = module.imports.borrow_mut();
        assert!(
            crate::objects::recipes::try_get_recipe(&manager, "Herbalist")
                .unwrap()
                .is_some(),
            "local hit does not read imports"
        );
        assert!(matches!(
            crate::objects::recipes::try_get_recipe(&manager, "Missing"),
            Err(ScriptAccessError::Borrowed("script module imports"))
        ));
        drop(held);
        assert!(crate::objects::recipes::try_get_recipe(&manager, "Missing")
            .unwrap()
            .is_none());
    }
    #[test]
    fn tags_and_types_compare_actual_identity_even_with_same_location() {
        let globals = globals();
        let tag = globals.constant("item_tag", "NO_DETECT").unwrap();
        let shadow = Rc::new(RegistryKey {
            location: tag.location.clone(),
            name: tag.name.clone(),
            item_key: None,
        });
        let mut item = crate::objects::items::Item::new();
        item.item_tags.add(Some(Rc::clone(tag)));
        assert!(item.has_tag(Some(tag)));
        assert!(!item.has_tag(Some(&shadow)));
        item.item_tags.add(Some(shadow));
        assert_eq!(item.item_tags.len(), 2);
        let food = globals.constant("item_type", "FOOD").unwrap();
        item.item_type = Some(Rc::clone(food));
        assert!(item.is_item_type_key(Some(food)));
        let shadow = Rc::new(RegistryKey {
            location: food.location.clone(),
            name: food.name.clone(),
            item_key: None,
        });
        assert!(!item.is_item_type_key(Some(&shadow)));
    }
}
