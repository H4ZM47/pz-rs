//! `zombie.tileDepth`: the per-tile depth maps world sprites are drawn with.
//!
//! Every tileset may have a `media/depthmaps/DEPTH_<tileset>.png`, laid out
//! like the tileset itself (eight 2x tiles per row). A pixel's alpha says
//! whether it has depth and its blue channel holds the depth, 0 (front) to 1
//! (back). Each tile's map is turned into a single-channel texture
//! ([`texture::depth_bytes`]) that the `tileWithDepth` shader samples
//! through the `uv1` coordinates [`TileDepthModifier`] computes.
//!
//! A renderer asks [`TileDepthTextureManager`] for
//! * the depth map of a tile: [`TileDepthTextureManager::texture`] (tileset
//!   name and index) or [`TileDepthTextureManager::sprite_depth_texture`]
//!   (what `IsoSprite.depthTexture` holds, assignments included);
//! * a preset: [`TileDepthTextureManager::texture_for_preset`];
//! * the GPU texture of a tile: [`TileDepthTextureManager::texture_of`];
//!
//! and computes the draw's `uv1` with [`TileDepthModifier::uv1`] or
//! [`depth_uv1`].
//!
//! Not here: the tile geometry editor (`TileGeometry*`, which only
//! recalculates depth maps from `tileGeometry.txt` when editing),
//! `TileSeamModifier`/`TileSeamManager` (floor/wall seam masks drawn with a
//! third texture), `CutawayAttachedModifier`, and saving.

pub mod assignments;
pub mod clamp;
pub mod geometry;
pub mod manager;
pub mod modifier;
pub mod texture;

pub use geometry::{TileGeometryManager, TileGeometryProperties};
pub use assignments::{TileDepthTextureAssignmentManager, TileDepthTextureAssignments};
pub use manager::{split_tile_name, ModData, TileDepthPreset, TileDepthTextureManager, TileDepthTextures};
pub use modifier::{depth_uv1, wall_preset, TileDepthModifier, WallDirection, WallFlags};
pub use texture::{depth_bytes, DepthBytes, TileDepthTexture, TilesetDepthTexture, DEPTH_TEXTURE_FLAGS};

#[cfg(test)]
mod tests;
