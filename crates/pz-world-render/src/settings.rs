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

/// APPROXIMATION, not a port, for tests and tools without a climate: a clear midday. `RenderSettings` lerps each
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

/// What `PlayerRenderSettings.updateRenderSettings` reads from
/// `ClimateManager` (after lightning), the player and the sandbox. Fill it
/// from `pz_core::weather::ClimateRenderValues`: `cm_global_light`'s
/// exterior and interior RGBA, `cm_global_light_intensity`, `cm_ambient`,
/// `cm_day_light_strength`, `cm_night_strength`, `cm_desaturation`,
/// `cm_view_distance`, `cm_fog_intensity` and `moon_float`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateInput {
    pub global_light_exterior: [f32; 4],
    pub global_light_interior: [f32; 4],
    pub ambient: f32,
    pub day_light_strength: f32,
    pub night_strength: f32,
    pub desaturation: f32,
    pub view_distance: f32,
    pub fog_intensity: f32,
    pub moon_float: f32,
    /// `GameTime.getNight()`.
    pub game_night: f32,
    /// `SandboxOptions.nightDarkness` (1-4).
    pub night_darkness: i32,
    /// `SceneShaderStore.weatherShader != null && Core.getOffscreenBuffer()
    /// != null`.
    pub weather_shader: bool,
    /// `Core.lastStand`.
    pub last_stand: bool,
}

/// What `updateRenderSettings` reads about one player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerRenderInput {
    pub night_vision_goggles: bool,
    pub night_vision_trait: bool,
}

/// `RenderSettings.PlayerRenderSettings.updateRenderSettings` over the
/// climate's values.
#[derive(Clone, Copy, Debug)]
pub struct ClimateSettings {
    pub climate: ClimateInput,
    pub players: [PlayerRenderInput; 4],
}

/// `GameTime.Lerp`.
fn game_lerp(start: f32, end: f32, delta: f32) -> f32 {
    let d = delta.clamp(0.0, 1.0);
    start + (end - start) * d
}

impl ClimateSettings {
    /// The settings for `player`, with `isExterior` forced true as the
    /// original does.
    pub fn compute(&self, player: usize) -> PlayerRenderSettings {
        let c = &self.climate;
        let p = self.players.get(player).copied().unwrap_or_default();
        let mut desaturation = c.desaturation;
        let mut night = c.night_strength;
        let mut darkness = 1.0 - c.day_light_strength;
        let blend = c.global_light_exterior;
        let blend_intensity = blend[3];
        let mut ambient = c.ambient;
        let mut ambient_min = match c.night_darkness {
            1 => 0.0,
            2 => 0.07,
            3 => 0.15,
            4 => 0.25,
            _ => 0.15,
        };
        ambient_min += 0.075 * c.moon_float * night;
        if ambient < 0.2 && p.night_vision_trait {
            ambient = 0.2;
        }
        ambient = ambient_min + (1.0 - ambient_min) * ambient;
        if c.last_stand {
            ambient = 0.65;
            darkness = 0.25;
            night = 0.25;
        }
        let desaturated = |s: f32| {
            let gray = blend[0] * 0.3086 + blend[1] * 0.6094 + blend[2] * 0.082;
            [
                gray * s + blend[0] * (1.0 - s),
                gray * s + blend[1] * (1.0 - s),
                gray * s + blend[2] * (1.0 - s),
            ]
        };
        let (rmod, gmod, bmod);
        if c.weather_shader && p.night_vision_goggles {
            ambient = 1.0;
            let v = game_lerp(1.0, 0.7, darkness);
            (rmod, gmod, bmod) = (v, v, v);
        } else {
            desaturation *= 1.0 - darkness;
            let b = desaturated(desaturation);
            rmod = game_lerp(1.0, b[0], blend_intensity);
            gmod = game_lerp(1.0, b[1], blend_intensity);
            bmod = game_lerp(1.0, b[2], blend_intensity);
            if !c.weather_shader && p.night_vision_goggles {
                // Legacy path with goggles: no blend, no night.
                night = 0.0;
                ambient = 0.8;
                return PlayerRenderSettings {
                    rmod: 1.0,
                    gmod: 1.0,
                    bmod: 1.0,
                    ambient,
                    night,
                    view_distance: c.view_distance,
                };
            }
        }
        PlayerRenderSettings {
            rmod,
            gmod,
            bmod,
            ambient,
            night,
            view_distance: c.view_distance,
        }
    }
}

/// What `updateRenderSettings` leaves on `PlayerRenderSettings` for the
/// screen pass (`WeatherShader.startMainThread`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRenderSettings {
    /// `getBlendColor()` rgb: the climate's exterior global light.
    pub blend_color: [f32; 3],
    /// `getBlendIntensity()`: its alpha; 0 on the legacy path with goggles.
    pub blend_intensity: f32,
    /// `getDesaturation()`: the climate's, times `1 - darkness` unless the
    /// weather shader draws the goggles.
    pub desaturation: f32,
    /// `getDarkness()`: `1 - dayLightStrength` (0.25 in Last Stand).
    pub darkness: f32,
    /// `isApplyNightVisionGoggles()`.
    pub night_vision_goggles: bool,
    /// `isExterior()`: forced true by the original.
    pub exterior: bool,
}

