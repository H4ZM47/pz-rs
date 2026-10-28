//! `zombie.scripting.objects.Registry`, `Registries`, `RegistryReset` and
//! `ResourceLocation`: namespaced ids for engine-defined keys (item types,
//! tags, body locations, traits, ...) that scripts and mods refer to.
//!
//! The original's key classes (`ItemTag`, `CharacterTrait`, ...) are thin
//! objects with identity equality. Runtime item-tag/type class statics retain
//! the same actual keys independently of their mutable registry maps.

pub mod builtin;

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::base::{ScriptError, ScriptResult};
use crate::java::{self, JHashMap};

pub const DEFAULT_NAMESPACE: &str = "base";

/// `ResourceLocation`: a lower-cased `namespace:path` pair.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceLocation {
    namespace: String,
    path: String,
}

impl ResourceLocation {
    pub fn new(namespace: &str, path: &str) -> ScriptResult<Self> {
        if namespace.is_empty() {
            return Err(illegal_argument("Namespace cannot be null or empty"));
        }
        if path.is_empty() {
            return Err(illegal_argument("Path cannot be null or empty"));
        }
        Ok(ResourceLocation { namespace: java::to_lower(namespace), path: java::to_lower(path) })
    }

    /// `ResourceLocation.of`: `ns:path`, or `path` in the `base` namespace.
    pub fn of(id: &str) -> ScriptResult<Self> {
        if id.is_empty() {
            return Err(illegal_argument("Identifier cannot be null or empty"));
        }
        match id.split_once(':') {
            None => Self::new(DEFAULT_NAMESPACE, id),
            Some((ns, path)) => Self::new(ns, path),
        }
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// `toString`: `namespace:path`.
    pub fn id(&self) -> String {
        format!("{}:{}", self.namespace, self.path)
    }

    /// `hashCode`: `Objects.hash(namespace, path)`.
    pub fn java_hash(&self) -> i32 {
        Self::hash_parts(&self.namespace, &self.path)
    }

    fn hash_parts(ns: &str, path: &str) -> i32 {
        31i32.wrapping_add(java::string_hash(ns)).wrapping_mul(31).wrapping_add(java::string_hash(path))
    }

    /// Hash of a location given as its `ns:path` id string.
    fn hash_id(id: &str) -> i32 {
        let (ns, path) = id.split_once(':').unwrap_or((DEFAULT_NAMESPACE, id));
        Self::hash_parts(ns, path)
    }
}

impl fmt::Display for ResourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.path)
    }
}

fn illegal_argument(msg: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.IllegalArgumentException: {msg}"))
}

/// `RegistryReset.createLocation`.
pub fn create_location(id: &str, allows_base_namespace: bool) -> ScriptResult<ResourceLocation> {
    let rl = ResourceLocation::of(id)?;
    if !allows_base_namespace && rl.namespace == DEFAULT_NAMESPACE {
        return Err(illegal_argument(&format!("Default namespace '{rl}' is not allowed!")));
    }
    Ok(rl)
}

/// One registered key. `name` is the id it was registered with (before
/// lower-casing), which the key classes use for their translation names;
/// `item_key` is `AmmoType`'s item.
#[derive(Debug, PartialEq, Eq)]
pub struct RegistryKey {
    pub location: ResourceLocation,
    pub name: String,
    pub item_key: Option<String>,
}

impl RegistryKey {
    /// `getName()` of `CharacterTrait`/`CharacterProfession`: the path.
    pub fn path(&self) -> &str {
        self.location.path()
    }
}

impl fmt::Display for RegistryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.location.fmt(f)
    }
}

/// `Registry<T>`: registration-ordered `values()` plus a `HashMap` by
/// location, whose order the iterator follows.
pub struct Registry {
    name: String,
    by_location: JHashMap<Rc<RegistryKey>>,
    values: Vec<Rc<RegistryKey>>,
    /// Actual Registry.byObject owner slots, keyed by physical identity.
    by_object: std::collections::HashMap<usize, Rc<RegistryKey>>,
    native_identity: bool,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Registry{{{}, size={}}}", self.name, self.by_location.len())
    }
}

impl Registry {
    pub fn new(name: &str) -> Self {
        Registry { name: name.to_string(), by_location: JHashMap::with_hasher(ResourceLocation::hash_id), values: Vec::new(), by_object: Default::default(), native_identity: false }
    }

