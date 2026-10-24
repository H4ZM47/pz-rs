//! `zombie.scripting.entity.components.spriteconfig`.

mod sprite_config_script;
mod sprite_overlay_config_script;

pub use sprite_config_script::{get_face_id_for_string, FaceScript, SpriteConfigScript, TileScript, XRow, ZLayer};
pub use sprite_overlay_config_script::{OverlayFaceScript, OverlayStyle, SpriteOverlayConfigScript};
