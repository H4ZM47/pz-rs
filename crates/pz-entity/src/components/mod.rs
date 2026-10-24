//! The component kinds and their saved state.

pub mod attributes;
pub mod basic;
pub mod crafting;
pub mod resources;

pub use attributes::{Attribute, AttributeContainer, AttributeValue};
pub use basic::{
    CraftBench, DictScriptRef, Durability, EntityScriptInfo, MetaTag, SpriteConfig,
    SpriteOverlayConfig, UiConfig,
};
pub use crafting::{CraftLogic, CraftRecipeData, DryingCraftLogic};
pub use resources::{
    EnergyRef, ItemList, ItemRecord, Resource, ResourceBody, ResourceGroup, Resources,
};
