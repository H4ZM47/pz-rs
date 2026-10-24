//! `LightingJNI.updateChunk`: the per-square arguments and the chunk
//! streaming calls.

use super::input::{ChunkInfo, CurtainType, GridSquare, IsoDirection, LightingWorld, SpecialObject};
use super::natives::LightingNatives;
use super::vision::{open_air, test_vision_adjacent};
use crate::{LightingError, TestResult, Transmission};

/// One `squareAdd*` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectCall {
    /// `squareAddCurtain(wnes, open)`.
    Curtain { wnes: i32, open: bool },
    /// `squareAddDoor(north, open, trans)`.
    Door { north: bool, open: bool, trans: bool },
    /// `squareAddThumpable(north, open, isDoor, doorTrans)`.
    Thumpable { north: bool, open: bool, is_door: bool, door_trans: bool },
    /// `squareAddWindow(north, open, opaque)`.
    Window { north: bool, open: bool, opaque: bool },
}

/// Everything `updateChunk` sends for one square.
#[derive(Clone, Debug, PartialEq)]
pub struct SquareArgs {
    /// One bit per [`IsoDirection`] ordinal: vision to that neighbour is
    /// not blocked.
    pub vision_unblocked: i32,
    pub up_clear: bool,
    pub down_clear: bool,
    pub elevated_floor: bool,
    /// `visionMatrix`, plus bit `27 + player` when the player has seen the
    /// square.
    pub vision_matrix: i32,
    pub building_id: i64,
    pub room_id: i64,
    /// The sky level for open-air squares (Java also stores it in
    /// `square.lightLevel`), the square's `lightLevel` otherwise.
    pub light_level: i32,
    pub open_air: bool,
    pub objects: Vec<ObjectCall>,
    /// W, N, E, S.
    pub transmissions: [Transmission; 4],
}

/// `PZMath.tryParseInt(s, 0)`.
fn try_parse_int(s: Option<&str>) -> Option<i32> {
    let s = s?;
    let t = s.trim();
    if t.is_empty() {
        return Some(0);
    }
    Some(t.parse().unwrap_or(0))
}

/// Builds the `squareSet`, `squareAdd*` and `squareSetLightTransmission`
/// arguments of `square` for `player` (the body of `updateChunk`'s square
/// loop).
pub fn square_args(world: &impl LightingWorld, sq: &GridSquare, player: i32, sky_light_level: i32) -> SquareArgs {
    let mut vision_matrix = sq.vision_matrix;
    if (sq.seen_by >> (player & 7)) & 1 != 0 {
        vision_matrix |= 1 << (27 + player);
    }
    let is_open_air = open_air(world, sq);
    let light_level = if is_open_air { sky_light_level } else { sq.light_level };
    let mut vision_unblocked = 0;
    for (i, dir) in IsoDirection::ALL.iter().enumerate() {
        let (dx, dy) = dir.delta();
        if test_vision_adjacent(world, sq, dx, dy, 0, true, false) != TestResult::Blocked {
            vision_unblocked |= 1 << i;
        }
    }
    let up_clear = test_vision_adjacent(world, sq, 0, 0, 1, true, false) != TestResult::Blocked;
    let down_clear = test_vision_adjacent(world, sq, 0, 0, -1, true, false) != TestResult::Blocked;

    const UNSET: f32 = f32::MAX;
    let mut intensity = [UNSET; 4]; // W N E S
    let mut colour = [[0.0f32; 3]; 4];
    let (w, n, e, s) = (0, 1, 2, 3);
    let mut objects = Vec::new();
    for obj in &sq.special_objects {
        match obj {
            SpecialObject::Curtain(c) => {
                let filter = |p: &Option<String>| {
                    try_parse_int(p.as_deref()).map_or(0.0, |v| v.clamp(0, 255) as f32 / 255.0)
                };
                let rgb = [filter(&c.light_filter_r), filter(&c.light_filter_g), filter(&c.light_filter_b)];
                let mut ci = 0.33f32;
                if let Some(v) = try_parse_int(c.light_filter_intensity.as_deref()) {
                    ci = v.max(0) as f32 / 100.0;
                }
                if let Some(v) = try_parse_int(c.light_filter_mix.as_deref()) {
                    ci = v.max(0) as f32 / 100.0;
                }
                let (wnes, side) = match c.kind {
                    CurtainType::W => (4, Some(w)),
                    CurtainType::N => (8, Some(n)),
                    CurtainType::E => (0x10, Some(e)),
                    CurtainType::S => (0x20, Some(s)),
                    CurtainType::Other => (0, None),
                };
                if let Some(k) = side {
                    if !c.open {
                        intensity[k] = intensity[k].min(ci);
                        colour[k] = rgb;
                    }
                }
                objects.push(ObjectCall::Curtain { wnes, open: c.open });
            }
            SpecialObject::Door(d) => {
                let mut trans = if d.open { true } else { d.door_trans && (!d.has_curtain || d.curtain_open) };
                if d.barricade_same.is_some_and(|b| b.block_vision()) {
                    trans = false;
                }
                if d.barricade_opposite.is_some_and(|b| b.block_vision()) {
                    trans = false;
                }
                if d.open && d.garage_door {
                    trans = true;
                }
                objects.push(ObjectCall::Door { north: d.north, open: d.open, trans });
                if !d.open && !d.has_curtain {
                    let k = if d.north { n } else { w };
                    intensity[k] = intensity[k].min(0.15);
                }
                if d.open || !d.has_curtain || d.curtain_open {
                    continue;
                }
                let k = if d.north { n } else { w };
                intensity[k] = intensity[k].min(0.33);
                colour[k] = [0.0; 3];
            }
            SpecialObject::Thumpable(t) => {
                let door_trans = t.door_trans || (t.is_door && t.open);
                objects.push(ObjectCall::Thumpable { north: t.north, open: t.open, is_door: t.is_door, door_trans });
                let mut opaque = false;
                if let Some(b) = t.barricade_same {
                    opaque |= b.block_vision();
                    let k = if t.north { n } else { w };
                    intensity[k] = intensity[k].min(b.light_transmission());
                }
                if let Some(b) = t.barricade_opposite {
                    opaque |= b.block_vision();
                    let k = if t.north { s } else { e };
                    intensity[k] = intensity[k].min(b.light_transmission());
                }
                objects.push(ObjectCall::Window { north: t.north, open: t.open, opaque });
            }
            SpecialObject::Window(win) => {
                let mut opaque = false;
                if let Some(b) = win.barricade_same {
                    opaque |= b.block_vision();
                    let k = if win.north { n } else { w };
                    intensity[k] = intensity[k].min(b.light_transmission());
                }
                if let Some(b) = win.barricade_opposite {
                    opaque |= b.block_vision();
                    let k = if win.north { s } else { e };
                    intensity[k] = intensity[k].min(b.light_transmission());
                }
                objects.push(ObjectCall::Window { north: win.north, open: win.open, opaque });
            }
            SpecialObject::Barricade(_) => {}
        }
    }
    let transmissions = std::array::from_fn(|k| Transmission {
        r: colour[k][0],
        g: colour[k][1],
        b: colour[k][2],
        intensity: if intensity[k] == UNSET { 0.0 } else { intensity[k] },
        mix: 0.0,
    });
    SquareArgs {
        vision_unblocked,
        up_clear,
        down_clear,
        elevated_floor: sq.elevated_floor,
        vision_matrix,
        building_id: sq.building_id.unwrap_or(-1),
        room_id: sq.room_id.unwrap_or(-1),
        light_level,
        open_air: is_open_air,
        objects,
        transmissions,
    }
}

