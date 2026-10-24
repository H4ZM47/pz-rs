//! A character's 3D model: the model tree (`ModelManager.ModelSlot`,
//! `ModelInstance`) built from the body, hair, beard, clothing and carried
//! items; the composited body/clothing textures
//! (`ModelInstanceTextureCreator`, as GPU [`texture_creator::SmartTextureJob`]s);
//! the per-frame lights (`ModelInstance.updateLights`); and the frame's
//! draw list (`ModelSlotRenderData`) as [`pz_render::model::ModelDraw`]s in
//! the original's order.
//!
//! Item visuals and clothing items come from pz-inventory, the human
//! visual and body-location groups from pz-characters, model scripts from
//! pz-scripts and meshes from pz-model (see [`data`] and
//! [`resources`]); square lights are still minimal structs in [`lights`].

#![allow(clippy::too_many_arguments, clippy::needless_range_loop)]

pub mod data;
pub mod lights;
pub mod mask;
pub mod model_tree;
pub mod render;
pub mod resources;
pub mod texture_creator;

pub use data::{
    BodyLocationGroup, BodyLocationKey, ClothingDecal, ClothingItem, ClothingItemExt,
    EquippedItems, HairStyles, HumanModel, HumanVisual, ItemVisual, ModelAttachment, ModelScript,
    ModelScriptRef, ResolvedItemVisual, StaticItemModel,
};
pub use lights::{EffectLight, LightContext, PlayerLightData, ResultLight};
pub use model_tree::{
    CharacterModelInput, InstanceKind, ModelInstance, ModelScripts, ModelSlot, TextureSource,
};
pub use render::{render_character, CharacterFrame, LoadedMesh, ModelResources};
pub use resources::MediaModelResources;
pub use texture_creator::{
    create_textures, SmartTextureJob, TextureCreatorInput, TextureCreatorOutput,
};

/// `PZMath.fastfloor`.
pub(crate) fn fastfloor(v: f32) -> i32 {
    let xi = v as i32;
    if v < xi as f32 {
        xi - 1
    } else {
        xi
    }
}
