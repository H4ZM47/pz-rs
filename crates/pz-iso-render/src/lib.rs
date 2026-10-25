//! The build-42 isometric world renderer: `FBORenderCell` and the
//! `IsoGridSquare`/`IsoObject`/`IsoSprite` render paths it drives, recording
//! [`pz_render::DrawCmd`]s that `pz-gpu` executes.
//!
//! Each chunk's level pair is recorded into its own colour + depth texture
//! (re-recorded only when dirty) and the textures are composited into the
//! offscreen buffer every frame; overlap between tiles and chunks is
//! resolved by per-pixel depth from the tiles' depth textures.
//!
//! The world is read through the plain data in [`world`], which the world
//! crate fills; lighting comes in as each square's light colour and eight
//! vertex lights, as `LightingJNI` produces them.

pub mod alpha;
pub mod blood;
pub mod atlas;
pub mod body_atlas;
pub mod cache;
pub mod cell;
pub mod chunk;
pub mod color;
pub mod cutaways;
pub mod fire;
pub mod frame;
pub mod hearth;
pub mod item_atlas;
pub mod joml;
pub mod models;
pub mod objects;
pub mod obscuring;
pub mod occlusion;
pub mod particles_fire;
pub mod puddles;
pub mod outlines;
pub mod seam;
pub mod skybox;
pub mod snow;
pub mod splashes;
pub mod shadows;
pub mod sprite;
pub mod texd;
pub mod trees;
pub mod water;
pub mod water_flow;
pub mod weather;
pub mod weather_fx;
pub mod weather_fx_mask;
pub mod world;

pub use cell::ChunkRenderer;
pub use alpha::CutawayView;
pub use chunk::ChunkTexture;
pub use cutaways::{CutawayPlayer, CutawayState, CutawayWorld, Cutaways};
pub use world::{DepthMaps, DepthPreset, ObjectKind, RenderObject, RenderSquare, SpriteFlags, SpriteInfo, TileType};

use pz_render::iso::fastfloor;

/// `IsoCamera.playerOffsetX`.
pub const PLAYER_OFFSET_X: i32 = 0;
/// `IsoCamera.playerOffsetY`: `-56 / (2 / Core.tileScale)`.
pub const PLAYER_OFFSET_Y: i32 = -56 / (2 / pz_render::iso::TILE_SCALE);

/// `IsoCamera.frameState` and the player camera values the renderer reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// `getOffX()/getOffY()`: integer screen offset of the view.
    pub off_x: i32,
    pub off_y: i32,
    pub zoom: f32,
    /// The camera character's position (`camCharacterX/Y/Z`).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `PlayerCamera.fixJigglyModels*`.
    pub fix_jiggly_x: f32,
    pub fix_jiggly_y: f32,
    pub fix_jiggly_square_x: f32,
    pub fix_jiggly_square_y: f32,
}

impl Camera {
    /// A camera at `(x, y, z)` whose view of `width` x `height` pixels at
    /// `zoom` is centred on it (`PlayerCamera.center` without the
    /// character offsets).
    pub fn centred(x: f32, y: f32, z: f32, width: i32, height: i32, zoom: f32) -> Self {
        let w = width as f32 * zoom;
        let h = height as f32 * zoom;
        let off_x = pz_render::iso::x_to_screen(x, y) - w / 2.0;
        let off_y = pz_render::iso::y_to_screen(x, y, z, 0) - h / 2.0;
        Camera {
            off_x: off_x as i32,
            off_y: off_y as i32,
            zoom,
            x,
            y,
            z,
            fix_jiggly_x: 0.0,
            fix_jiggly_y: 0.0,
            fix_jiggly_square_x: 0.0,
            fix_jiggly_square_y: 0.0,
        }
    }

    /// `PlayerCamera.center` for a camera character at `(x, y, z)` with
    /// no deferred movement or offset: [`Camera::centred`] moved by
    /// `IsoCamera.playerOffsetY` (-56 pixels at tile scale 2), which
    /// lines tiles up with the model camera's `-0.48` foot offset.
    pub fn player(x: f32, y: f32, z: f32, width: i32, height: i32, zoom: f32) -> Self {
        let w = width as f32 * zoom;
        let h = height as f32 * zoom;
        let off_x = pz_render::iso::x_to_screen(x, y) - w / 2.0 + PLAYER_OFFSET_X as f32;
        let off_y = pz_render::iso::y_to_screen(x, y, z, 0) - h / 2.0 + PLAYER_OFFSET_Y as f32;
        Camera { off_x: off_x as i32, off_y: off_y as i32, ..Camera::centred(x, y, z, width, height, zoom) }
    }

    /// The camera character's chunk.
    pub fn chunk_x(&self) -> i32 {
        fastfloor(self.x / 8.0)
    }

    pub fn chunk_y(&self) -> i32 {
        fastfloor(self.y / 8.0)
    }
}
