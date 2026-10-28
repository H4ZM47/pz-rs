//! Port of `zombie.entity`'s component storage and save format: the
//! components a `GameEntity` (map object, inventory item, vehicle part,
//! meta entity) carries, `GameEntity.saveEntity`/`loadEntity` with their
//! `ByteBlock` framing, the saved state of each component type, and
//! `GameEntityFactory`'s default components.
//!
//! The codec:
//!
//! - [`load_components`] / [`load_components_into`] read `loadEntity` data
//!   (from the count byte) given the world version and an
//!   [`EntityContext`] for the registries the original consults;
//! - [`save_components`] writes `saveEntity` data and needs no context.
//!
//! Decoding then encoding reproduces the saved bytes whenever the original's
//! load-then-save would.

pub mod color;
pub mod components;
pub mod context;
pub mod entity;
pub mod factory;
pub mod fluids;
pub mod io;
pub mod java;
pub mod meta;
pub mod runtime;
pub mod runtime_attributes;
pub mod runtime_durability;
pub mod runtime_event;
pub mod runtime_fluid;
pub mod runtime_load;
pub mod runtime_pool;
pub mod tables;

pub use context::{
    AttributeKind, BasicContext, EntityContext, FluidInfo, Lookup, ScriptDictionary, ScriptRegister,
};
pub use entity::{
    entity_len, load_components, load_components_into, save_components, write_components,
    Component, Components, LoadInfo,
};
pub use fluids::{FluidContainer, FluidFilter, FluidInstance, FluidRef, FluidRegistry};
pub use io::{EntityError, Result};
pub use meta::{load_meta_entities, save_meta_entities, MetaEntity};
pub use pz_scripts::objects::entity::ComponentType;
pub use runtime::{
    ComponentBodyMut, ComponentRef, EncodedComponent, EntityOwner, EntryPresence, Promotion,
    RejectedComponent, RejectedComponentRef, RejectedEncoded, RejectedValues, RetiredComponents,
    RuntimeComponent, RuntimeComponents, RuntimeEntryView, RuntimeError,
};
pub use runtime_durability::RuntimeDurability;
pub use tables::{
    AttributeTypeDefinition, AttributeTypeRef, AttributeTypes, NumericAttributeBounds,
    NumericAttributeValidation,
};

#[cfg(test)]
mod tests;
