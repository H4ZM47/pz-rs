//! What component loading needs from outside the entity: registries the
//! original reads through statics (`Attribute`, `Fluid`, `ScriptsDictionary`,
//! `ScriptManager`). Loading asks [`EntityContext`]; saving needs nothing.

use std::collections::HashMap;

use crate::fluids::FluidRegistry;
use crate::tables::AttributeTypes;

/// The answer to a registry lookup.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup<T> {
    /// The context has no such registry: keep what the data says (no
    /// validation, nothing dropped).
    Unknown,
    /// The registry has no such entry: the original's lookup returns null.
    Missing,
    Found(T),
}

impl<T> Lookup<T> {
    pub fn is_missing(&self) -> bool {
        matches!(self, Lookup::Missing)
    }

    pub fn found(&self) -> Option<&T> {
        match self {
            Lookup::Found(t) => Some(t),
            _ => None,
        }
    }
}

/// `AttributeType` subclasses: how an attribute's value is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttributeKind {
    Long,
    Int,
    Short,
    Byte,
    Double,
    Float,
    String,
    Bool,
    Enum,
    EnumSet,
    EnumStringSet,
}

impl AttributeKind {
    /// From the `AttributeType$<Kind>` inner-class simple name.
    pub fn from_class_suffix(s: &str) -> Option<AttributeKind> {
        use AttributeKind::*;
        Some(match s {
            "Long" => Long,
            "Int" => Int,
            "Short" => Short,
            "Byte" => Byte,
            "Double" => Double,
            "Float" => Float,
            "String" => String,
            "Bool" => Bool,
            "Enum" => Enum,
            "EnumSet" => EnumSet,
            "EnumStringSet" => EnumStringSet,
            _ => return None,
        })
    }
}

/// A registered `Fluid`: its type string and `getColor()` (r, g, b, a).
#[derive(Clone, Debug, PartialEq)]
pub struct FluidInfo {
    pub name: String,
    pub color: [f32; 4],
    /// `getCategories()` (ordinal order); empty when unknown.
    pub categories: Vec<pz_scripts::objects::definitions::FluidCategory>,
}

/// `ScriptsDictionary` registers whose script references components save.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScriptRegister {
    SpriteConfigs,
    SpriteOverlayConfigs,
    ContextMenuConfigs,
}

impl ScriptRegister {
    /// The register's `name` (as stored in `WorldDictionary.bin`).
    pub fn name(self) -> &'static str {
        match self {
            ScriptRegister::SpriteConfigs => "SpriteConfigs",
            ScriptRegister::SpriteOverlayConfigs => "SpriteOverlayConfigs",
            ScriptRegister::ContextMenuConfigs => "ContextMenuConfigs",
        }
    }

    pub fn from_name(s: &str) -> Option<ScriptRegister> {
        [
            ScriptRegister::SpriteConfigs,
            ScriptRegister::SpriteOverlayConfigs,
            ScriptRegister::ContextMenuConfigs,
        ]
        .into_iter()
        .find(|r| r.name() == s)
    }
}

/// A dictionary entry's script: full type and `getScriptVersion()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictScript {
    pub name: String,
    pub script_version: i64,
}

/// What `CraftRecipeData.load` checks against the recipe script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CraftRecipeInfo {
    pub script_version: i64,
    /// `inputs.size()` after `setRecipe`.
    pub input_count: usize,
}

/// The registries component loading consults.
pub trait EntityContext {
    /// `Attribute.TypeFromId(id)`: the value kind, `None` when no type has
    /// the id (the load throws). Required: the kind decides the layout.
    fn attribute_kind(&self, id: i16) -> Option<AttributeKind>;

    /// `Attribute.TypeFromName` (case-insensitive), for building attribute
    /// containers from scripts.
    fn attribute_type_by_name(&self, _name: &str) -> Option<crate::tables::AttributeTypeInfo> {
        None
    }

    /// `Fluid.Get(FluidType.FromId(id))`.
    fn fluid_by_id(&self, _id: i8) -> Lookup<FluidInfo> {
        Lookup::Unknown
    }

    /// `Fluid.Get(name)`.
    fn fluid_by_name(&self, _name: &str) -> Lookup<FluidInfo> {
        Lookup::Unknown
    }

    /// `ScriptsDictionary.<register>.getInfoForID(id).script`.
    fn dictionary_script(&self, _register: ScriptRegister, _id: i16) -> Lookup<DictScript> {
        Lookup::Unknown
    }

    /// `ScriptManager.getCraftRecipe(name)`.
    fn craft_recipe(&self, _name: &str) -> Lookup<CraftRecipeInfo> {
        Lookup::Unknown
    }

    /// Whether `SpriteOverlayConfig.getAvailableStyles()` of the overlay
    /// script contains `style`.
    fn overlay_style_exists(&self, _script: Option<&str>, _style: &str) -> bool {
        true
    }
}

/// `ScriptsDictionary` contents: per register, id to script.
#[derive(Clone, Debug, Default)]
pub struct ScriptDictionary {
    entries: HashMap<(ScriptRegister, i16), DictScript>,
}

impl ScriptDictionary {
    pub fn insert(&mut self, register: ScriptRegister, id: i16, script: DictScript) {
        self.entries.insert((register, id), script);
    }

    pub fn get(&self, register: ScriptRegister, id: i16) -> Option<&DictScript> {
        self.entries.get(&(register, id))
    }

    /// The id a script name is registered under.
    pub fn id_of(&self, register: ScriptRegister, name: &str) -> Option<i16> {
        self.entries
            .iter()
            .find(|((r, _), s)| *r == register && s.name == name)
            .map(|((_, id), _)| *id)
    }
}

/// The standard context: attribute types (from the jar), and optionally the
/// fluid registry, the scripts dictionary and craft recipes. Registries left
/// unset answer [`Lookup::Unknown`].
#[derive(Clone, Debug, Default)]
pub struct BasicContext {
    pub attributes: AttributeTypes,
    pub fluids: Option<FluidRegistry>,
    pub dictionary: Option<ScriptDictionary>,
    pub recipes: Option<HashMap<String, CraftRecipeInfo>>,
}

impl EntityContext for BasicContext {
    fn attribute_kind(&self, id: i16) -> Option<AttributeKind> {
        self.attributes.kind(id)
    }

    fn attribute_type_by_name(&self, name: &str) -> Option<crate::tables::AttributeTypeInfo> {
        self.attributes.by_name(name).cloned()
    }

    fn fluid_by_id(&self, id: i8) -> Lookup<FluidInfo> {
        match &self.fluids {
            None => Lookup::Unknown,
            Some(f) => f.by_id(id).cloned().map_or(Lookup::Missing, Lookup::Found),
        }
    }

    fn fluid_by_name(&self, name: &str) -> Lookup<FluidInfo> {
        match &self.fluids {
            None => Lookup::Unknown,
            Some(f) => f
                .by_name(name)
                .cloned()
                .map_or(Lookup::Missing, Lookup::Found),
        }
    }

    fn dictionary_script(&self, register: ScriptRegister, id: i16) -> Lookup<DictScript> {
        match &self.dictionary {
            None => Lookup::Unknown,
            Some(d) => d
                .get(register, id)
                .cloned()
                .map_or(Lookup::Missing, Lookup::Found),
        }
    }

    fn craft_recipe(&self, name: &str) -> Lookup<CraftRecipeInfo> {
        match &self.recipes {
            None => Lookup::Unknown,
            Some(r) => r.get(name).cloned().map_or(Lookup::Missing, Lookup::Found),
        }
    }
}
