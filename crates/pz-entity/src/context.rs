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
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FluidInfo {
    pub name: String,
    pub color: [f32; 4],
    /// `getCategories()` (ordinal order); empty when unknown.
    pub categories: Vec<pz_scripts::objects::definitions::FluidCategory>,
    /// `blendWhitelist` / `blendBlacklist` (`canBlendWith`).
    pub blend_whitelist: Option<crate::fluids::FluidFilter>,
    pub blend_blacklist: Option<crate::fluids::FluidFilter>,
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

    /// Actual registered type identity, with missing capability separate
    /// from a genuine native null registry lookup. No type body is borrowed.
    fn attribute_type_ref(&self, _id: i16) -> Lookup<crate::tables::AttributeTypeRef> {
        Lookup::Unknown
    }

    fn attribute_type_by_name_ref(&self, _name: &str) -> Lookup<crate::tables::AttributeTypeRef> {
        Lookup::Unknown
    }

    /// `Attribute.TypeFromName` (case-insensitive), for building attribute
    /// containers from scripts.
    fn attribute_type_by_name(&self, _name: &str) -> Option<crate::tables::AttributeTypeInfo> {
        None
    }

    /// `Attribute.TypeFromId(id)`'s registration (name, kind, UI
    /// display), when the context knows it.
    fn attribute_type(&self, _id: i16) -> Option<crate::tables::AttributeTypeInfo> {
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

    /// The registered Fluid's actual sealed properties. `Found(None)` means
    /// a known native null field; `Unknown` does not imply zero properties.
    /// This lookup must be callback-free when used by a component data phase.
    fn fluid_properties(
        &self,
        _fluid: &crate::fluids::FluidRef,
    ) -> Lookup<Option<crate::fluids::SealedFluidProperties>> {
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
        self.attributes.by_name(name).map(|value| (*value).clone())
    }

    fn attribute_type(&self, id: i16) -> Option<crate::tables::AttributeTypeInfo> {
        self.attributes.get(id).map(|value| (*value).clone())
    }

    fn attribute_type_ref(&self, id: i16) -> Lookup<crate::tables::AttributeTypeRef> {
        self.attributes
            .get_ref(id)
            .map_or(Lookup::Missing, Lookup::Found)
    }

    fn attribute_type_by_name_ref(&self, name: &str) -> Lookup<crate::tables::AttributeTypeRef> {
        self.attributes
            .by_name_ref(name)
            .map_or(Lookup::Missing, Lookup::Found)
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

    fn fluid_properties(
        &self,
        fluid: &crate::fluids::FluidRef,
    ) -> Lookup<Option<crate::fluids::SealedFluidProperties>> {
        match &self.fluids {
            None => Lookup::Unknown,
            Some(registry) => registry.properties(fluid),
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

#[cfg(test)]
mod fluid_property_tests {
    use super::*;
    use crate::fluids::{FluidRef, SealedFluidProperties};

    #[test]
    fn absent_registry_missing_fluid_and_known_null_are_distinct() {
        let fluid = FluidRef::Modded("Example.Tea".into());
        let mut context = BasicContext::default();
        assert_eq!(context.fluid_properties(&fluid), Lookup::Unknown);
        context.fluids = Some(FluidRegistry::default());
        assert_eq!(context.fluid_properties(&fluid), Lookup::Missing);
        let registry = context.fluids.as_mut().unwrap();
        registry.insert(
            None,
            FluidInfo {
                name: "Example.Tea".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            context.fluid_properties(&fluid),
            Lookup::Unknown,
            "known colour metadata does not prove null or zero properties"
        );
        assert!(context
            .fluids
            .as_mut()
            .unwrap()
            .set_properties("Example.Tea", None));
        assert_eq!(context.fluid_properties(&fluid), Lookup::Found(None));
    }

    #[test]
    fn enum_and_name_lookup_share_properties_and_replacement_invalidates_unknown_metadata() {
        use pz_scripts::objects::definitions::FluidType;
        let mut registry = FluidRegistry::default();
        let info = FluidInfo {
            name: "Water".into(),
            ..Default::default()
        };
        registry.insert(Some(FluidType::Water.id()), info.clone());
        let properties = SealedFluidProperties {
            calories: 17.25,
            poison: 3.0,
            food_sickness_change: -4,
            ..Default::default()
        };
        assert!(registry.set_properties("Water", Some(properties.clone())));
        let context = BasicContext {
            fluids: Some(registry),
            ..Default::default()
        };
        for fluid in [
            FluidRef::Type(FluidType::Water.id()),
            FluidRef::Modded("Water".into()),
        ] {
            assert_eq!(
                context.fluid_properties(&fluid),
                Lookup::Found(Some(properties.clone()))
            );
        }
        let mut registry = context.fluids.unwrap();
        registry.insert(Some(FluidType::Water.id()), info);
        assert_eq!(
            registry.properties(&FluidRef::Type(FluidType::Water.id())),
            Lookup::Unknown
        );
        assert!(!registry.set_properties("Missing", None));
    }
}
