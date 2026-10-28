//! `FBORenderCell.renderFog`: `ImprovedFog` over levels 0 and 1, drawn
//! after the per-level translucent passes (distinct from `IsoWeatherFX`'s
//! fog layer).
//!
//! `ImprovedFog` walks the view's squares in diagonal rows (constant
//! `x + y`, [`RectangleIterator`]) and joins runs of fogged squares
//! (outdoors, or with no square loaded) into screen rectangles, one per row
//! run, every row at high fog quality and every other row at low. The
//! rows are walked as `renderFog` meets each square holding a `MinusFloor`
//! object (`renderRowsBehind`) and finished by `endRender`; every
//! rectangle lands in one `ImprovedFogDrawer`, drawn once with the `fog`
//! shader ([`pz_render::DrawCmd::ImprovedFog`]).
//!
//! The fog's state ([`ImprovedFog`]) is updated once per game tick by
//! `IsoWorld.update` (`ImprovedFog.update`) from the climate; the frame
//! reads it with the player and camera ([`FogFrame`]).

use std::collections::HashMap;

use pz_render::iso::{fastfloor, square_depth, TILE_SCALE};
use pz_render::{DrawCmd, FogDraw, FogRect, Texture};

use crate::cell::{ChunkRenderer, Layer};
use crate::frame::{ChunkLevels, FrameSize};
use crate::weather_fx::SteppedUpdateFloat;
use crate::world::RenderSquare;
use crate::Camera;

/// `ImprovedFog.minXOffset`, `maxXOffset`, `maxYOffset`.
const MIN_X_OFFSET: i32 = -2;
const MAX_X_OFFSET: i32 = 12;
const MAX_Y_OFFSET: i32 = -5;

/// `ImprovedFog.offsets`: each row's noise offset, by `|row| % 30`.
pub const OFFSETS: [f32; 30] = [
    0.3, 0.8, 0.0, 0.6, 0.3, 0.1, 0.5, 0.9, 0.2, 0.0, 0.7, 0.1, 0.4, 0.2, 0.5, 0.3, 0.8, 0.4, 0.9,
    0.5, 0.8, 0.4, 0.7, 0.2, 0.0, 0.6, 0.1, 0.6, 0.9, 0.7,
];

/// What `ImprovedFog.update` reads each tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogClimate {
    /// `ClimateManager.getFogIntensity()`.
    pub fog_intensity: f32,
    /// `ClimateManager.getColorNewFog().getExterior()` (r, g, b).
    pub color: [f32; 3],
    /// `ClimateManager.getWindAngleRadians()`.
    pub wind_angle_radians: f32,
    /// `ClimateManager.getWindIntensity()`.
    pub wind_intensity: f32,
    /// `GameTime.getMultiplier()`.
    pub multiplier: f32,
    /// `PerformanceSettings.fogQuality`: 0 high, 1 low, 2 off.
    pub fog_quality: i32,
}

/// `ImprovedFog`'s static state, as `update` leaves it (no editing).
#[derive(Clone, Debug, PartialEq)]
pub struct ImprovedFog {
    /// `fogIntensity`: steps 0.005 per multiplier unit towards the climate.
    pub fog_intensity: SteppedUpdateFloat,
    pub base_alpha: f32,
    pub high_quality: bool,
    pub render_every_x_row: i32,
    pub top_alpha_height: f32,
    pub bottom_alpha_height: f32,
    pub octaves: f32,
    pub second_layer_alpha: f32,
    pub color: [f32; 3],
    /// The wind scroll (`scalingX/Y`).
    pub scaling_x: f32,
    pub scaling_y: f32,
}

impl Default for ImprovedFog {
    /// The class's static initial values.
    fn default() -> Self {
        ImprovedFog {
            fog_intensity: SteppedUpdateFloat::new(0.0, 0.005, 0.0, 1.0),
            base_alpha: 0.0,
            high_quality: true,
            render_every_x_row: 1,
            top_alpha_height: 0.38,
            bottom_alpha_height: 0.24,
            octaves: 6.0,
            second_layer_alpha: 0.5,
            color: [1.0; 3],
            scaling_x: 1.0,
            scaling_y: 1.0,
        }
    }
}

