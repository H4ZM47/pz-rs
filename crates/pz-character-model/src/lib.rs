//! A character's 3D model: the model tree (`ModelManager.ModelSlot`,
//! `ModelInstance`) built from the body, hair, beard, clothing and carried
//! items; the composited body/clothing textures
//! (`ModelInstanceTextureCreator`, as GPU [`texture_creator::SmartTextureJob`]s);
//! the per-frame lights (`ModelInstance.updateLights`); and the frame's
//! draw list (`ModelSlotRenderData`) as [`pz_render::model::ModelDraw`]s in
//! the original's order; and, in [`scene`], the character's drop shadow and
//! draws as one object of the iso frame's object pass.
//!
//! Item visuals and clothing items come from pz-inventory, the human
//! visual from pz-inventory, body-location groups and attached items from
//! pz-characters, model scripts from pz-scripts and meshes from pz-model
//! (see [`data`], [`items`] and [`resources`]); the static models of held
//! and attached items are picked in [`items`]. Types other threads own
//! come in through [`external`]; square lights are still minimal structs
//! in [`lights`].

#![allow(clippy::too_many_arguments, clippy::needless_range_loop)]

pub mod combine;
pub mod corpse;
pub mod data;
pub mod external;
pub mod item_texture;
pub mod items;
pub mod lights;
pub mod mannequin;
pub mod mask;
pub mod model_tree;
pub mod render;
pub mod resources;
pub mod scene;
pub mod texture_creator;
pub mod world_item;

pub use data::{
    BeardStyles, BodyLocationGroup, BodyLocationKey, ClothingDecal, ClothingDecals, ClothingItem,
    ClothingItemExt, HairStyles, HumanModel, HumanVisual, ItemVisual, ModelAttachment, ModelScript,
    ModelScriptRef, PopTemplateManager, Population, PopulationStyles, ResolvedItemVisual,
};
pub use items::{ActionHandModels, AttachedModel, EquippedModels, HeldItem, ItemTextureInit};
pub use lights::{EffectLight, LightContext, PlayerLightData, ResultLight};
pub use model_tree::{
    CharacterModelInput, InstanceKind, ModelInstance, ModelScripts, ModelSlot, TextureSource,
};
pub use render::{render_character, CharacterFrame, LoadedMesh, ModelResources};
pub use resources::MediaModelResources;
pub use scene::{character_shadow, render_character_object, shadow_params, ShadowInput, ShadowSmoothing};
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
