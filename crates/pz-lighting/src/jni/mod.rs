//! The Java half of `zombie.iso.LightingJNI`, on plain data.
//!
//! The world crate describes squares, special objects, chunks, lights and
//! the player with the structs in this module (each field names the Java
//! accessor it mirrors) and implements [`LightingWorld`] for square lookup.
//! The functions here then make the same native calls the game makes:
//!
//! * `checkLights()` ([`LightSync::check_lights`]): light, room-light and
//!   torch bookkeeping;
//! * one player's block of `update()` ([`update_player`])
//!   (`stateBeginUpdate`, `playerSet`, `stateEndFrame`, `updateChunk` for
//!   changed chunks, `chunkLightingDone`, `stateEndUpdate`);
//! * `updateChunk` ([`square_args`], [`update_chunk`]): the per-square `squareSet`,
//!   `squareAdd*` and transmission arguments (vision flags from
//!   [`test_vision_adjacent`], open air, curtain filters, door and
//!   barricade transmissions);
//! * `calculateVisionCone` ([`VisionCone::calculate`]);
//! * `JNILighting` ([`SquareLight`]), decoding `getSquareLighting` for the
//!   renderer (vis bits with `ForcedVis`, light colour, darkness, the eight
//!   ABGR vertex lights, the light list) with the FBO path's per-level
//!   invalidation signal, or the legacy path's 0.85 scaling below the
//!   player; [`ChunkLevelLight`] does a whole chunk level.
//!
//! All of it is generic over [`LightingNatives`], implemented by
//! [`crate::LightingSession`].
//!
//! A frame, in the game's order:
//!
//! 1. `sync.check_lights(&mut session, ..)`; set `lightSourceUpdate = 100`
//!    if it says so;
//! 2. for each player, `update_player(..)` and apply its result (clear the
//!    light-check flags, store `update_counter`);
//! 3. `lightSourceUpdate = 0`;
//! 4. `session.do_lighting_update(nanos, dirty_global)` (the lighting
//!    thread);
//! 5. when rendering, `refresh` the chunk levels on screen and redraw those
//!    whose result says `invalidate`.

mod chunk;
mod cone;
mod decode;
mod driver;
mod input;
mod lights;
mod natives;
mod vision;

pub use chunk::{square_args, update_chunk, ObjectCall, SquareArgs};
pub use cone::{detection_range, effective_fatigue, luma_inverted, VisionCone, VisionConeInput};
pub use decode::{
    ChunkLevelLight, ColorInfo, DecodePlayer, LevelRefresh, Refresh, ResultLightInfo, SquareLight, SquarePos, FORCED_VIS,
};
pub use driver::{player_character, update_player, visible_room_ids, FrameParams, PlayerFrame, PlayerFrameResult};
pub use input::{
    Barricade, ChunkInfo, Curtain, CurtainType, Door, GridSquare, IsoDirection, LightSource, LightingWorld,
    PlayerInput, PlayerRenderSettings, RoomLightSource, SpecialObject, Thumpable, TorchInfo, Window,
};
pub use lights::{player_torch_id, vehicle_torch_id, CheckLightsResult, LightSync};
pub use natives::LightingNatives;
pub use vision::{open_air, test_vision_adjacent};