impl ImprovedFog {
    /// `init()` (`IsoWorld.init`): the intensity jumps to the climate's.
    pub fn init(&mut self, fog_intensity: f32) {
        self.fog_intensity.set_target(fog_intensity);
        self.fog_intensity.override_current_value(fog_intensity);
        self.base_alpha = self.fog_intensity.value();
    }

    /// `update()` (`IsoWorld.update`, once per tick).
    pub fn update(&mut self, c: &FogClimate) {
        self.high_quality = c.fog_quality == 0;
        self.fog_intensity.set_target(c.fog_intensity);
        self.fog_intensity.update(c.multiplier);
        self.base_alpha = self.fog_intensity.value();
        if self.high_quality {
            self.render_every_x_row = 1;
            self.top_alpha_height = 0.38;
            self.bottom_alpha_height = 0.24;
            self.octaves = 6.0;
            self.second_layer_alpha = 0.5;
        } else {
            self.render_every_x_row = 2;
            self.top_alpha_height = 0.32;
            self.bottom_alpha_height = 0.32;
            self.octaves = 3.0;
            self.second_layer_alpha = 1.0;
        }
        self.color = c.color;
        if self.base_alpha <= 0.0 {
            self.scaling_x = 0.0;
            self.scaling_y = 0.0;
        } else {
            let mut rad = c.wind_angle_radians as f64;
            rad -= 2.356_194_490_192_345;
            rad = std::f64::consts::PI - rad;
            let x = rad.cos() as f32;
            let y = rad.sin() as f32;
            self.scaling_x += x * c.wind_intensity * c.multiplier;
            self.scaling_y += y * c.wind_intensity * c.multiplier;
        }
    }
}

/// The viewing player as `startRender` reads it (`IsoPlayer.players[n]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FogPlayer {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `isInARoom()`.
    pub in_room: bool,
    /// `getVehicle() != null`.
    pub in_vehicle: bool,
}

/// One frame's `ImprovedFog` inputs.
#[derive(Clone, Debug, Default)]
pub struct FogFrame {
    /// The state `ImprovedFog.update` left (owner: the weather update).
    pub fog: ImprovedFog,
    /// `PerformanceSettings.fogQuality` (owner: the options).
    pub fog_quality: i32,
    /// `media/textures/weather/fognew/fog_noise.png` (`NoiseTexture`).
    pub noise: Option<Texture>,
    /// The viewing player (owner: the player).
    pub player: FogPlayer,
    /// `ClimateManager.getWeatherPeriod()` is running a tropical storm or
    /// thunderstorm: the clear circle about the player shrinks.
    pub storm: bool,
    /// `PlayerCamera.rightClickX + XToScreen(deferedX, deferedY)` and the
    /// y counterpart (owner: the camera; 0 without a right-click look).
    pub right_click_off: [f32; 2],
    /// `IsoCamera.getOffscreenLeft/Top` (split screen; 0 for player 0).
    pub offscreen_left: i32,
    pub offscreen_top: i32,
}

/// A square as `renderRowsBehind` tests it (`chunkMap.getGridSquare`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FogSquare {
    /// `isExteriorCache`.
    pub exterior: bool,
    /// `isInARoom()`.
    pub in_room: bool,
}

impl FogSquare {
    /// A rendered square: exterior when its objects' sprites say so (the
    /// square's merged properties), in a room when it has one.
    pub fn of(sq: &RenderSquare) -> Self {
        FogSquare {
            exterior: sq.objects.iter().any(|o| o.flags().exterior),
            in_room: sq.cut.room.is_some(),
        }
    }
}

/// `PZMath.ceil(float)`: `(int)(val + 0.9999999f)` for positive values,
/// which rounds whole numbers from 2 up one higher (the sum rounds up in
/// float), as the original's rows are.
fn pz_ceil(val: f32) -> f32 {
    if val >= 0.0 {
        (val + 0.999_999_9) as i32 as f32
    } else {
        (val - 1.0e-7) as i32 as f32
    }
}

