//! What water, puddles, snow and blood read beyond the squares: the
//! weather (`ClimateManager` through `IsoWater.update` and
//! `IsoPuddles.update`), the render options, the sky reflection textures
//! and the named textures they draw with.
//!
//! The weather comes in as [`ClimateInputs`], which the caller fills each
//! frame from the climate (`pz_core::weather`'s
//! `game_time.climate.render_values(...)` once that lands):
//!
//! | field | `ClimateManager` getter (`ClimateRenderValues` field of the same meaning) |
//! |---|---|
//! | `rain_intensity` | `getRainIntensity()` (`rain_intensity`) |
//! | `wind_intensity` | `getWindIntensity()` (`wind_intensity`) |
//! | `corrected_wind_angle_intensity` | `getCorrectedWindAngleIntensity()` (`wind_angle_intensity`, the corrected value) |
//! | `temperature` | `getTemperature()` (`temperature`) |
//! | `snow_target` | `IsoCell.getSnowTarget()`: `(int)(snowFracNow * 100)`, 0 when `SandboxOptions.enableSnowOnGround` is off (`snow_frac_now`) |
//! | `winter` | `getSeason().isSeason(5)` or `Core.isForceSnow()` (`season`) |
//! | `raining` | `isRaining()` (`RainManager.isRaining()`) |
//! | `precipitation_intensity` | `getPrecipitationIntensity()` (`precipitation_intensity`; `RainManager.rainIntensity` while raining) |
//! | `cell_rain_intensity` | `IsoCell.getRainIntensity()`: the debug override, normally 0 |
//!
//! and the `GameTime` values `multiplier` (`getMultiplier()`),
//! `thirty_fps_multiplier` (`getThirtyFPSMultiplier()`) and
//! `world_age_hours` (`getWorldAgeHours()`).
//!
//! [`WaterState`] and [`PuddlesState`] are `IsoWater`'s and `IsoPuddles`'
//! own state (shader time, accumulated wind, the puddle floats), advanced
//! once per game update by their `update` methods. Their uniforms map as
//! `WaterShader.updateWaterParams` / `PuddlesShader.updatePuddlesParams`
//! set them; the puddle floats are `IsoPuddles.PuddlesFloat`s
//! (`getWetGroundFinalValue`, `getPuddlesSizeFinalValue`,
//! `getMuddyPuddlesFinalValue`, `getRainIntensity`).

use pz_render::Texture;

use crate::blood::ChunkBlood;
use crate::world::RenderSquare;

/// The climate values the surfaces read (see the module docs for where
/// each comes from).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClimateInputs {
    pub rain_intensity: f32,
    pub wind_intensity: f32,
    pub corrected_wind_angle_intensity: f32,
    pub temperature: f32,
    /// `IsoCell.getSnowTarget()`, 0..=100.
    pub snow_target: i32,
    pub winter: bool,
    pub raining: bool,
    pub precipitation_intensity: f32,
    pub cell_rain_intensity: i32,
}

/// `IsoWater`'s per-update state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaterState {
    /// `waterTime` (`WTime`).
    pub time: f32,
    pub wind_angle: f32,
    /// `waterWindIntensity` (already times 5).
    pub wind_intensity: f32,
    pub rain_intensity: f32,
    /// `waterParamWindInt` (`WParamWind`).
    pub wind: [f32; 2],
}

impl WaterState {
    /// `IsoWater.update(ClimateManager)`; `multiplier` is
    /// `GameTime.getMultiplier()`.
    pub fn update(&mut self, c: &ClimateInputs, multiplier: f32) {
        self.wind_angle = c.corrected_wind_angle_intensity;
        self.wind_intensity = c.wind_intensity * 5.0;
        self.rain_intensity = c.rain_intensity;
        self.time += 0.0166 * multiplier;
        self.wind[0] += (self.wind_angle * 6.0).sin() * self.wind_intensity * 0.05 * (multiplier / 1.6);
        self.wind[1] += (self.wind_angle * 6.0).cos() * self.wind_intensity * 0.15 * (multiplier / 1.6);
    }

    /// `getWaterWindSpeed()` (`WParamWindSpeed`).
    pub fn wind_speed(&self) -> f32 {
        self.wind_intensity * 2.0
    }
}

