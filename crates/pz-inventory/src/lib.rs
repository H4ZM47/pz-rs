//! Inventory items (`zombie.inventory`): `InventoryItem` and its
//! subclasses, `InventoryItemFactory` creation from item scripts, item
//! saving and loading (`save`/`load`, `saveWithSize`/`loadItem`),
//! `ItemContainer` with `CompressIdenticalItems`, and the item ids of
//! `WorldDictionary`.

pub mod api;
pub mod component;
pub mod container;
pub mod device;
pub mod dictionary;
pub mod env;
pub mod io;
pub mod item;
pub mod java;
pub mod lua_table;
pub mod saved;
pub mod template;
pub mod visual;

pub use component::{ComponentBlob, ComponentCodec, OpaqueComponents};
pub use container::ItemContainer;
pub use env::ItemEnv;
pub use item::{InventoryItem, ItemKind};
pub use saved::SavedItem;
pub use template::{ItemClass, ItemTemplate};
