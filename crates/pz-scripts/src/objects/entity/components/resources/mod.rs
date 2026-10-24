//! `zombie.scripting.entity.components.resources`, with the
//! `zombie.entity.components.resources` data it parses.

mod enums;
mod resource_blueprint;
mod resources_script;

pub use enums::{ResourceChannel, ResourceFlag, ResourceIO, ResourceType};
pub use resource_blueprint::ResourceBlueprint;
pub use resources_script::ResourcesScript;