/// `IsoPuddles.PuddlesFloat`: a value clamped to 0..1 with an admin
/// override.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PuddlesFloat {
    pub final_value: f32,
    pub admin_override: Option<f32>,
}

impl PuddlesFloat {
    pub fn get(&self) -> f32 {
        self.admin_override.unwrap_or(self.final_value)
    }

    pub fn set(&mut self, f: f32) {
        self.final_value = f.clamp(0.0, 1.0);
    }

    pub fn add(&mut self, f: f32) {
        self.final_value = (self.final_value + f).clamp(0.0, 1.0);
    }

    /// `addFinalValueForMax`.
    pub fn add_for_max(&mut self, f: f32, maximum: f32) {
        self.final_value = (self.final_value + f).min(maximum).max(0.0);
    }
}

/// `IsoPuddles`' per-update state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PuddlesState {
    pub wind_angle: f32,
    pub wind_intensity: f32,
    /// `puddlesTime` (`WTime`).
    pub time: f32,
    /// `puddlesParamWindInt`.
    pub wind: [f32; 2],
    pub rain: PuddlesFloat,
    pub wet_ground: PuddlesFloat,
    pub muddy_puddles: PuddlesFloat,
    pub puddles_size: PuddlesFloat,
    pub rain_intensity: PuddlesFloat,
}

impl PuddlesState {
    /// `IsoPuddles.update(ClimateManager)` without the network
    /// interpolation.
    pub fn update(&mut self, c: &ClimateInputs, multiplier: f32, thirty_fps_multiplier: f32) {
        self.wind_angle = c.corrected_wind_angle_intensity;
        self.wind_intensity = c.wind_intensity;
        self.rain.set(c.rain_intensity);
        let t = thirty_fps_multiplier;
        let drying_ground = 2.0e-5 * t * c.temperature;
        let drying_puddles = 2.0e-5 * t;
        let particle_deposition = 2.0e-4 * t;
        let mut rain = self.rain.get();
        rain = rain * rain * 0.05 * t;
        self.rain_intensity.set(self.rain.get() * 2.0);
        self.wet_ground.add(rain);
        self.muddy_puddles.add(rain * 2.0);
        self.puddles_size.add_for_max(rain * 0.01, 0.7);
        if rain == 0.0 {
            self.wet_ground.add(-drying_ground);
            self.muddy_puddles.add(-particle_deposition);
        }
        if self.wet_ground.get() == 0.0 {
            self.puddles_size.add(-drying_puddles);
        }
        self.time += 0.0166 * multiplier;
        self.wind[0] += (self.wind_angle * 6.0).sin() * self.wind_intensity * 0.05;
        self.wind[1] += (self.wind_angle * 6.0).cos() * self.wind_intensity * 0.05;
    }

    /// `getPuddlesParams(z)` as the shader reads it (`PuddlesParams[0]` and
    /// `[1]`, uploaded transposed).
    pub fn params(&self, z: i32) -> [[f32; 4]; 2] {
        [[self.wind[0], self.wind[1], self.wind_intensity, z as f32], [self.muddy_puddles.get(), self.wet_ground.get(), self.puddles_size.get(), self.rain_intensity.get()]]
    }
}

/// The render options the surfaces depend on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSettings {
    /// `PerformanceSettings.waterQuality`: 0 `water_hq`, 1 `water`, 2 no
    /// water shader.
    pub water_quality: i32,
    /// `PerformanceSettings.puddlesQuality`: 0 `puddles_hq`, 1
    /// `puddles_mq`, 2 `puddles_lq` drawn into the chunk textures.
    pub puddles_quality: i32,
    /// `Core.getPerfPuddles()`: 0 all (and puddles leaking into rooms), 1
    /// ground level only, 2 also no direction flags, 3 off.
    pub perf_puddles: i32,
    /// `Core.getOptionBloodDecals()`, 0..=10.
    pub blood_decals: i32,
    /// `PerformanceSettings.getLockFPS()` (blood fade-out).
    pub lock_fps: i32,
    /// `Core.getUseShaders()`.
    pub use_shaders: bool,
}