/// `ImprovedFog.RectangleIterator`: the squares of a diamond walked row by
/// row, alternate rows one longer and one shorter.
#[derive(Clone, Debug, Default)]
pub struct RectangleIterator {
    cur_x: i32,
    cur_y: i32,
    s_x: i32,
    s_y: i32,
    row_len: i32,
    alt_row: bool,
    cur_row: i32,
    row_index: i32,
    max_rows: i32,
}

impl RectangleIterator {
    pub fn reset(&mut self, rows: i32, rowlen: i32) {
        *self = RectangleIterator {
            row_len: pz_ceil(rowlen as f32 / 2.0) as i32,
            max_rows: rows,
            ..Default::default()
        };
    }

    /// `next(vec)`: the next position, and whether to go on (false with
    /// the last position of the last row, which `renderRowsBehind` then
    /// skips).
    pub fn advance(&mut self) -> (bool, (i32, i32)) {
        if self.row_len <= 0 || self.max_rows <= 0 || self.cur_row >= self.max_rows {
            return (false, (0, 0));
        }
        let v = (self.cur_x, self.cur_y);
        self.row_index += 1;
        if self.row_index == self.row_len {
            self.row_len = if self.alt_row {
                self.row_len - 1
            } else {
                self.row_len + 1
            };
            self.row_index = 0;
            self.s_x = if self.alt_row { self.s_x + 1 } else { self.s_x };
            self.s_y = if self.alt_row { self.s_y } else { self.s_y + 1 };
            self.alt_row = !self.alt_row;
            self.cur_x = self.s_x;
            self.cur_y = self.s_y;
            self.cur_row += 1;
            return (self.cur_row != self.max_rows, v);
        }
        self.cur_x += 1;
        self.cur_y -= 1;
        (true, v)
    }
}

/// `IsoUtils.XToScreen` / `YToScreen` in the original's order of
/// operations.
fn x_to_screen(x: f32, y: f32) -> f32 {
    let mut sx = 0.0;
    sx += x * (32 * TILE_SCALE) as f32;
    sx - y * (32 * TILE_SCALE) as f32
}

fn y_to_screen(x: f32, y: f32, z: f32) -> f32 {
    let mut sy = 0.0;
    sy += y * (16 * TILE_SCALE) as f32;
    sy += x * (16 * TILE_SCALE) as f32;
    sy + (0.0 - z) * (96 * TILE_SCALE) as f32
}

/// `ImprovedFog`'s per-level render state (`startRender` .. `endRender`).
pub struct FogLevel<'a> {
    fog: &'a ImprovedFog,
    cam: Camera,
    lookup: &'a dyn Fn(i32, i32, i32) -> Option<FogSquare>,
    iter: RectangleIterator,
    min_x: i32,
    min_y: i32,
    z_layer: i32,
    player_row: i32,
    last_row: i32,
    last_iter: (i32, i32),
    /// `fogRectangle`'s start and `hasStarted`.
    start: (i32, i32),
    started: bool,
    // The uniform sources startFrame copies.
    zoom: f32,
    screen: [f32; 2],
    world_offset: [f32; 4],
    alpha_circle: [f32; 2],
    camera_offscreen: [f32; 2],
}

