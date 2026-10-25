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
//! - [`fire_lights`]: fires' and fireplaces' light sources in the cell's
//!   light list ([`WorldLighting::add_lamppost`] and friends).
//! - [`weather_mask`]: the weather mask's `FxMaskWorld` over the render
//!   squares and `pz_world::isoregion` world regions.
//! - [`invalidate`]: the `RenderSquareHooks` square mutations are given
//!   (chunk texture, lighting, water and building invalidation) and the
//!   per-frame rebuild of the squares they changed.
//! - [`hooks`] and [`settings`]: inputs owned by other systems (thumpables,
//!   barricades, power, torches; `RenderSettings`/`ClimateManager`).
//! - [`state_light`]: the light the character states read
//!   (`StateWorld::square_light`, `square_light_level`,
//!   `ambient_for_player`).
//!
//! Square properties, vision and collision matrices come from pz-world
//! (`IsoGridSquare::recalc_properties`, `IsoCell::recalc_chunk_squares`).
//!
//! Per frame, for each loaded or changed chunk:
//! `IsoCell::recalc_chunk_squares`, [`RenderWorld::build_chunk`],
//! [`WorldLighting::sync_chunk`]; then [`WorldLighting::frame`] for each
//! player, [`WorldLighting::apply`] into the [`RenderWorld`], and invalidate
//! the chunk textures of the returned dirty levels. Square edits after
//! load go through [`RenderInvalidation`]; drain its [`PendingRender`]
//! before the lighting frames and run [`check_changed_buildings`] after
//! them.

pub mod fire_lights;
pub mod hooks;
pub mod invalidate;
pub mod lighting;
pub mod render_world;
pub mod settings;
pub mod sprites;
pub mod state_light;
pub mod weather_mask;

pub use fire_lights::{fireplace_input, FireLightLinks, HearthLink, LightLink};
pub use hooks::{NoWorldHooks, WorldHooks};
pub use invalidate::{check_changed_buildings, PendingRender, RenderInvalidation};
pub use lighting::{
    default_player, grid_square, CellLight, LightingFrame, LightingSquares, PlayerFrameInput, WorldLighting,
};
pub use render_world::{
    building_id, cutaway_info, render_square, seam_neighbours, wall_type, RenderWorld, WorldView,
};
pub use settings::{
    ClimateInput, ClimateSettings, NoonSettings, PlayerRenderInput, RenderSettingsSource,
};
pub use sprites::{GameTextures, NoTextures, SpriteCache, SpriteTextures};
pub use state_light::{RenderSettings, StateLighting};
pub use weather_mask::{master_region, WorldFxMask};