impl ClimateSettings {
    /// The screen-pass half of `updateRenderSettings` for `player`.
    pub fn screen_settings(&self, player: usize) -> ScreenRenderSettings {
        let c = &self.climate;
        let p = self.players.get(player).copied().unwrap_or_default();
        let mut desaturation = c.desaturation;
        let mut darkness = 1.0 - c.day_light_strength;
        // isExterior = true: the exterior light, no interior adjustment.
        let blend = c.global_light_exterior;
        let mut blend_intensity = blend[3];
        if c.last_stand {
            darkness = 0.25;
        }
        if c.weather_shader {
            if !p.night_vision_goggles {
                desaturation *= 1.0 - darkness;
            }
        } else {
            desaturation *= 1.0 - darkness;
            if p.night_vision_goggles {
                blend_intensity = 0.0;
            }
        }
        ScreenRenderSettings {
            blend_color: [blend[0], blend[1], blend[2]],
            blend_intensity,
            desaturation,
            darkness,
            night_vision_goggles: p.night_vision_goggles,
            exterior: true,
        }
    }

    /// `params` with the fields `WeatherShader.startMainThread` takes from
    /// `RenderSettings` for `player` (`Light`, `LightIntensity`,
    /// `DesaturationVal`, `NightVisionGoggles`, `NightValue`, `Exterior`).
    pub fn screen_params(&self, player: usize, params: pz_render::ScreenParams) -> pz_render::ScreenParams {
        self.screen_settings(player).apply(params)
    }
}

impl ScreenRenderSettings {
    /// `params` with these values in its render-settings fields.
    pub fn apply(&self, params: pz_render::ScreenParams) -> pz_render::ScreenParams {
        pz_render::ScreenParams {
            light: self.blend_color,
            light_intensity: self.blend_intensity,
            desaturation: self.desaturation,
            night_vision_goggles: self.night_vision_goggles,
            night_value: self.darkness,
            exterior: self.exterior,
            ..params
        }
    }
}

impl RenderSettingsSource for ClimateSettings {
    fn player_settings(&mut self, player: i32) -> PlayerRenderSettings {
        self.compute(player.max(0) as usize)
    }

    fn day_light_strength(&self) -> f32 {
        self.climate.day_light_strength
    }

    fn night(&self) -> f32 {
        self.climate.game_night
    }
}

#[cfg(test)]
mod climate_tests {
    use super::*;

    fn input() -> ClimateInput {
        ClimateInput {
            global_light_exterior: [1.0, 0.8, 0.6, 0.5],
            global_light_interior: [1.0; 4],
            ambient: 0.5,
            day_light_strength: 1.0,
            night_strength: 0.0,
            desaturation: 0.0,
            view_distance: 30.0,
            fog_intensity: 0.0,
            moon_float: 0.0,
            game_night: 0.0,
            night_darkness: 3,
            weather_shader: true,
            last_stand: false,
        }
    }

    #[test]
    fn day_settings_follow_the_global_light() {
        let s = ClimateSettings {
            climate: input(),
            players: Default::default(),
        }
        .compute(0);
        assert_eq!(s.rmod, 1.0);
        assert!((s.gmod - 0.9).abs() < 1e-6 && (s.bmod - 0.8).abs() < 1e-6);
        // ambientMin 0.15 for night darkness 3: 0.15 + 0.85 * 0.5.
        assert!((s.ambient - 0.575).abs() < 1e-6);
    }

    fn screen(climate: ClimateInput, goggles: bool) -> ScreenRenderSettings {
        let mut players = [PlayerRenderInput::default(); 4];
        players[1].night_vision_goggles = goggles;
        ClimateSettings { climate, players }.screen_settings(1)
    }

    /// Dusk with the weather shader: desaturation 0.6 * (1 - 0.25).
    #[test]
    fn screen_settings_scale_desaturation_by_darkness() {
        let c = ClimateInput { desaturation: 0.6, day_light_strength: 0.75, ..input() };
        let s = screen(c, false);
        assert_eq!(s, ScreenRenderSettings { blend_color: [1.0, 0.8, 0.6], blend_intensity: 0.5, desaturation: 0.6 * 0.75, darkness: 0.25, night_vision_goggles: false, exterior: true });
    }

    /// Goggles under the weather shader keep the climate's desaturation;
    /// on the legacy path they drop the blend intensity instead.
    #[test]
    fn screen_settings_with_goggles() {
        let c = ClimateInput { desaturation: 0.6, day_light_strength: 0.75, ..input() };
        let s = screen(c, true);
        assert_eq!((s.desaturation, s.blend_intensity, s.night_vision_goggles), (0.6, 0.5, true));
        let s = screen(ClimateInput { weather_shader: false, ..c }, true);
        assert_eq!((s.desaturation, s.blend_intensity, s.darkness), (0.6 * 0.75, 0.0, 0.25));
    }

    /// Last Stand fixes the darkness at 0.25; the exterior light is used
    /// even indoors (`isExterior` is forced true).
    #[test]
    fn screen_settings_in_last_stand() {
        let c = ClimateInput { desaturation: 0.4, day_light_strength: 0.1, last_stand: true, global_light_interior: [0.1, 0.2, 0.3, 0.9], ..input() };
        let s = screen(c, false);
        assert_eq!((s.darkness, s.desaturation, s.blend_color, s.exterior), (0.25, 0.3, [1.0, 0.8, 0.6], true));
    }

    #[test]
    fn screen_params_take_the_render_settings_fields() {
        let c = ClimateInput { desaturation: 0.6, day_light_strength: 0.75, ..input() };
        let mut players = [PlayerRenderInput::default(); 4];
        players[1].night_vision_goggles = true;
        let base = pz_render::ScreenParams { timer: 5.0, zoom: 1.5, drunk_factor: 0.3, ..Default::default() };
        let p = ClimateSettings { climate: c, players }.screen_params(1, base);
        let want = pz_render::ScreenParams { light: [1.0, 0.8, 0.6], light_intensity: 0.5, desaturation: 0.6, night_vision_goggles: true, night_value: 0.25, exterior: true, ..base };
        assert_eq!(p, want);
    }
}
