//! `LightingJNI.JNILighting`: the per-square, per-player cache the
//! renderer reads, filled from `getSquareLighting`.

use super::input::IsoDirection;
use super::natives::LightingNatives;
use crate::model::fastfloor;
use crate::LightingError;

/// `ForcedVis[forward direction]`: (dx, dy) pairs within two squares in
/// front of the player that count as visible when they are in line of
/// sight.
pub const FORCED_VIS: [[i32; 20]; 8] = [
    [-1, 0, -1, -1, 0, -1, 1, -1, 1, 0, -2, -2, -1, -2, 0, -2, 1, -2, 2, -2],
    [-1, 1, -1, 0, -1, -1, 0, -1, 1, -1, -2, 0, -2, -1, -2, -2, -1, -2, 0, -2],
    [0, 1, -1, 1, -1, 0, -1, -1, 0, -1, -2, 2, -2, 1, -2, 0, -2, -1, -2, -2],
    [1, 1, 0, 1, -1, 1, -1, 0, -1, -1, 0, 2, -1, 2, -2, 2, -2, 1, -2, 0],
    [1, 0, 1, 1, 0, 1, -1, 1, -1, 0, 2, 2, 1, 2, 0, 2, -1, 2, -2, 2],
    [-1, 1, 0, 1, 1, 1, 1, 0, 1, -1, 2, 0, 2, 1, 2, 2, 1, 2, 0, 2],
    [0, 1, 1, 1, 1, 0, 1, -1, 0, -1, 2, -2, 2, -1, 2, 0, 2, 1, 2, 2],
    [-1, -1, 0, -1, 1, -1, 1, 0, 1, 1, 0, -2, 1, -2, 2, -2, 2, -1, 2, 0],
];

/// `ColorInfo`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorInfo {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// `IsoGridSquare.ResultLight`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResultLightInfo {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    /// World level.
    pub z: i32,
    pub radius: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// 0 point light, 1 room light, 2 torch.
    pub flags: i32,
}

/// The player data the decoders use.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DecodePlayer {
    /// `getX()`, `getY()`, `getZ()` (world level).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getForwardIsoDirection()`.
    pub forward: Option<IsoDirection>,
}

/// What a square refresh means for the renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Refresh {
    /// New results were read.
    pub updated: bool,
    /// The chunk level's render texture must be redrawn
    /// (`FBORenderLevels.invalidateLevel(z, 32)`).
    pub invalidate_level: bool,
    /// `couldSee` changed (`FBORenderCutaways.squareChanged`).
    pub could_see_changed: bool,
    /// The square is seen (`checkRoomSeen`).
    pub seen: bool,
    /// The square was seen for the first time (`Meta.dealWithSquareSeen`).
    pub first_seen: bool,
}

/// `JNILighting`: one square's decoded lighting for one player.
#[derive(Clone, Debug, PartialEq)]
pub struct SquareLight {
    /// Visibility bits (1 seen, 2 canSee, 4 couldSee), with `ForcedVis`
    /// applied.
    pub vis: u8,
    /// `lightInfo()`: the square colour.
    pub light_info: ColorInfo,
    pub dark_multi: f32,
    pub target_dark_multi: f32,
    /// `lightverts(i)`: `0xFFbbggrr` (ABGR); 0-3 floor corners, 4-7 the
    /// same corners one level up.
    pub vert_light: [u32; 8],
    /// The propagated light level; Java also stores it in
    /// `IsoGridSquare.lightLevel` (feed it back as
    /// [`super::GridSquare::light_level`]).
    pub light_level: i32,
    pub lights: Vec<ResultLightInfo>,
    update_tick: i32,
}

impl Default for SquareLight {
    fn default() -> Self {
        SquareLight {
            vis: 0,
            light_info: ColorInfo { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            dark_multi: 0.0,
            target_dark_multi: 0.0,
            vert_light: [0xff00_0000; 8],
            light_level: 0,
            lights: Vec::new(),
            update_tick: -1,
        }
    }
}

/// The square position (world level) the cache belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SquarePos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl SquareLight {
    pub fn new() -> SquareLight {
        SquareLight::default()
    }

    pub fn seen(&self) -> bool {
        self.vis & 1 != 0
    }

    pub fn can_see(&self) -> bool {
        self.vis & 2 != 0
    }

    pub fn could_see(&self) -> bool {
        self.vis & 4 != 0
    }