impl<'a> FogLevel<'a> {
    /// `startRender(nPlayer, z)`: `None` when level `z` takes no fog.
    pub fn start(
        frame: &'a FogFrame,
        cam: &Camera,
        size: FrameSize,
        z: i32,
        lookup: &'a dyn Fn(i32, i32, i32) -> Option<FogSquare>,
    ) -> Option<Self> {
        let fog = &frame.fog;
        if z > 1 || fog.base_alpha <= 0.0 || frame.fog_quality == 2 {
            return None;
        }
        let p = &frame.player;
        if p.in_room && z > 0 {
            return None;
        }
        let zoom = cam.zoom;
        let (ow, oh) = (
            (size.width as f32 * zoom) as i32,
            (size.height as f32 * zoom) as i32,
        );
        let (off_x, off_y) = (cam.off_x as f32, cam.off_y as f32);
        let world_offset = [
            off_x - frame.offscreen_left as f32 * zoom,
            off_y + frame.offscreen_top as f32 * zoom,
            frame.right_click_off[0],
            frame.right_click_off[1],
        ];
        let hq = fog.high_quality;
        let alpha_circle = if p.in_vehicle {
            [0.0, if hq { 2.0 } else { 2.6 }]
        } else if p.in_room {
            [0.0, if hq { 1.25 } else { 1.5 }]
        } else {
            let mut rad: f32 = if hq { 2.5 } else { 3.0 };
            if frame.storm {
                rad *= 0.6;
            }
            [if hq { 0.1 } else { 0.16 }, rad]
        };
        let to_iso_x = |sx: f32, sy: f32| {
            ((sx + off_x) + 2.0 * (sy + off_y)) / (64.0 * TILE_SCALE as f32) + 3.0 * z as f32
        };
        let to_iso_y = |sx: f32, sy: f32| {
            ((sx + off_x) - 2.0 * (sy + off_y)) / (-64.0 * TILE_SCALE as f32) + 3.0 * z as f32
        };
        let min_y = fastfloor(to_iso_y(0.0, 0.0));
        let mut max_y = fastfloor(to_iso_y(ow as f32, oh as f32));
        let mut min_x = fastfloor(to_iso_x(0.0, 0.0));
        let mut max_x = fastfloor(to_iso_x(ow as f32, oh as f32));
        max_x += MAX_X_OFFSET;
        min_x += MIN_X_OFFSET;
        let rows = max_x - min_x;
        let mut rowlen = rows;
        max_y += MAX_Y_OFFSET;
        if min_y != max_y {
            rowlen += (min_y - max_y).abs();
        }
        let mut iter = RectangleIterator::default();
        iter.reset(rows, rowlen);
        Some(FogLevel {
            fog,
            cam: *cam,
            lookup,
            iter,
            min_x,
            min_y,
            z_layer: z,
            player_row: fastfloor(p.x) + fastfloor(p.y),
            last_row: i32::MAX,
            last_iter: (0, 0),
            start: (0, 0),
            started: false,
            zoom,
            screen: [ow as f32, oh as f32],
            world_offset,
            alpha_circle,
            camera_offscreen: [frame.offscreen_left as f32, frame.offscreen_top as f32],
        })
    }

    /// `ImprovedFog.startFrame(drawer)`: the drawer's uniforms from this
    /// level's state.
    pub fn start_frame(&self, noise: Option<Texture>) -> FogDraw {
        let f = self.fog;
        FogDraw {
            noise,
            screen: [self.screen[0], self.screen[1], self.zoom],
            texture_info: [0.0, f.octaves, f.base_alpha, TILE_SCALE as f32],
            world_offset: self.world_offset,
            scaling: [
                f.scaling_x,
                f.scaling_y,
                if f.high_quality { 1.0 } else { 0.0 },
            ],
            color_info: [f.color[0], f.color[1], f.color[2], 1.0],
            param_info: [
                f.top_alpha_height,
                f.bottom_alpha_height,
                self.alpha_circle[0],
                self.alpha_circle[1],
            ],
            camera: [
                self.camera_offscreen[0],
                self.camera_offscreen[1],
                self.zoom,
            ],
            alpha: f.base_alpha,
            rects: Vec::new(),
        }
    }

    /// `renderRowsBehind(squareMax)`: walks the rows up to (the start of)
    /// square `(x, y)`'s row, or to the end with `None`.
    pub fn render_rows_behind(&mut self, square: Option<(i32, i32)>, out: &mut Vec<FogRect>) {
        let mut max_row = -1;
        if let Some((x, y)) = square {
            max_row = x + y;
            if max_row < self.min_x + self.min_y {
                return;
            }
        }
        if self.last_row != i32::MIN && self.last_row == max_row {
            return;
        }
        loop {
            let (go, (vx, vy)) = self.iter.advance();
            if !go {
                break;
            }
            let (x, y) = (vx + self.min_x, vy + self.min_y);
            let cur_row = x + y;
            if cur_row != self.last_row {
                if self.last_row != i32::MIN {
                    self.end_rectangle(out);
                }
                self.last_row = cur_row;
            }
            let has_fog =
                (self.lookup)(x, y, self.z_layer).is_none_or(|s| s.exterior && !s.in_room);
            if has_fog {
                if !self.started {
                    self.started = true;
                    self.start = (x, y);
                }
            } else {
                self.end_rectangle(out);
            }
            self.last_iter = (x, y);
            if max_row == -1 || cur_row != max_row {
                continue;
            }
            break;
        }
    }

