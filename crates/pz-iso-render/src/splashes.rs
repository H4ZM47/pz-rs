//! Rain splashes (`IsoChunkLevel.updateRainSplashes` /
//! `renderRainSplashes`, `IsoGridSquare.renderRainSplash`) on outdoor
//! floors, drawn in the per-level pass, and fish splashes
//! (`IsoGridSquare.WaterSplashData`, `renderFishSplash`) on water, drawn
//! after the water.
//!
//! Both keep state across frames, which the caller owns ([`RainSplashes`],
//! [`FishSplashes`]) and lends to [`crate::weather::Surfaces`]. Random
//! choices come from a small generator in the state; the original's
//! `Rand` is an unseeded Java generator, so only the distribution matters.

use std::collections::HashMap;

use pz_render::iso::{fastfloor, square_depth, x_to_screen, y_to_screen, TILE_SCALE};
use pz_render::{gl, DrawCmd, Rgba, WorldShader};

use crate::cell::ChunkRenderer;
use crate::texd::TexDraw;
use crate::weather::Surfaces;
use crate::world::RenderSquare;

/// The `IsoCamera.frameState` values the splashes read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameState {
    /// `frameCount`.
    pub frame_count: i32,
    pub paused: bool,
    /// `unPausedAccumulator`: frames (at 60 a second) of unpaused time.
    pub unpaused_accumulator: f32,
}

/// A xorshift generator standing in for `Rand`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplashRng(u64);

impl Default for SplashRng {
    fn default() -> Self {
        SplashRng(0x9E37_79B9_7F4A_7C15)
    }
}

impl SplashRng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// `Rand.Next(max)`: 0..max, 0 for max <= 0.
    pub fn next_int(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        (self.next_u64() % max as u64) as i32
    }

    /// `Rand.Next(min, max)` for floats.
    pub fn next_f32(&mut self, min: f32, max: f32) -> f32 {
        let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        min + u * (max - min)
    }
}

/// `Rand.AdjustForFramerate(chance)`.
fn adjust_for_framerate(chance: i32, lock_fps: i32) -> i32 {
    (chance as f32 * (lock_fps as f32 / 30.0)) as i32
}

/// One chunk level's rain splashes (`IsoChunkLevel.rainSplashFrame`,
/// `rainFlags`) and its squares' splash positions (`splashX/Y`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChunkLevelRain {
    pub frame: [f32; 64],
    pub flags: [u8; 64],
    pub raining: bool,
    pub frame_num: i32,
    pub splash_xy: [(f32, f32); 64],
}

impl Default for ChunkLevelRain {
    fn default() -> Self {
        ChunkLevelRain { frame: [-1.0; 64], flags: [0; 64], raining: false, frame_num: -1, splash_xy: [(0.0, 0.0); 64] }
    }
}

/// `CLDSF_RAIN_RANDOM_XY`.
const RAIN_RANDOM_XY: u8 = 2;

/// The rain values the splashes read: `IsoCell.getRainIntensity()` (the
/// debug override, normally 0), `RainManager.isRaining()` and
/// `RainManager.rainIntensity` (the precipitation intensity while raining).
fn rain_values(s: &Surfaces<'_>) -> (i32, bool, f32) {
    let c = &s.climate;
    let rm = if c.raining { c.precipitation_intensity } else { 0.0 };
    (c.cell_rain_intensity, c.raining, rm)
}

/// Every chunk level's rain splashes.
#[derive(Clone, Debug, Default)]
pub struct RainSplashes {
    pub levels: HashMap<(i32, i32, i32), ChunkLevelRain>,
    pub rng: SplashRng,
}

impl RainSplashes {
    /// `IsoChunkLevel.clear()` for a chunk leaving the map.
    pub fn remove_chunk(&mut self, wx: i32, wy: i32) {
        self.levels.retain(|(x, y, _), _| (*x, *y) != (wx, wy));
    }