    pub(crate) fn native(name: &str) -> Self {
        Self { native_identity: true, ..Self::new(name) }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `register(id, t)`: duplicates are an error.
    pub fn register(
        &mut self,
        location: ResourceLocation,
        name: &str,
        item_key: Option<&str>,
    ) -> ScriptResult<Rc<RegistryKey>> {
        let id = location.id();
        let duplicate = self.by_location.contains_key(&id);
        if duplicate && !self.native_identity {
            return Err(illegal_argument(&format!("Tried to register duplicate object: {id} in: {}", self.name)));
        }
        let key = Rc::new(RegistryKey { location, name: name.to_string(), item_key: item_key.map(str::to_string) });
        self.by_object.insert(Rc::as_ptr(&key) as usize, key.clone());
        self.by_location.insert(id.clone(), key.clone());
        self.values.push(key.clone());
        // Native Registry.register mutates all three collections before it
        // throws for a duplicate. Previously retained static fields survive.
        if duplicate {
            return Err(illegal_argument(&format!("Tried to register duplicate object: {id} in: {}", self.name)));
        }
        Ok(key)
    }

    /// The key classes' `register(allowDefaultNamespace, id)`.
    pub fn register_id(&mut self, id: &str, allow_default_namespace: bool) -> ScriptResult<Rc<RegistryKey>> {
        let loc = create_location(id, allow_default_namespace)?;
        self.register(loc, id, None)
    }

    pub fn get(&self, location: &ResourceLocation) -> Option<Rc<RegistryKey>> {
        self.by_location.get(&location.id()).cloned()
    }

    /// `X.get(ResourceLocation.of(id))`, the form script loaders use.
    pub fn get_id(&self, id: &str) -> ScriptResult<Option<Rc<RegistryKey>>> {
        Ok(self.get(&ResourceLocation::of(id)?))
    }

    pub fn contains(&self, location: &ResourceLocation) -> bool {
        self.by_location.contains_key(&location.id())
    }

    /// `values()`: registration order.
    pub fn values(&self) -> &[Rc<RegistryKey>] {
        &self.values
    }

    /// `iterator()`: `HashMap` order of the locations.
    pub fn iter(&self) -> impl Iterator<Item = &Rc<RegistryKey>> {
        self.by_location.values()
    }

    /// `reset()`: drops everything outside the `base` namespace.
    pub fn reset(&mut self) {
        let removed: Vec<String> = self
            .by_location
            .iter()
            .filter(|(_, k)| k.location.namespace != DEFAULT_NAMESPACE)
            .map(|(id, _)| id.to_string())
            .collect();
        for id in removed {
            self.by_location.remove(&id);
        }
        self.by_object.retain(|_, key| key.location.namespace == DEFAULT_NAMESPACE);
        self.values.retain(|k| k.location.namespace == DEFAULT_NAMESPACE);
    }
}

/// `Registries`: the registry of registries, in declaration order.
pub const REGISTRY_NAMES: &[&str] = &[
    "ammo_type",
    "book",
    "book_subject",
    "brochure",
    "business",
    "catalogue",
    "character_profession",
    "character_trait",
    "comic_book",
    "doodle",
    "doodle_kids",
    "flier",
    "game_mode",
    "genericMail",
    "locket",
    "tile_property_key",
    "item_body_location",
    "item_tag",
    "item_type",
    "job",
    "magazine",
    "magazine_subject",
    "meta_recipe",
    "moodle_type",
    "newspaper",
    "old_newspaper",
    "pet_name",
    "photo",
    "postcard",
    "rpg_magazine",
    "season_recipe",
    "sound_key",
    "survival_guide_entry",
    "weapon_category",
    "container_type",
    "keybind_id",
];

/// All registries by name. The game's built-in keys are registered by
/// whoever provides them; mods add theirs from `media/registries.lua`
/// (`ModRegistries.init`).
pub struct Registries {
    registries: Vec<Rc<RefCell<Registry>>>,
}

impl Default for Registries {
    fn default() -> Self {
        Registries { registries: REGISTRY_NAMES.iter().map(|n| Rc::new(RefCell::new(Registry::new(n)))).collect() }
    }
}

impl Registries {
    pub(crate) fn replace_ref(&mut self, name: &str, registry: &Rc<RefCell<Registry>>) {
        let index = REGISTRY_NAMES.iter().position(|n| *n == name).expect("known registry");
        self.registries[index] = Rc::clone(registry);
    }

    pub fn get(&self, name: &str) -> Option<&RefCell<Registry>> {
        let i = REGISTRY_NAMES.iter().position(|n| *n == name)?;
        Some(self.registries[i].as_ref())
    }

    pub fn all(&self) -> &[Rc<RefCell<Registry>>] {
        &self.registries
    }

    /// `RegistryReset.resetAll`.
    pub fn reset_all(&self) {
        for r in &self.registries {
            r.borrow_mut().reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_lower_case_and_default_namespace() {
        let a = ResourceLocation::of("SomeTag").unwrap();
        assert_eq!(a.id(), "base:sometag");
        let b = ResourceLocation::of("MyMod:Thing:x").unwrap();
        assert_eq!((b.namespace(), b.path()), ("mymod", "thing:x"));
        assert!(ResourceLocation::of("").is_err());
        assert!(ResourceLocation::of("ns:").is_err());
        assert!(create_location("Thing", false).is_err());
        assert!(create_location("mod:Thing", false).is_ok());
    }

    #[test]
    fn hash_matches_objects_hash() {
        let l = ResourceLocation::of("a:b").unwrap();
        assert_eq!(l.java_hash(), (31 + 97) * 31 + 98);
        assert_eq!(ResourceLocation::hash_id("a:b"), l.java_hash());
    }

    #[test]
    fn registry_rejects_duplicates_and_resets_mod_keys() {
        let mut r = Registry::new("item_tag");
        r.register_id("Alpha", true).unwrap();
        r.register_id("mod:beta", false).unwrap();
        assert!(r.register_id("ALPHA", true).is_err());
        assert!(r.get_id("alpha").unwrap().is_some());
        assert_eq!(r.values()[0].name, "Alpha");
        r.reset();
        assert_eq!(r.values().len(), 1);
        assert!(r.get_id("mod:beta").unwrap().is_none());
    }
}