    /// `endRender()`.
    pub fn end(&mut self, out: &mut Vec<FogRect>) {
        self.render_rows_behind(None, out);
        if self.started {
            self.end_rectangle(out);
        }
    }

    /// `endFogRectangle(lastIterPos)`.
    fn end_rectangle(&mut self, out: &mut Vec<FogRect>) {
        if !self.started {
            return;
        }
        self.started = false;
        if let Some(r) = self.segment(self.start, self.last_iter) {
            out.push(r);
        }
    }

    /// `renderFogSegment()` (the `fboRenderChunk` path).
    fn segment(
        &self,
        (start_x, start_y): (i32, i32),
        (end_x, end_y): (i32, i32),
    ) -> Option<FogRect> {
        let row = start_x + start_y;
        if row % self.fog.render_every_x_row != 0 {
            return None;
        }
        let z = self.z_layer;
        let (off_x, off_y) = (self.cam.off_x as f32, self.cam.off_y as f32);
        let mut sx = x_to_screen(start_x as f32, start_y as f32) - off_x;
        let mut sy = y_to_screen(start_x as f32, start_y as f32, z as f32) - off_y;
        let mut ex = x_to_screen(end_x as f32, end_y as f32) - off_x;
        let ey = y_to_screen(end_x as f32, end_y as f32, z as f32) - off_y;
        let ts = TILE_SCALE as f32;
        sy -= 80.0 * ts;
        let height = 96.0 * ts;
        ex += 32.0 * ts;
        sx -= 32.0 * ts;
        let width_tiles = (ex - sx) / (64.0 * ts);
        let offset = start_x as f32 % 6.0;
        let uv_offset = offset / 6.0;
        let uv_width = width_tiles / 6.0;
        let (u0, u1) = (uv_offset, uv_width + uv_offset);
        let mx = sx + (ex - sx) / 2.0;
        let my = sy + (ey - sy) / 2.0;
        let x = ((mx + off_x) + 2.0 * (my + off_y)) / (64.0 * ts) + 3.0 * z as f32;
        let y = ((mx + off_x) - 2.0 * (my + off_y)) / (-64.0 * ts) + 3.0 * z as f32;
        let (xm, ym) = (x + 1.5, y + 1.5);
        let (cx, cy) = (fastfloor(self.cam.x), fastfloor(self.cam.y));
        let far = square_depth(cx, cy, xm - 1.0, ym - 1.0, z as f32);
        let front = square_depth(cx, cy, xm, ym, z as f32);
        let far2 = square_depth(cx, cy, xm - 1.0, ym - 1.0, (z + 1) as f32);
        let front2 = square_depth(cx, cy, xm, ym, (z + 1) as f32);
        let calc_z = far - front + front;
        let calc_z2 = far2 - front2 + front2;
        let depth_bottom = calc_z2 - calc_z + calc_z;
        let depth_top = depth_bottom - (ey - sy) / 96.0 * 0.002_886_708_4 * 0.6;
        Some(FogRect {
            pos: [sx, sy, ex, sy + height],
            uv: [u0, 0.0, u1, 1.0],
            depth: [depth_top, depth_top, depth_bottom, depth_bottom],
            offset: OFFSETS[(row.unsigned_abs() % OFFSETS.len() as u32) as usize],
            layer_alpha: if z > 0 {
                self.fog.second_layer_alpha
            } else {
                1.0
            },
            z_layer: z,
        })
    }

    /// The player's row (`playerRow`, for the debug `renderOnlyOneRow`).
    pub fn player_row(&self) -> i32 {
        self.player_row
    }
}

