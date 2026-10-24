//! `LightingJNI.update()`'s per-player block: `stateBeginUpdate`,
//! `updatePlayer`, `stateEndFrame`, the chunk updates and
//! `stateEndUpdate`.

use super::chunk::update_chunk;
use super::input::{ChunkInfo, LightingWorld, PlayerInput, PlayerRenderSettings};
use super::natives::LightingNatives;
use crate::{Character, LightingError};

/// Values `update()` reads from `GameTime` once per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameParams {
    /// `GameTime.getViewDistMax()`.
    pub view_dist_max: f32,
    /// `GameTime.lightSourceUpdate` (set to 100 when lights changed, reset
    /// to 0 after the frame).
    pub light_source_update: f32,
    /// `GameTime.getSkyLightLevel()`, see
    /// [`PlayerRenderSettings::sky_light_level`].
    pub sky_light_level: i32,
}

/// One player's chunk map and settings for a frame.
#[derive(Clone, Debug, Default)]
pub struct PlayerFrame<'a> {
    pub index: i32,
    /// `IsoChunkMap.getWorldXMin()` / `getWorldYMin()` (chunks).
    pub chunk_x_min: i32,
    pub chunk_y_min: i32,
    /// `IsoChunkMap.chunkGridWidth`.
    pub chunk_grid_width: i32,
    /// `IsoPlayer.players[index]`; `None` skips `playerSet`.
    pub player: Option<PlayerInput>,
    /// `calculateVisionCone(player)`.
    pub cone: f32,
    /// `RenderSettings.getPlayerSettings(index)`.
    pub render: PlayerRenderSettings,
    /// `LosUtil.cachecleared[index]`.
    pub cache_cleared: bool,
    /// `cm.getChunk(cx, cy)` for `cy` outer, `cx` inner, the non-null ones.
    pub chunks: &'a [ChunkInfo],
}

/// What a player frame did, for the world to apply.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerFrameResult {
    /// `LightingJNI.updateCounter[index]` after `stateEndUpdate`.
    pub update_counter: i32,
    /// Chunks that were sent: clear `IsoChunk.lightCheck[index]`.
    pub chunks_sent: Vec<(i32, i32)>,
    /// Levels that were sent `(wx, wy, z)`: clear
    /// `IsoChunkLevel.lightCheck[index]`.
    pub levels_sent: Vec<(i32, i32, i32)>,
    /// `IsoChunk.lightingNeverDone[index]` per loaded chunk.
    pub lighting_never_done: Vec<((i32, i32), bool)>,
}

/// `updatePlayer(playerIndex)`: the `playerSet` arguments.
pub fn player_character(p: &PlayerInput, cone: f32) -> Character {
    let (mut lx, mut ly) = (p.look_x, p.look_y);
    if p.reversing_driver {
        let a = f64::from(std::f32::consts::PI);
        let rx = f64::from(lx) * a.cos() - f64::from(ly) * a.sin();
        let ry = f64::from(lx) * a.sin() + f64::from(ly) * a.cos();
        lx = rx as f32;
        ly = ry as f32;
    }
    Character {
        x: p.x,
        y: p.y,
        z: p.z + 32.0,
        look_x: lx,
        look_y: ly,
        see_all: false,
        reanimated: p.reanimated,
        ghost: p.ghost,
        short_sighted: p.short_sighted,
        fatigue: p.effective_fatigue,
        perception: p.detection_range,
        cone,
    }
}

/// One player's part of `LightingJNI.update()`. Run the
/// [`super::LightSync::check_lights`] of the frame first, and call
/// `do_lighting_update` (the lighting thread) between frames.
pub fn update_player(
    n: &mut impl LightingNatives,
    world: &impl LightingWorld,
    f: &PlayerFrame<'_>,
    params: &FrameParams,
) -> Result<PlayerFrameResult, LightingError> {
    let mut out = PlayerFrameResult::default();
    n.state_begin_update(f.index, f.chunk_x_min, f.chunk_y_min, f.chunk_grid_width, f.chunk_grid_width)?;
    let body = (|| {
        if let Some(p) = &f.player {
            n.player_set(player_character(p, f.cone))?;
        }
        let r = &f.render;
        n.state_end_frame(
            r.rmod,
            r.gmod,
            r.bmod,
            r.ambient,
            r.night,
            r.view_distance,
            params.view_dist_max,
            f.cache_cleared,
            params.light_source_update,
            params.sky_light_level,
        )?;
        for c in f.chunks.iter().filter(|c| c.loaded) {
            if c.light_check {
                let mut sent = Vec::new();
                update_chunk(n, world, f.index, c, params.sky_light_level, |z| {
                    let i = (z - c.min_level) as usize;
                    let check = c.level_light_check.get(i).copied().unwrap_or(false);
                    if check {
                        sent.push((c.wx, c.wy, z));
                    }
                    check
                })?;
                out.levels_sent.extend(sent);
                out.chunks_sent.push((c.wx, c.wy));
            }
            let done = n.chunk_lighting_done(c.wx, c.wy)?;
            out.lighting_never_done.push(((c.wx, c.wy), !done));
        }
        Ok(())
    })();
    let end = n.state_end_update();
    body.and(end)?;
    out.update_counter = n.state_update_counter(f.index)?;
    Ok(out)
}

/// `updateVisibleRooms()` for one player: the room ids the library saw this
/// update (the world maps them to `RoomDef`s).
pub fn visible_room_ids(n: &mut impl LightingNatives, player: i32) -> Result<Vec<i64>, LightingError> {
    let count = n.get_visible_room_count(player)?;
    if count <= 0 {
        return Ok(Vec::new());
    }
    let mut ids = vec![0i64; count as usize];
    n.get_visible_rooms(player, &mut ids)?;
    Ok(ids)
}