/// Sends one square (`squareBeginUpdate` .. `squareEndUpdate`).
fn send_square(n: &mut impl LightingNatives, x: i32, y: i32, z: i32, a: &SquareArgs) -> Result<(), LightingError> {
    n.square_begin_update(x, y, z)?;
    let body = (|| {
        n.square_set(a.vision_unblocked, a.up_clear, a.down_clear, a.elevated_floor, a.vision_matrix, a.building_id, a.room_id, a.light_level, a.open_air)?;
        for o in &a.objects {
            match *o {
                ObjectCall::Curtain { wnes, open } => n.square_add_curtain(wnes, open)?,
                ObjectCall::Door { north, open, trans } => n.square_add_door(north, open, trans)?,
                ObjectCall::Thumpable { north, open, is_door, door_trans } => n.square_add_thumpable(north, open, is_door, door_trans)?,
                ObjectCall::Window { north, open, opaque } => n.square_add_window(north, open, opaque)?,
            }
        }
        n.square_set_light_transmission(a.transmissions)
    })();
    // `finally { squareEndUpdate(); }`
    let end = n.square_end_update();
    body.and(end)
}

/// `LightingJNI.updateChunk(playerIndex, chunk)`. `level_light_check` is
/// `getLevelData(z).lightCheck[playerIndex]`: called once per level from
/// `chunk.min_level` up, it returns the flag and clears it (Java clears it
/// before sending the level). `sky_light_level` is
/// `GameTime.getSkyLightLevel()`.
pub fn update_chunk(
    n: &mut impl LightingNatives,
    world: &impl LightingWorld,
    player: i32,
    chunk: &ChunkInfo,
    sky_light_level: i32,
    mut level_light_check: impl FnMut(i32) -> bool,
) -> Result<(), LightingError> {
    n.chunk_begin_update(chunk.wx, chunk.wy, chunk.min_level + 32, chunk.max_level + 32)?;
    let body = (|| {
        for z in chunk.min_level..=chunk.max_level {
            if !level_light_check(z) {
                continue;
            }
            n.chunk_level_begin_update(z + 32)?;
            let level = (|| {
                for y in 0..8 {
                    for x in 0..8 {
                        match world.square(chunk.wx * 8 + x, chunk.wy * 8 + y, z) {
                            Some(sq) => {
                                let a = square_args(world, sq, player, sky_light_level);
                                send_square(n, x, y, z + 32, &a)?;
                            }
                            None => n.square_set_null(x, y, z + 32)?,
                        }
                    }
                }
                Ok(())
            })();
            let end = n.chunk_level_end_update();
            level.and(end)?;
        }
        Ok(())
    })();
    let end = n.chunk_end_update();
    body.and(end)
}
