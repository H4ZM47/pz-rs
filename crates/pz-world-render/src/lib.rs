//! Turns pz-world data into what pz-iso-render and pz-lighting need.
//!
//! - [`sprites`]: `IsoSprite` to the renderer's `SpriteInfo` (flags,
//!   textures, depth textures, tile-geometry depth flags).
//! - [`render_world`]: `IsoGridSquare`s and their objects as
//!   `RenderSquare`s (seams, cutaway and occlusion inputs) and a
//!   `CutawayWorld` over them.
//! - [`lighting`]: the `LightingWorld` adapter, lamp and room-light sync and
//!   the per-frame lighting driver that writes results back into the render
//!   squares.
//! - [`hooks`] and [`settings`]: inputs owned by other systems (thumpables,
//!   barricades, power, torches; `RenderSettings`/`ClimateManager`).
//!
//! Square properties, vision and collision matrices come from pz-world
//! (`IsoGridSquare::recalc_properties`, `IsoCell::recalc_chunk_squares`).
//!
//! Per frame, for each loaded or changed chunk:
//! `IsoCell::recalc_chunk_squares`, [`RenderWorld::build_chunk`],
//! [`WorldLighting::sync_chunk`]; then [`WorldLighting::frame`] for each
//! player, [`WorldLighting::apply`] into the [`RenderWorld`], and invalidate
//! the chunk textures of the returned dirty levels.

pub mod hooks;
pub mod lighting;
pub mod render_world;
pub mod settings;
pub mod sprites;

pub use hooks::{NoWorldHooks, WorldHooks};
pub use lighting::{
    default_player, grid_square, LightingFrame, LightingSquares, PlayerFrameInput, WorldLighting,
};
pub use render_world::{
    building_id, cutaway_info, render_square, seam_neighbours, wall_type, RenderWorld, WorldView,
};
pub use settings::{ClimateInput, ClimateSettings, NoonSettings, PlayerRenderInput, RenderSettingsSource};
pub use sprites::{GameTextures, NoTextures, SpriteCache, SpriteTextures};