    /// `updateRainSplashes` (once a frame per chunk level).
    pub fn update(&mut self, s: &Surfaces<'_>, wx: i32, wy: i32, level: i32) {
        let (cell, raining, rm) = rain_values(s);
        let lock_fps = s.settings.lock_fps;
        let frame = s.frame;
        let rng = &mut self.rng;
        let l = self.levels.entry((wx, wy, level)).or_default();
        if l.frame_num == frame.frame_count {
            return;
        }
        l.frame_num = frame.frame_count;
        if cell > 0 || (raining && rm > 0.0) {
            l.raining = true;
            if frame.paused {
                return;
            }
            let mut intensity = cell;
            if intensity == 0 {
                intensity = (fastfloor(rm / 0.2) + 1).min(5);
            }
            for i in 0..64 {
                if l.frame[i] < 0.0 {
                    let n = adjust_for_framerate((5.0 / intensity as f32) as i32 * 100, lock_fps);
                    if rng.next_int(n) == 0 {
                        l.frame[i] = 0.0;
                        l.flags[i] |= RAIN_RANDOM_XY;
                    }
                    continue;
                }
                l.frame[i] += 0.08 * (30.0 / lock_fps as f32);
                if l.frame[i] >= 1.0 {
                    l.frame[i] = -1.0;
                }
            }
            return;
        }
        if l.raining {
            l.raining = false;
            l.frame = [-1.0; 64];
        }
    }
}

/// `renderRainSplashes(playerIndex)` for chunk `(wx, wy)` at `level`, after
/// updating it.
pub fn render_rain_splashes(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, wx: i32, wy: i32, level: i32, out: &mut Vec<DrawCmd>) {
    let Some(rain) = s.rain else { return };
    let mut rain = rain.borrow_mut();
    rain.update(s, wx, wy, level);
    let RainSplashes { levels, rng } = &mut *rain;
    let Some(l) = levels.get_mut(&(wx, wy, level)) else { return };
    if !l.raining {
        return;
    }
    let cv = r.cutaways.as_ref();
    for i in 0..64 {
        if l.frame[i] < 0.0 {
            continue;
        }
        let Some(sq) = s.world.square(wx * 8 + (i % 8) as i32, wy * 8 + (i / 8) as i32, level) else { continue };
        if cv.is_some_and(|cv| !cv.is_visible(sq)) {
            continue;
        }
        render_rain_splash(r, s, sq, l.frame[i], l.flags[i] & RAIN_RANDOM_XY != 0, &mut l.splash_xy[i], rng, out);
        l.flags[i] &= !RAIN_RANDOM_XY;
    }
}

/// `IsoGridSquare.renderRainSplash(playerIndex, lightInfo, splashFrame,
/// bRandomXY)` in the chunk-texture renderer.
#[allow(clippy::too_many_arguments)]
fn render_rain_splash(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, sq: &RenderSquare, frame: f32, random_xy: bool, xy: &mut (f32, f32), rng: &mut SplashRng, out: &mut Vec<DrawCmd>) {
    let f = (frame * 4.0) as i32;
    if frame < 0.0 || f >= 50 {
        return;
    }
    if !sq.cut.could_see {
        return;
    }
    if !sq.has(|f| f.exterior) || sq.has(|f| f.vegitation) || !sq.has(|f| f.solidfloor) {
        return;
    }
    let Some(tex) = s.textures.texture(&format!("RainSplash_00_{f}")) else { return };
    if random_xy {
        *xy = (rng.next_f32(0.1, 0.9), rng.next_f32(0.1, 0.9));
    }
    let cam = &r.cam;
    let (x, y, z) = (sq.x as f32 + xy.0, sq.y as f32 + xy.1, sq.z as f32);
    let mut sx = x_to_screen(x, y) - cam.off_x as f32;
    let mut sy = y_to_screen(x, y, z, 0) - cam.off_y as f32;
    sx -= ((tex.width as i32 / 2) * TILE_SCALE) as f32;
    sy -= ((tex.height as i32 / 2) * TILE_SCALE) as f32;
    let depth = square_depth(fastfloor(cam.x), fastfloor(cam.y), x + 0.1, y + 0.1, z) * 2.0 - 1.0;
    let (cell, _, rm) = rain_values(s);
    let alpha = 0.6 * if cell > 0 { 1.0 } else { rm };
    let l = sq.light_info;
    out.extend([
        DrawCmd::GlEnable(gl::DEPTH_TEST),
        DrawCmd::DepthFunc(gl::LEQUAL),
        DrawCmd::DepthMask(false),
        DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA),
    ]);
    let (w, h) = ((tex.width as i32 * TILE_SCALE) as f32, (tex.height as i32 * TILE_SCALE) as f32);
    if let Some(d) = TexDraw::new(&tex, sx, sy, w, h, Rgba(0.8 * l.r, 0.9 * l.g, l.b, alpha)) {
        out.push(d.into_cmd(WorldShader::Default { z: depth }));
    }
}

/// `IsoGridSquare.WaterSplashData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterSplashData {
    pub dx: f32,
    pub dy: f32,
    pub frame: f32,
    pub size: f32,
    pub big: bool,
    frame_count: i32,
    frame_cache_shift: i32,
    unpaused_accumulator: f32,
}

