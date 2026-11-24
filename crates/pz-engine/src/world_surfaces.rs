//! Engine surface helpers: rain/fish splash state (`RainSplashes`,
//! `FishSplashes`, `FrameState`), splash texture names, and full
//! `ClimateInputs` snapshotting for [`pz_iso_render::weather::Surfaces`].

use std::cell::RefCell;

use pz_iso_render::splashes::{FishSplashes, FrameState, RainSplashes, WaterSplashData};
use pz_iso_render::weather::ClimateInputs;

/// Per-world rain and fish splash state owned by [`crate::world_render::WorldRender`].
#[derive(Default)]
pub struct WorldSplashes {
    pub rain: RefCell<RainSplashes>,
    pub fish: RefCell<FishSplashes>,
    pub frame: FrameState,
    last_tick: Option<i64>,
}

impl WorldSplashes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates `IsoCamera.frameState` (`frameCount`, `paused`,
    /// `unPausedAccumulator`) once per engine tick so split-screen players
    /// rendered on the same tick share one frame counter and accumulator.
    pub fn advance_frame(&mut self, tick: i64, paused: bool, realworld_seconds: f32) {
        self.frame.paused = paused;
        if self.last_tick == Some(tick) {
            return;
        }
        self.last_tick = Some(tick);
        self.frame.frame_count = self.frame.frame_count.wrapping_add(1);
        if !paused && realworld_seconds.is_finite() && realworld_seconds > 0.0 {
            self.frame.unpaused_accumulator += realworld_seconds * 60.0;
        }
    }

    /// Drops splash state for chunk `(wx, wy)` when it leaves the cell.
    pub fn on_chunk_removed(&mut self, wx: i32, wy: i32) {
        self.rain.borrow_mut().remove_chunk(wx, wy);
        self.fish
            .borrow_mut()
            .squares
            .retain(|&(x, y, _), _| (x.div_euclid(8), y.div_euclid(8)) != (wx, wy));
    }

    /// Starts a fish splash at square `(x, y, z)` (`IsoGridSquare.startWaterSplash`).
    pub fn start_fish_splash(
        &self,
        x: i32,
        y: i32,
        z: i32,
        big: bool,
        offset: Option<(f32, f32)>,
        seen: bool,
    ) {
        self.fish
            .borrow_mut()
            .start(x, y, z, big, offset, seen, &self.frame);
    }
}

/// Snapshots the climate values [`pz_iso_render::weather::Surfaces`] reads
/// from the shared [`pz_core::GameTime`]/`ClimateManager`.
pub fn surface_climate_inputs(snow_target: i32, winter: bool) -> ClimateInputs {
    let gt = pz_lua_api::bind_climate::shared_game_time();
    let Ok(g) = gt.try_borrow() else {
        return ClimateInputs {
            snow_target,
            winter,
            ..Default::default()
        };
    };
    let cm = &g.climate;
    ClimateInputs {
        rain_intensity: cm.get_rain_intensity(),
        wind_intensity: cm.get_wind_intensity(),
        corrected_wind_angle_intensity: cm.get_corrected_wind_angle_intensity(),
        temperature: cm.get_temperature(),
        snow_target,
        winter,
        raining: cm.is_raining(),
        precipitation_intensity: cm.get_precipitation_intensity(),
        cell_rain_intensity: 0,
    }
}

/// Texture names preloaded into `GroundTextures` for rain and fish splashes.
pub fn splash_texture_names() -> impl Iterator<Item = String> {
    (0..50)
        .map(|i| format!("RainSplash_00_{i}"))
        .chain((0..80).map(WaterSplashData::texture_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_iso_render::splashes::ChunkLevelRain;

    #[test]
    fn world_splashes_advances_once_per_tick_pauses_and_cleans_removed_chunks() {
        let mut s = WorldSplashes::new();
        s.advance_frame(1, false, 1.0 / 60.0);
        assert_eq!(s.frame.frame_count, 1);
        assert!((s.frame.unpaused_accumulator - 1.0).abs() < 1e-5);
        assert!(!s.frame.paused);

        // Second player on the same tick does not double-advance frame_count or accumulator.
        s.advance_frame(1, false, 1.0 / 60.0);
        assert_eq!(s.frame.frame_count, 1);
        assert!((s.frame.unpaused_accumulator - 1.0).abs() < 1e-5);

        // Paused tick increments frame_count but holds unpaused_accumulator steady.
        s.advance_frame(2, true, 1.0 / 60.0);
        assert_eq!(s.frame.frame_count, 2);
        assert!(s.frame.paused);
        assert!((s.frame.unpaused_accumulator - 1.0).abs() < 1e-5);

        // Unpaused tick resumes accumulation and drives fish splashes cleanly.
        s.advance_frame(3, false, 0.5);
        assert_eq!(s.frame.frame_count, 3);
        assert!((s.frame.unpaused_accumulator - 31.0).abs() < 1e-4);

        s.rain
            .borrow_mut()
            .levels
            .insert((2, 3, 0), ChunkLevelRain::default());
        s.rain
            .borrow_mut()
            .levels
            .insert((4, 5, 0), ChunkLevelRain::default());
        s.start_fish_splash(16, 24, 0, false, Some((0.0, 0.0)), true);
        s.start_fish_splash(32, 40, 0, true, Some((0.0, 0.0)), true);
        assert!(s.fish.borrow().squares.contains_key(&(16, 24, 0)));
        assert!(s.fish.borrow().squares.contains_key(&(32, 40, 0)));

        s.on_chunk_removed(2, 3);
        assert!(!s.rain.borrow().levels.contains_key(&(2, 3, 0)));
        assert!(s.rain.borrow().levels.contains_key(&(4, 5, 0)));
        assert!(!s.fish.borrow().squares.contains_key(&(16, 24, 0)));
        assert!(s.fish.borrow().squares.contains_key(&(32, 40, 0)));
    }

    #[test]
    fn splash_texture_names_cover_rain_and_small_and_big_fish_splashes() {
        let names: Vec<String> = splash_texture_names().collect();
        assert_eq!(names.len(), 130);
        assert!(names.iter().any(|n| n == "RainSplash_00_0"));
        assert!(names.iter().any(|n| n == "RainSplash_00_49"));
        assert!(names
            .iter()
            .any(|n| n == "media/textures/waterSplashes/WaterSplashSmall0_0.png"));
        assert!(names
            .iter()
            .any(|n| n == "media/textures/waterSplashes/WaterSplashBig0_20.png"));
        assert!(names
            .iter()
            .any(|n| n == "media/textures/waterSplashes/WaterSplashBig1_48.png"));
    }

    #[test]
    fn surface_climate_inputs_reads_live_climate_and_preserves_snow_and_winter() {
        let inputs = surface_climate_inputs(42, true);
        assert_eq!(inputs.snow_target, 42);
        assert!(inputs.winter);
        assert_eq!(inputs.cell_rain_intensity, 0);
        assert!(inputs.rain_intensity.is_finite());
        assert!(inputs.precipitation_intensity.is_finite());
        assert!(inputs.wind_intensity.is_finite());
    }
}