    /// `bSeen(boolean)`, the only setter Java allows.
    pub fn set_seen(&mut self, seen: bool) {
        self.vis = if seen { self.vis | 1 } else { self.vis & 0xfe };
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.update_tick = -1;
        self.vert_light = [0xff00_0000; 8];
        self.vis = 0;
        self.dark_multi = 0.0;
        self.target_dark_multi = 0.0;
        self.light_level = 0;
        self.lights.clear();
        self.light_info = ColorInfo { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    }

    fn forced_vis(&mut self, pos: SquarePos, player: Option<&DecodePlayer>) {
        let Some(p) = player else { return };
        if self.vis & 2 != 0 || self.vis & 4 == 0 {
            return;
        }
        let dx = pos.x - fastfloor(p.x);
        let dy = pos.y - fastfloor(p.y);
        let Some(f) = p.forward else { return };
        if dx.abs() > 2 || dy.abs() > 2 {
            return;
        }
        let fv = &FORCED_VIS[f as usize];
        if fv.chunks(2).any(|c| c[0] == dx && c[1] == dy) {
            self.vis |= 2;
        }
    }

    fn read_lights(&mut self, ints: &[i32; 49], mut k: usize) {
        let count = ints[k].max(0) as usize;
        k += 1;
        self.lights.clear();
        for _ in 0..count.min(5) {
            let rgb = ints[k + 5];
            self.lights.push(ResultLightInfo {
                id: ints[k],
                x: ints[k + 1],
                y: ints[k + 2],
                z: ints[k + 3] - 32,
                radius: ints[k + 4],
                r: (rgb & 0xff) as f32 / 255.0,
                g: ((rgb >> 8) & 0xff) as f32 / 255.0,
                b: ((rgb >> 16) & 0xff) as f32 / 255.0,
                flags: (rgb >> 24) & 0xff,
            });
            k += 6;
        }
    }

    fn read_head(&mut self, ints: &[i32; 49]) {
        self.vis = (ints[0] & 7) as u8;
        let c = ints[1];
        self.light_info = ColorInfo {
            r: (c & 0xff) as f32 / 255.0,
            g: ((c >> 8) & 0xff) as f32 / 255.0,
            b: ((c >> 16) & 0xff) as f32 / 255.0,
            a: 1.0,
        };
        self.dark_multi = ints[2] as f32 / 100000.0;
        self.target_dark_multi = ints[3] as f32 / 100000.0;
        self.light_level = ints[4];
    }

    /// `JNILighting.updateFBORenderChunk()`: the FBO renderer's path (raw
    /// vertex lights). `update_counter` is `LightingJNI.updateCounter[p]`,
    /// `on_screen` is `FBORenderLevels.isOnScreen(z)`.
    pub fn update_fbo(
        &mut self,
        n: &mut impl LightingNatives,
        player_index: i32,
        pos: SquarePos,
        update_counter: i32,
        player: Option<&DecodePlayer>,
        on_screen: bool,
    ) -> Result<Refresh, LightingError> {
        let mut out = Refresh::default();
        if update_counter == -1 || self.update_tick == update_counter {
            return Ok(out);
        }
        if !n.get_square_dirty(player_index, pos.x, pos.y, pos.z + 32)? {
            return Ok(out);
        }
        let mut ints = [0i32; 49];
        if !n.get_square_lighting(player_index, pos.x, pos.y, pos.z + 32, &mut ints)? {
            return Ok(out);
        }
        out.updated = true;
        let was_could_see = self.vis & 4 != 0;
        let was_seen = self.vis & 1 != 0;
        let byte = |v: f32| (v * 255.0) as i32;
        let was = (
            [byte(self.light_info.r), byte(self.light_info.g), byte(self.light_info.b)],
            (self.dark_multi * 1000.0) as i32,
            (self.target_dark_multi * 1000.0) as i32,
            self.light_level,
            self.vert_light,
        );
        self.read_head(&ints);
        self.forced_vis(pos, player);
        for i in 0..8 {
            self.vert_light[i] = ints[5 + i] as u32;
        }
        let is = (
            [byte(self.light_info.r), byte(self.light_info.g), byte(self.light_info.b)],
            (self.dark_multi * 1000.0) as i32,
            (self.target_dark_multi * 1000.0) as i32,
            self.light_level,
            self.vert_light,
        );
        if is.0 != was.0 || is.1 != was.1 || is.2 != was.2 || is.3 != was.3 || is.4 != was.4 {
            out.invalidate_level = true;
        }
        out.could_see_changed = was_could_see != (self.vis & 4 != 0);
        self.read_lights(&ints, 13);
        if self.update_tick == -1 && on_screen {
            out.invalidate_level = true;
        }
        self.update_tick = update_counter;
        out.seen = self.vis & 1 != 0;
        out.first_seen = out.seen && !was_seen;
        Ok(out)
    }

    /// `JNILighting.update()` without FBO rendering: vertex lights of the
    /// levels below the player are scaled by 0.85 (the level directly below
    /// only on its floor corners). (Java's `playerIndex == -1` variant,
    /// `getLightLevel2`, decodes whatever the shared buffer last held and is
    /// not reproduced.)
    pub fn update_legacy(
        &mut self,
        n: &mut impl LightingNatives,
        player_index: i32,
        pos: SquarePos,
        update_counter: i32,
        player: Option<&DecodePlayer>,
    ) -> Result<Refresh, LightingError> {
        let mut out = Refresh::default();
        if update_counter == -1 || self.update_tick == update_counter {
            return Ok(out);
        }
        if !n.get_square_dirty(player_index, pos.x, pos.y, pos.z + 32)? {
            return Ok(out);
        }
        let mut ints = [0i32; 49];
        if !n.get_square_lighting(player_index, pos.x, pos.y, pos.z + 32, &mut ints)? {
            return Ok(out);
        }
        out.updated = true;
        let was_seen = self.vis & 1 != 0;
        self.read_head(&ints);
        let mut upper = 1.0f32;
        let mut lower = 1.0f32;
        if let Some(p) = player {
            let dz = pos.z - fastfloor(p.z);
            if dz == -1 {
                lower = 0.85;
            } else if dz < -1 {
                upper = 0.85;
                lower = 0.85;
            }
        }
        self.forced_vis(pos, player);
        for i in 0..8 {
            let m = if i < 4 { lower } else { upper };
            let col = ints[5 + i];
            let r = (col & 0xff) as f32 * m;
            let g = ((col & 0xff00) >> 8) as f32 * m;
            let b = ((col & 0xff_0000) >> 16) as f32 * m;
            self.vert_light[i] = (r as i32 as u32) | ((g as i32 as u32) << 8) | ((b as i32 as u32) << 16) | 0xff00_0000;
        }
        self.read_lights(&ints, 13);
        self.update_tick = update_counter;
        out.seen = self.vis & 1 != 0;
        out.first_seen = out.seen && !was_seen;
        Ok(out)
    }
}

/// The lighting of one chunk level (64 squares, index `x + y * 8`) for one
/// player, with the renderer's invalidation signal.
#[derive(Clone, Debug)]
pub struct ChunkLevelLight {
    pub wx: i32,
    pub wy: i32,
    /// World level.
    pub z: i32,
    pub squares: Vec<SquareLight>,
}

/// What [`ChunkLevelLight::refresh`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelRefresh {
    /// Redraw this chunk level's texture.
    pub invalidate: bool,
    /// Squares (index `x + y * 8`) whose `couldSee` changed.
    pub could_see_changed: Vec<usize>,
    /// Squares seen for the first time.
    pub first_seen: Vec<usize>,
}

impl ChunkLevelLight {
    pub fn new(wx: i32, wy: i32, z: i32) -> ChunkLevelLight {
        ChunkLevelLight { wx, wy, z, squares: vec![SquareLight::default(); 64] }
    }

    /// Runs [`SquareLight::update_fbo`] for every square that `exists`.
    pub fn refresh(
        &mut self,
        n: &mut impl LightingNatives,
        player_index: i32,
        update_counter: i32,
        player: Option<&DecodePlayer>,
        on_screen: bool,
        exists: impl Fn(usize) -> bool,
    ) -> Result<LevelRefresh, LightingError> {
        let mut out = LevelRefresh::default();
        for i in 0..64 {
            if !exists(i) {
                continue;
            }
            let pos = SquarePos { x: self.wx * 8 + (i % 8) as i32, y: self.wy * 8 + (i / 8) as i32, z: self.z };
            let r = self.squares[i].update_fbo(n, player_index, pos, update_counter, player, on_screen)?;
            out.invalidate |= r.invalidate_level;
            if r.could_see_changed {
                out.could_see_changed.push(i);
            }
            if r.first_seen {
                out.first_seen.push(i);
            }
        }
        Ok(out)
    }
}