impl Default for WaterSplashData {
    fn default() -> Self {
        WaterSplashData { dx: 0.0, dy: 0.0, frame: -1.0, size: 0.0, big: false, frame_count: 0, frame_cache_shift: 0, unpaused_accumulator: 0.0 }
    }
}

impl WaterSplashData {
    /// `waterSplashCache`'s texture name for cache index `i`.
    pub fn texture_name(i: i32) -> String {
        match i {
            0..=15 => format!("media/textures/waterSplashes/WaterSplashSmall0_{i}.png"),
            16..=47 => format!("media/textures/waterSplashes/WaterSplashBig0_{i}.png"),
            _ => format!("media/textures/waterSplashes/WaterSplashBig1_{i}.png"),
        }
    }

    /// `getTexture()`'s cache index.
    pub fn texture_index(&self) -> i32 {
        (self.frame * (self.frame_count - 1) as f32) as i32 + self.frame_cache_shift
    }

    #[allow(clippy::too_many_arguments)]
    fn init(&mut self, frame_count: i32, shift: i32, random_size: bool, dx: f32, dy: f32, frame: &FrameState, rng: &mut SplashRng) {
        self.frame = 0.0;
        self.frame_count = frame_count;
        self.frame_cache_shift = shift;
        self.unpaused_accumulator = frame.unpaused_accumulator;
        self.dx = dx;
        self.dy = dy;
        self.size = 0.5;
        if random_size {
            self.size = rng.next_f32(0.25, 0.75);
        }
    }

    /// `isSplashNow()`.
    pub fn is_splash_now(&mut self, f: &FrameState) -> bool {
        if !f.paused && (0.0..=1.0).contains(&self.frame) {
            if f.unpaused_accumulator < self.unpaused_accumulator {
                self.unpaused_accumulator = 0.0;
            }
            if self.frame + 0.0166 * (f.unpaused_accumulator - self.unpaused_accumulator) > 1.5 {
                self.frame = -1.0;
            }
        }
        (0.0..=1.0).contains(&self.frame)
    }

    /// `update()`.
    pub fn update(&mut self, f: &FrameState) {
        if f.unpaused_accumulator < self.unpaused_accumulator {
            self.unpaused_accumulator = 0.0;
        }
        if !f.paused && f.unpaused_accumulator > self.unpaused_accumulator {
            self.frame += 0.0166 * (f.unpaused_accumulator - self.unpaused_accumulator);
            if self.frame > 1.0 {
                self.frame = -1.0;
                self.unpaused_accumulator = 0.0;
            } else {
                self.unpaused_accumulator = f.unpaused_accumulator;
            }
        }
    }
}

/// The squares' fish splashes (`IsoGridSquare.waterSplashData`).
#[derive(Clone, Debug, Default)]
pub struct FishSplashes {
    pub squares: HashMap<(i32, i32, i32), WaterSplashData>,
    pub rng: SplashRng,
}

impl FishSplashes {
    /// `IsoGridSquare.startWaterSplash(isBigSplash, dx, dy)`: `seen` is
    /// `isSeenByAnyLocalPlayer()`. `None` offsets pick the original's
    /// random ones (`Rand.Next(0, 0.5) - 0.25`).
    #[allow(clippy::too_many_arguments)]
    pub fn start(&mut self, x: i32, y: i32, z: i32, big: bool, offset: Option<(f32, f32)>, seen: bool, frame: &FrameState) {
        let (dx, dy) = offset.unwrap_or_else(|| (self.rng.next_f32(0.0, 0.5) - 0.25, self.rng.next_f32(0.0, 0.5) - 0.25));
        let d = self.squares.entry((x, y, z)).or_default();
        if !seen || d.is_splash_now(frame) {
            return;
        }
        if big {
            let i = self.rng.next_int(2);
            d.init(32, 16 + 32 * i, false, dx, dy, frame, &mut self.rng);
            d.big = true;
        } else {
            d.init(16, 0, true, dx, dy, frame, &mut self.rng);
            d.big = false;
        }
    }
}

/// `shouldRenderFishSplash`: a lone object without attached sprites,
/// visible, splashing now.
pub fn should_render_fish_splash(s: &Surfaces<'_>, sq: &RenderSquare) -> bool {
    let Some(fish) = s.fish else { return false };
    if sq.objects.len() != 1 || !sq.objects[0].attached.is_empty() || !sq.cut.could_see {
        return false;
    }
    fish.borrow_mut().squares.get_mut(&(sq.x, sq.y, sq.z)).is_some_and(|d| d.is_splash_now(&s.frame))
}

