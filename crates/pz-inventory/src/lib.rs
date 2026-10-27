//! Inventory items (`zombie.inventory`): `InventoryItem` and its
//! subclasses, `InventoryItemFactory` creation from item scripts, item
//! saving and loading (`save`/`load`, `saveWithSize`/`loadItem`),
//! `ItemContainer` with `CompressIdenticalItems`, and the item ids of
//! `WorldDictionary`.

pub mod alarm;
pub mod api;
pub mod archive;
pub mod assets;
pub mod component;
pub mod component_host;
pub mod container;
pub mod context;
pub mod device;
pub mod dictionary;
pub mod env;
pub mod factory;
pub mod floor_drop;
pub mod food_age;
pub mod gameplay;
pub mod io;
pub mod item;
pub mod item_components;
pub mod item_texture;
pub mod java;
pub mod key_namer;
pub mod light;
pub mod lua_table;
pub mod moddata;
pub mod model;
pub mod naming;
pub mod saved;
pub mod shared;
pub mod shared_save;
pub mod shared_load;
pub mod template;
pub mod visual;
pub mod world_item;
pub mod world_update;
pub mod worn_items;

pub use assets::{BloodClothingTypes, ClothingItem, ClothingItems, ItemAssets};
pub use component::{ComponentBlob, ComponentCodec, OpaqueComponents};
pub use container::ItemContainer;
pub use context::{ItemContext, ItemOwner, ItemPlace, WetDryState};
pub use env::ItemEnv;
pub use item::{InventoryItem, ItemKind};
pub use item_components::ItemComponents;
pub use moddata::{ItemModData, ModDataError, ModDataResult};
pub use saved::SavedItem;
pub use shared::{ContainerRef, SharedContainer};
pub use template::{ItemClass, ItemTemplate};

#[cfg(test)]
mod gameplay_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vehicle_tests;
