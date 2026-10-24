//! `RenderSettings` and the climate values lighting reads, per player and
//! frame. `ClimateManager` and `RenderSettings` are ported elsewhere and
//! plug in through [`RenderSettingsSource`]; [`NoonSettings`] is a fixed
//! stand-in.

use pz_lighting::jni::PlayerRenderSettings;

/// What `LightingJNI.update()` reads from `RenderSettings`, `GameTime` and
/// `ClimateManager` for one player and frame.
pub trait RenderSettingsSource {
    /// `RenderSettings.getPlayerSettings(playerIndex)`.
    fn player_settings(&mut self, player: i32) -> PlayerRenderSettings;
    /// `ClimateManager.getDayLightStrength()` (the vision cone).
    fn day_light_strength(&self) -> f32;
    /// `GameTime.getNight()` (street lights are off below 0.5).
    fn night(&self) -> f32;
    /// `GameTime.getViewDistMax()`.
    fn view_dist_max(&self) -> f32 {
        42.0
    }
}

/// APPROXIMATION, not a port: a clear midday. `RenderSettings` lerps each
/// channel from 1 towards the climate's global light colour by its
/// intensity (`rmod = lerp(1, colour.r, intensity)`), with full ambient and
/// no night. The colour and intensity are inputs (the climate's values live
/// in the game's `ClimateMain.lua`, read at runtime); the default is white
/// at intensity 0, i.e. untinted. Replace with the real `RenderSettings`
/// over `ClimateManager` once those are ported.
#[derive(Clone, Copy, Debug)]
pub struct NoonSettings {
    /// The global light colour.
    pub colour: [f32; 3],
    /// Its intensity, 0 to 1.
    pub intensity: f32,
    pub view_distance: f32,
}

impl Default for NoonSettings {
    fn default() -> Self {
        NoonSettings {
            colour: [1.0; 3],
            intensity: 0.0,
            view_distance: 30.0,
        }
    }
}

impl RenderSettingsSource for NoonSettings {
    fn player_settings(&mut self, _player: i32) -> PlayerRenderSettings {
        let lerp = |c: f32| 1.0 + (c - 1.0) * self.intensity;
        let [r, g, b] = self.colour;
        PlayerRenderSettings {
            rmod: lerp(r),
            gmod: lerp(g),
            bmod: lerp(b),
            ambient: 1.0,
            night: 0.0,
            view_distance: self.view_distance,
        }
    }

    fn day_light_strength(&self) -> f32 {
        1.0
    }

    fn night(&self) -> f32 {
        0.0
    }
}