impl Default for SurfaceSettings {
    fn default() -> Self {
        SurfaceSettings { water_quality: 0, puddles_quality: 0, perf_puddles: 0, blood_decals: 10, lock_fps: 60, use_shaders: true }
    }
}

impl SurfaceSettings {
    /// `IsoWater.getShaderEnable()`.
    pub fn water_shader(&self) -> bool {
        self.water_quality != 2
    }

    /// `IsoPuddles.getShaderEnable()`.
    pub fn puddles_shader(&self) -> bool {
        self.perf_puddles != 3
    }
}

/// The world beyond the chunk being drawn: neighbour squares (snow, blood)
/// and per-chunk blood splats.
pub trait SurfaceWorld {
    /// `getGridSquare(x, y, z)`.
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare>;
    /// The loaded chunk `(wx, wy)`'s floor blood splats; `None` when the
    /// chunk is not loaded.
    fn chunk_blood(&self, _wx: i32, _wy: i32) -> Option<&ChunkBlood> {
        None
    }
}

/// Textures by name: `Texture.getSharedTexture(name)` for tiles
/// (`e_newsnow_ground_1_*`, `blood_floor_*`) and `media/textures` images
/// (`river_bottom.png`, `puddles_hm.png`).
pub trait SurfaceTextures {
    fn texture(&self, name: &str) -> Option<Texture>;
}

/// `SkyBox`'s reflection textures and their blend
/// ([`crate::skybox::SkyBox::reflection`]).
#[derive(Clone, Debug, Default)]
pub struct SkyReflection {
    /// `getTextureCurrent()` and `getTexturePrev()`; white when missing.
    pub current: Option<Texture>,
    pub prev: Option<Texture>,
    /// `getTextureShift()`.
    pub shift: f32,
}

/// Everything the surfaces need for a frame.
pub struct Surfaces<'a> {
    pub settings: SurfaceSettings,
    pub climate: ClimateInputs,
    pub water: WaterState,
    pub puddles: PuddlesState,
    pub sky: SkyReflection,
    /// `GameTime.getWorldAgeHours()` (blood ageing).
    pub world_age_hours: f32,
    /// `IsoWorld.metaGrid.getMinX()` (snow tile variation).
    pub meta_grid_min_x: i32,
    pub world: &'a dyn SurfaceWorld,
    pub textures: &'a dyn SurfaceTextures,
    /// `IsoCamera.frameState`'s frame counter and pause state.
    pub frame: crate::splashes::FrameState,
    /// The rain and fish splash state; `None` draws none.
    pub rain: Option<&'a std::cell::RefCell<crate::splashes::RainSplashes>>,
    pub fish: Option<&'a std::cell::RefCell<crate::splashes::FishSplashes>>,
}

impl Surfaces<'_> {
    /// `IsoPuddles.shouldRenderPuddles()`.
    pub fn should_render_puddles(&self) -> bool {
        self.settings.puddles_shader() && self.settings.use_shaders && (self.puddles.wet_ground.get() != 0.0 || self.puddles.puddles_size.get() != 0.0)
    }

    /// `FBORenderSnow.isSnowAnywhere()`.
    pub fn snow_anywhere(&self) -> bool {
        self.climate.snow_target > 0
    }
}

/// `FBORenderCell`'s checks that redraw every chunk texture when a surface
/// option or value changes (`checkWaterQualityOption`,
/// `checkPuddlesQualityOption`, the low-quality puddle and snow checks in
/// `renderInternal`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceChecks {
    water_shader: Option<bool>,
    puddles_lq: Option<bool>,
    puddles_value: f32,
    wet_ground_value: f32,
    puddles_redraw_ms: i64,
    snow_target: i32,
}

/// What [`SurfaceChecks::run`] asks the caller to invalidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceInvalidation {
    /// `invalidateAll`: every chunk texture.
    pub all: bool,
    /// `invalidateLevel(0, REDRAW)` for the on-screen chunks' level 0.
    pub level_zero: bool,
    /// Puddles must be re-initialised (`IsoPuddlesGeometry.init`).
    pub reinit_puddles: bool,
}

