//! A character's 3D model: the model tree (`ModelManager.ModelSlot`,
//! `ModelInstance`) built from the body, hair, beard, clothing and carried
//! items; the composited body/clothing textures
//! (`ModelInstanceTextureCreator`, as GPU [`texture_creator::SmartTextureJob`]s);
//! the per-frame lights (`ModelInstance.updateLights`); and the frame's
//! draw list (`ModelSlotRenderData`) as [`pz_render::model::ModelDraw`]s in
//! the original's order.
//!
//! Inputs the other crates will own once merged (item visuals, model
//! scripts, body-location rules, square lights) are minimal structs in
//! [`data`] and [`lights`].

#![allow(clippy::too_many_arguments, clippy::needless_range_loop)]

pub mod data;
pub mod lights;
pub mod mask;
pub mod model_tree;
pub mod render;
pub mod texture_creator;

pub use data::{BodyLocationGroup, ClothingDecal, ClothingItem, EquippedItems, HairStyles, HumanVisual, ImmutableColor, ItemVisual, ModelAttachment, ModelScript, StaticItemModel};
pub use lights::{EffectLight, LightContext, PlayerLightData, ResultLight};
pub use model_tree::{CharacterModelInput, InstanceKind, ModelInstance, ModelScripts, ModelSlot, TextureSource};
pub use render::{render_character, CharacterFrame, LoadedMesh, ModelResources};
pub use texture_creator::{create_textures, SmartTextureJob, TextureCreatorInput, TextureCreatorOutput};

/// `PZMath.fastfloor`.
pub(crate) fn fastfloor(v: f32) -> i32 {
    let xi = v as i32;
    if v < xi as f32 {
        xi - 1
    } else {
        xi
    }
}
