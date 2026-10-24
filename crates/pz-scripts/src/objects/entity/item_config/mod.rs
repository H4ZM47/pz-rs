//! `zombie.scripting.itemConfig`: item configs, their bucket scripts and enums.

mod bucket_root_script;
mod enums;
#[allow(clippy::module_inception)]
mod item_config;
mod selector_bucket_script;

#[cfg(test)]
pub(crate) use bucket_root_script::load_selector_bucket;
pub use bucket_root_script::BucketRootScript;
pub use enums::{RootType, SelectorType, SituatedType};
pub use item_config::ItemConfig;
pub use selector_bucket_script::SelectorBucketScript;