/// `renderFog` over explicit inputs: for each level that takes fog,
/// `behind(z)` lists the squares (in `renderFog`'s chunk and square order)
/// holding a `MinusFloor` object, and `lookup` answers
/// `chunkMap.getGridSquare` (`None`: no square, which is fogged). `None`
/// when no rectangle was added.
pub fn fog_draw(
    frame: &FogFrame,
    cam: &Camera,
    size: FrameSize,
    behind: &dyn Fn(i32) -> Vec<(i32, i32)>,
    lookup: &dyn Fn(i32, i32, i32) -> Option<FogSquare>,
) -> Option<FogDraw> {
    if cam.z < 0.0 || frame.fog_quality == 2 {
        return None;
    }
    let mut draw: Option<FogDraw> = None;
    let mut rects = Vec::new();
    for z in 0..=1 {
        let Some(mut level) = FogLevel::start(frame, cam, size, z, lookup) else {
            continue;
        };
        if draw.is_none() {
            draw = Some(level.start_frame(frame.noise.clone()));
        }
        for sq in behind(z) {
            level.render_rows_behind(Some(sq), &mut rects);
        }
        level.end(&mut rects);
    }
    let mut d = draw?;
    if rects.is_empty() {
        return None;
    }
    d.rects = rects;
    Some(d)
}