impl SurfaceChecks {
    /// One frame's checks at `now_ms` (`System.currentTimeMillis()`).
    /// `enable_snow_on_ground` is `SandboxOptions.enableSnowOnGround`.
    pub fn run(&mut self, s: &Surfaces<'_>, enable_snow_on_ground: bool, now_ms: i64) -> SurfaceInvalidation {
        let mut inv = SurfaceInvalidation::default();
        let ws = s.settings.water_shader();
        if self.water_shader.is_some_and(|w| w != ws) {
            inv.level_zero = true;
        }
        self.water_shader = Some(ws);
        let lq = s.settings.puddles_quality == 2;
        if self.puddles_lq != Some(lq) {
            if self.puddles_lq.is_some() {
                inv.reinit_puddles = true;
                inv.all = true;
            }
            self.puddles_lq = Some(lq);
        }
        let pv = (s.puddles.puddles_size.get() * 500.0).ceil();
        let wg = (s.puddles.wet_ground.get() * 500.0).ceil();
        if lq && (self.puddles_value != pv || self.wet_ground_value != wg) && self.puddles_redraw_ms + 1000 < now_ms {
            self.puddles_value = pv;
            self.wet_ground_value = wg;
            self.puddles_redraw_ms = now_ms;
            inv.all = true;
        }
        if enable_snow_on_ground && self.snow_target != s.climate.snow_target {
            self.snow_target = s.climate.snow_target;
            inv.all = true;
        }
        inv
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_wets_the_ground_and_grows_puddles() {
        let mut p = PuddlesState::default();
        let c = ClimateInputs { rain_intensity: 1.0, temperature: 10.0, ..Default::default() };
        for _ in 0..100 {
            p.update(&c, 1.0, 1.0);
        }
        // 0.05 a tick: wet after 20 ticks; puddles grow 0.0005 a tick.
        assert_eq!(p.wet_ground.get(), 1.0);
        assert!((p.puddles_size.get() - 0.05).abs() < 1e-4);
        assert_eq!(p.rain_intensity.get(), 1.0);
        assert_eq!(p.params(0)[1][3], 1.0);
        let dry = ClimateInputs { temperature: 10.0, ..Default::default() };
        p.update(&dry, 1.0, 1.0);
        assert!((p.wet_ground.get() - (1.0 - 2.0e-4)).abs() < 1e-6);
    }

    #[test]
    fn checks_ask_for_redraws_when_surfaces_change() {
        struct W;
        impl SurfaceWorld for W {
            fn square(&self, _: i32, _: i32, _: i32) -> Option<&RenderSquare> {
                None
            }
        }
        impl SurfaceTextures for W {
            fn texture(&self, _: &str) -> Option<Texture> {
                None
            }
        }
        let mut s = Surfaces {
            settings: SurfaceSettings::default(),
            climate: ClimateInputs::default(),
            water: WaterState::default(),
            puddles: PuddlesState::default(),
            sky: SkyReflection::default(),
            world_age_hours: 0.0,
            meta_grid_min_x: 0,
            world: &W,
            textures: &W,
            frame: Default::default(),
            rain: None,
            fish: None,
        };
        let mut c = SurfaceChecks::default();
        assert_eq!(c.run(&s, true, 0), SurfaceInvalidation::default());
        s.settings.water_quality = 2;
        assert!(c.run(&s, true, 10).level_zero);
        s.climate.snow_target = 40;
        assert!(c.run(&s, true, 20).all);
        assert!(!c.run(&s, true, 30).all);
        s.settings.puddles_quality = 2;
        let inv = c.run(&s, true, 40);
        assert!(inv.all && inv.reinit_puddles);
        // Low-quality puddles redraw as they grow, at most once a second.
        s.puddles.puddles_size.set(0.5);
        assert!(!c.run(&s, true, 900).all);
        assert!(c.run(&s, true, 1100).all);
    }

    #[test]
    fn water_time_and_wind_advance() {
        let mut w = WaterState::default();
        let c = ClimateInputs { wind_intensity: 1.0, corrected_wind_angle_intensity: 0.0, ..Default::default() };
        w.update(&c, 1.6);
        assert!((w.time - 0.0166 * 1.6).abs() < 1e-7);
        assert_eq!(w.wind_speed(), 10.0);
        assert_eq!(w.wind[0], 0.0);
        assert!((w.wind[1] - 5.0 * 0.15).abs() < 1e-6);
    }
}