/// `FBORenderCell.renderFishSplashes`: each square's `renderFishSplash`.
pub fn render_fish_splashes(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, squares: &[&RenderSquare], out: &mut Vec<DrawCmd>) {
    let Some(fish) = s.fish else { return };
    if squares.is_empty() {
        return;
    }
    out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    let mut fish = fish.borrow_mut();
    let cam = &r.cam;
    for sq in squares {
        let Some(d) = fish.squares.get_mut(&(sq.x, sq.y, sq.z)) else { continue };
        if !sq.cut.could_see || !d.is_splash_now(&s.frame) {
            continue;
        }
        let Some(tex) = s.textures.texture(&WaterSplashData::texture_name(d.texture_index())) else { continue };
        let (w, h) = (tex.width as f32 * d.size, tex.height as f32 * d.size);
        let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
        let sx = x_to_screen(x + 0.5, y + 0.5) - cam.off_x as f32 - w / 2.0 + cam.fix_jiggly_x * cam.zoom;
        let sy = y_to_screen(x + 0.5, y + 0.5, z, 0) - cam.off_y as f32 - h / 2.0 + cam.fix_jiggly_y * cam.zoom;
        let depth = (square_depth(fastfloor(cam.x), fastfloor(cam.y), x + 0.99, y + 0.99, z) + 0.001) * 2.0 - 1.0;
        out.extend([DrawCmd::GlEnable(gl::DEPTH_TEST), DrawCmd::DepthFunc(gl::LEQUAL), DrawCmd::DepthMask(false)]);
        let l = sq.light_info;
        if let Some(t) = TexDraw::new(&tex, sx, sy, w, h, Rgba(0.8 * l.r, 0.9 * l.g, l.b, 1.0)) {
            out.push(t.into_cmd(WorldShader::Default { z: depth }));
        }
        d.update(&s.frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fish_splashes_play_through_their_frames() {
        let mut f = FishSplashes::default();
        let mut fs = FrameState { frame_count: 0, paused: false, unpaused_accumulator: 100.0 };
        f.start(1, 2, 0, false, Some((0.1, 0.1)), true, &fs);
        let d = f.squares.get_mut(&(1, 2, 0)).unwrap();
        assert!(d.is_splash_now(&fs));
        assert!((0.25..0.75).contains(&d.size));
        assert_eq!(d.texture_index(), 0);
        // 30 frames later: 0.498 through the 16 small frames.
        fs.unpaused_accumulator += 30.0;
        d.update(&fs);
        assert_eq!(d.texture_index(), 7);
        fs.unpaused_accumulator += 40.0;
        d.update(&fs);
        assert!(!d.is_splash_now(&fs));
        assert_eq!(WaterSplashData::texture_name(20), "media/textures/waterSplashes/WaterSplashBig0_20.png");
    }

    #[test]
    fn rain_starts_splashes_and_clears_them_when_it_stops() {
        struct W;
        impl crate::weather::SurfaceWorld for W {
            fn square(&self, _: i32, _: i32, _: i32) -> Option<&RenderSquare> {
                None
            }
        }
        impl crate::weather::SurfaceTextures for W {
            fn texture(&self, _: &str) -> Option<pz_render::Texture> {
                None
            }
        }
        let mut s = Surfaces {
            settings: Default::default(),
            climate: crate::weather::ClimateInputs { raining: true, precipitation_intensity: 1.0, ..Default::default() },
            water: Default::default(),
            puddles: Default::default(),
            sky: Default::default(),
            world_age_hours: 0.0,
            meta_grid_min_x: 0,
            world: &W,
            textures: &W,
            frame: FrameState::default(),
            rain: None,
            fish: None,
        };
        let mut rain = RainSplashes::default();
        for f in 0..400 {
            s.frame.frame_count = f;
            rain.update(&s, 0, 0, 0);
            // Once a frame only.
            rain.update(&s, 0, 0, 0);
        }
        let l = rain.levels[&(0, 0, 0)];
        assert!(l.raining && l.frame.iter().any(|f| *f >= 0.0));
        s.climate.raining = false;
        s.frame.frame_count = 400;
        rain.update(&s, 0, 0, 0);
        assert!(rain.levels[&(0, 0, 0)].frame.iter().all(|f| *f < 0.0));
    }

    #[test]
    fn adjusts_chances_for_the_frame_rate() {
        assert_eq!(adjust_for_framerate(100, 60), 200);
        let mut r = SplashRng::default();
        assert!((0..1000).map(|_| r.next_f32(0.1, 0.9)).all(|v| (0.1..0.9).contains(&v)));
    }
}