/// `renderFog(playerIndex)`: the fog over levels 0 and 1 of the on-screen
/// chunks, read from the frame's squares (and the cut-away view's world
/// for squares off screen).
pub fn render_fog(
    r: &ChunkRenderer<'_>,
    fog: &FogFrame,
    chunks: &[ChunkLevels<'_>],
    size: FrameSize,
    out: &mut Vec<DrawCmd>,
) {
    let cv = r.cutaways.as_ref();
    let mut known: HashMap<(i32, i32, i32), FogSquare> = HashMap::new();
    for c in chunks {
        for (_, l) in &c.levels {
            for sq in l.iter().flatten() {
                known.insert((sq.x, sq.y, sq.z), FogSquare::of(sq));
            }
        }
    }
    let lookup = |x: i32, y: i32, z: i32| {
        known.get(&(x, y, z)).copied().or_else(|| {
            cv.and_then(|cv| cv.world.square(x, y, z))
                .map(FogSquare::of)
        })
    };
    let behind = |z: i32| {
        let mut v = Vec::new();
        // perPlayerData.onScreenChunks: each chunk once, its level z.
        let mut seen = Vec::new();
        for c in chunks {
            let k = (c.texture.wx, c.texture.wy);
            let Some((_, squares)) = c.levels.iter().find(|(l, _)| *l == z) else {
                continue;
            };
            if seen.contains(&k) {
                continue;
            }
            seen.push(k);
            for sq in squares.iter().flatten() {
                if cv.is_some_and(|cv| !cv.state.should_render_square(sq.x, sq.y, sq.z)) {
                    continue;
                }
                if sq
                    .objects
                    .iter()
                    .any(|o| crate::alpha::render_layer(cv, sq, o) == Layer::MinusFloor)
                {
                    v.push((sq.x, sq.y));
                }
            }
        }
        v
    };
    if let Some(d) = fog_draw(fog, &r.cam, size, &behind, &lookup) {
        out.push(DrawCmd::ImprovedFog(Box::new(d)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pz_ceil_rounds_large_whole_numbers_up() {
        assert_eq!(pz_ceil(1.0), 1.0);
        assert_eq!(pz_ceil(3.0), 4.0);
        assert_eq!(pz_ceil(19.5), 20.0);
        assert_eq!(pz_ceil(27.0), 28.0);
    }

    #[test]
    fn iterator_walks_alternating_rows() {
        let mut it = RectangleIterator::default();
        it.reset(4, 6);
        let mut v = Vec::new();
        loop {
            let (go, p) = it.advance();
            if !go {
                break;
            }
            v.push(p);
        }
        // ceil(6 / 2) is 4 here (`PZMath.ceil`): rows of 4, 5, 4, then 5
        // with its last position cut.
        assert_eq!(&v[..4], &[(0, 0), (1, -1), (2, -2), (3, -3)]);
        assert_eq!(&v[4..9], &[(0, 1), (1, 0), (2, -1), (3, -2), (4, -3)]);
        assert_eq!(&v[9..13], &[(1, 1), (2, 0), (3, -1), (4, -2)]);
        assert_eq!(v.len(), 17);
    }

    fn frame(alpha: f32) -> FogFrame {
        let mut fog = ImprovedFog::default();
        fog.init(alpha);
        FogFrame {
            fog,
            ..Default::default()
        }
    }

    fn view() -> (Camera, FrameSize) {
        (
            Camera::centred(50.0, 50.0, 0.0, 800, 600, 1.0),
            FrameSize {
                width: 800,
                height: 600,
            },
        )
    }

    #[test]
    fn no_fog_without_intensity_or_at_quality_two() {
        let (cam, size) = view();
        let none = |_: i32, _: i32, _: i32| None;
        assert!(fog_draw(&frame(0.0), &cam, size, &|_| Vec::new(), &none).is_none());
        let mut f = frame(1.0);
        f.fog_quality = 2;
        assert!(fog_draw(&f, &cam, size, &|_| Vec::new(), &none).is_none());
        let d = fog_draw(&frame(1.0), &cam, size, &|_| Vec::new(), &none).expect("fog");
        // Levels 0 and 1, every row one rectangle when nothing is indoors.
        assert!(d.rects.iter().any(|r| r.z_layer == 0) && d.rects.iter().any(|r| r.z_layer == 1));
        assert!(d
            .rects
            .iter()
            .all(|r| r.pos[3] - r.pos[1] == 192.0 && r.depth[0] < r.depth[2]));
        assert_eq!(
            d.rects.iter().find(|r| r.z_layer == 1).unwrap().layer_alpha,
            0.5
        );
    }

    #[test]
    fn indoor_squares_split_rows_and_a_room_drops_level_one() {
        let (cam, size) = view();
        let none = |_: i32, _: i32, _: i32| None;
        let room = |x: i32, y: i32, _: i32| {
            ((45..55).contains(&x) && (45..55).contains(&y)).then_some(FogSquare {
                exterior: false,
                in_room: true,
            })
        };
        let open = fog_draw(&frame(1.0), &cam, size, &|_| Vec::new(), &none).unwrap();
        let split = fog_draw(&frame(1.0), &cam, size, &|_| Vec::new(), &room).unwrap();
        assert!(split.rects.len() > open.rects.len());
        let mut f = frame(1.0);
        f.player.in_room = true;
        let d = fog_draw(&f, &cam, size, &|_| Vec::new(), &room).unwrap();
        assert!(d.rects.iter().all(|r| r.z_layer == 0));
        assert_eq!(d.param_info[2..], [0.0, 1.25]);
    }

    #[test]
    fn walking_rows_square_by_square_gives_the_same_rectangles() {
        let (cam, size) = view();
        let room = |x: i32, y: i32, _: i32| {
            ((45..55).contains(&x) && (40..52).contains(&y)).then_some(FogSquare {
                exterior: false,
                in_room: true,
            })
        };
        let all = fog_draw(&frame(1.0), &cam, size, &|_| Vec::new(), &room).unwrap();
        let some = fog_draw(
            &frame(1.0),
            &cam,
            size,
            &|_| vec![(40, 40), (30, 30), (60, 61), (60, 61), (70, 70)],
            &room,
        )
        .unwrap();
        assert_eq!(all.rects, some.rects);
    }

    #[test]
    fn update_steps_towards_the_climate_and_scrolls_with_the_wind() {
        let mut f = ImprovedFog::default();
        let c = FogClimate {
            fog_intensity: 1.0,
            color: [0.9, 0.9, 0.95],
            wind_angle_radians: 1.0,
            wind_intensity: 0.5,
            multiplier: 2.0,
            fog_quality: 1,
        };
        f.update(&c);
        assert_eq!(f.base_alpha, 0.01);
        assert_eq!(
            (f.render_every_x_row, f.octaves, f.second_layer_alpha),
            (2, 3.0, 1.0)
        );
        assert_ne!((f.scaling_x, f.scaling_y), (1.0, 1.0));
    }
}
