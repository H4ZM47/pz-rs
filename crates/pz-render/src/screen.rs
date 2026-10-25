//! `WeatherShader`: what the `screen` shader (the offscreen world buffer
//! drawn to the screen, `MultiTextureFBO2.render`) is given per player.
//!
//! `startMainThread` copies the player's inputs into `TextureDraw.vars`
//! (and `f1`, `flipped`, `col0..3`, `singleCol`); `startRenderThread`
//! uploads them as uniforms with the frame timer. [`ScreenParams`] holds
//! those inputs, [`ScreenParams::uniforms`] is the upload and
//! [`ScreenTimer`] the timer.
//!
//! Who fills each value (the engine copies them in per frame):
//! - `RenderSettings.PlayerRenderSettings` (`updateRenderSettings`, from
//!   the climate): `light`, `light_intensity`, `desaturation`,
//!   `night_value` (`getDarkness`), `exterior`.
//! - The player: `night_vision_goggles`
//!   (`isWearingNightVisionGoggles`), `drunk_factor` (intoxication / 100)
//!   and `blur_factor` (`IsoPlayer.getBlurFactor`), through
//!   `PlayerRenderSettings`.
//! - `SearchMode` (characters/gameplay): [`SearchModeShader`].
//! - The camera and `Core`: offscreen rectangle and size, right-click
//!   offset, zoom, tile width.

/// What `SearchMode.PlayerSearchMode` gives the shader: each `getShader*`
/// value is the exterior or interior one by where the player stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SearchModeShader {
    /// `isShaderEnabled()`: enabled, or fading in or out (`VarInfo.x`).
    pub enabled: bool,
    /// `getShaderBlur()` (`SearchMode.x`).
    pub blur: f32,
    /// `getShaderRadius()` in tiles (`SearchMode.y`).
    pub radius: f32,
    /// `getShaderGradientWidth()` in tiles (`ParamInfo.z` is this times
    /// half the tile width).
    pub gradient_width: f32,
    /// `getShaderDesat()` (`ParamInfo.w`).
    pub desat: f32,
    /// `getShaderDarkness()` (`VarInfo.y`).
    pub darkness: f32,
}

/// One player's `WeatherShader` inputs. The default is all zero: no
/// search mode, goggles, drunkenness or blur, with the rendered size taken
/// from the viewport (see [`ScreenParams::offscreen_width`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenParams {
    /// `vars[0..3]`, `Light`: `PlayerRenderSettings.getBlendColor()`.
    /// The live shader does not read it.
    pub light: [f32; 3],
    /// `vars[3]`, `LightIntensity`: `getBlendIntensity()` (not read).
    pub light_intensity: f32,
    /// `vars[4]`, `DesaturationVal`: `getDesaturation()`.
    pub desaturation: f32,
    /// `vars[5]`, `NightVisionGoggles`: `isApplyNightVisionGoggles()`.
    pub night_vision_goggles: bool,
    /// `f1`, `NightValue`: `getDarkness()` (the night-vision strength).
    pub night_value: f32,
    /// `flipped`, `Exterior`: `isExterior()` (not read).
    pub exterior: bool,
    /// `vars[22]`, `DrunkFactor`: `getDrunkFactor()`.
    pub drunk_factor: f32,
    /// `vars[23]`, `BlurFactor`: `getBlurFactor()`.
    pub blur_factor: f32,
    /// `vars[6, 7, 16..=19]`.
    pub search: SearchModeShader,
    /// `vars[8, 9]`: `IsoCamera.getOffscreenLeft/Top(player)`
    /// (`SearchMode.zw`).
    pub offscreen_left: f32,
    pub offscreen_top: f32,
    /// `vars[10, 11]` and `col0, col1`: `Core.getOffscreenWidth/Height
    /// (player)` (`ScreenInfo.xy`, `bgl_RenderedTextureWidth/Height`).
    /// Zero stands for the viewport size (one player at zoom 1).
    pub offscreen_width: f32,
    pub offscreen_height: f32,
    /// `vars[12, 13]`: the camera's `rightClickX/Y` (`ScreenInfo.zw`);
    /// [`ScreenParams::thumbnail`] sets them for the save thumbnail.
    pub right_click: [f32; 2],
    /// `vars[14]`: `Core.getZoom(player)` (`ParamInfo.x`).
    pub zoom: f32,
    /// `vars[15]`: 64 at `Core.tileScale` 2, else 32 (`ParamInfo.y`).
    pub tile_width: f32,
    /// `timer` and `timerWrap` ([`ScreenTimer::next`]).
    pub timer: f32,
    pub timer_wrap: f32,
}

/// The uniforms `WeatherShader.startRenderThread` sets, by GLSL name.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenUniforms {
    pub bgl_rendered_texture_width: f32,
    pub bgl_rendered_texture_height: f32,
    pub night_value: f32,
    pub light: [f32; 3],
    pub light_intensity: f32,
    pub desaturation_val: f32,
    pub night_vision_goggles: f32,
    pub exterior: f32,
    pub timer: f32,
    pub timer_wrap: f32,
    pub texture_size: [f32; 2],
    pub zoom: f32,
    pub search_mode: [f32; 4],
    pub screen_info: [f32; 4],
    pub param_info: [f32; 4],
    pub var_info: [f32; 4],
    pub drunk_factor: f32,
    pub blur_factor: f32,
}

impl ScreenParams {
    /// `WeatherShader.startThumbnail`: the player's values with the
    /// right-click offset centring a `thumb_width` x `thumb_height` area
    /// (at `zoom`) in the offscreen buffer, and that zoom.
    pub fn thumbnail(mut self, thumb_width: i32, thumb_height: i32, zoom: f32) -> Self {
        self.right_click = [
            (self.offscreen_width - thumb_width as f32 * zoom) / 2.0,
            -((self.offscreen_height - thumb_height as f32 * zoom) / 2.0),
        ];
        self.zoom = zoom;
        self
    }

    /// `singleCol`, the `Zoom` uniform: zoom above 2, or in [1.75, 2).
    pub fn zoom_flag(&self) -> bool {
        self.zoom > 2.0 || (f64::from(self.zoom) < 2.0 && self.zoom >= 1.75)
    }

    /// `startRenderThread`'s uniforms, `texture_size` being the offscreen
    /// texture's (`Core.getOffscreenTrueWidth/Height`).
    pub fn uniforms(&self, texture_size: [f32; 2]) -> ScreenUniforms {
        let s = &self.search;
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        ScreenUniforms {
            bgl_rendered_texture_width: self.offscreen_width,
            bgl_rendered_texture_height: self.offscreen_height,
            night_value: self.night_value,
            light: self.light,
            light_intensity: self.light_intensity,
            desaturation_val: self.desaturation,
            night_vision_goggles: flag(self.night_vision_goggles),
            exterior: flag(self.exterior),
            timer: self.timer,
            timer_wrap: self.timer_wrap,
            texture_size,
            zoom: flag(self.zoom_flag()),
            search_mode: [s.blur, s.radius, self.offscreen_left, self.offscreen_top],
            screen_info: [
                self.offscreen_width,
                self.offscreen_height,
                self.right_click[0],
                self.right_click[1],
            ],
            param_info: [
                self.zoom,
                self.tile_width,
                s.gradient_width * self.tile_width / 2.0,
                s.desat,
            ],
            // vars[20] and [21] are never written: zero.
            var_info: [flag(s.enabled), s.darkness, 0.0, 0.0],
            drunk_factor: self.drunk_factor,
            blur_factor: self.blur_factor,
        }
    }
}

/// `WeatherShader`'s frame timer (`timerVal`, `alt`, `timerWrapVal`),
/// advanced by every `startRenderThread`: once per player screen pass and
/// per save thumbnail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenTimer {
    pub value: i32,
    pub alt: bool,
    pub wrap: f32,
}

impl Default for ScreenTimer {
    fn default() -> Self {
        ScreenTimer {
            value: 0,
            alt: false,
            wrap: -1.0,
        }
    }
}

impl ScreenTimer {
    /// The `timer` and `timerWrap` uniforms of one `startRenderThread`,
    /// then the advance: by one every other call when
    /// `PerformanceSettings.getLockFPS()` is 60 or more, else by two.
    pub fn next(&mut self, lock_fps: i32) -> (f32, f32) {
        let out = ((self.value / 2) as f32, self.wrap);
        if lock_fps >= 60 {
            if self.alt {
                self.value = self.value.wrapping_add(1);
            }
            self.alt = !self.alt;
        } else {
            self.value = self.value.wrapping_add(2);
        }
        self.wrap = 1.0 - 2.0 * (self.value as f32 / 2.147_483_6e9);
        out
    }

    /// [`ScreenParams`] with this call's `timer` and `timer_wrap`.
    pub fn apply(&mut self, params: ScreenParams, lock_fps: i32) -> ScreenParams {
        let (timer, timer_wrap) = self.next(lock_fps);
        ScreenParams {
            timer,
            timer_wrap,
            ..params
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_advances_every_other_call_at_60_fps() {
        let mut t = ScreenTimer::default();
        let got: Vec<_> = (0..6).map(|_| t.next(60)).collect();
        // timerVal 0,0,1,1,2,2 -> timer = val / 2; wrap lags one call.
        let w = |v: i32| 1.0 - 2.0 * (v as f32 / 2.147_483_6e9);
        assert_eq!(
            got,
            vec![
                (0.0, -1.0),
                (0.0, w(0)),
                (0.0, w(1)),
                (0.0, w(1)),
                (1.0, w(2)),
                (1.0, w(2))
            ]
        );
    }

    #[test]
    fn timer_advances_by_two_below_60_fps() {
        let mut t = ScreenTimer::default();
        let got: Vec<_> = (0..3).map(|_| t.next(30).0).collect();
        assert_eq!(got, vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn uniforms_pack_the_vars() {
        let p = ScreenParams {
            light: [0.1, 0.2, 0.3],
            light_intensity: 0.4,
            desaturation: 0.5,
            night_vision_goggles: true,
            night_value: 0.6,
            exterior: true,
            drunk_factor: 0.7,
            blur_factor: 0.8,
            search: SearchModeShader {
                enabled: true,
                blur: 1.0,
                radius: 4.0,
                gradient_width: 4.0,
                desat: 0.85,
                darkness: 0.3,
            },
            offscreen_left: 960.0,
            offscreen_top: 540.0,
            offscreen_width: 960.0,
            offscreen_height: 540.0,
            right_click: [12.0, -7.0],
            zoom: 1.5,
            tile_width: 64.0,
            timer: 3.0,
            timer_wrap: 0.5,
        };
        let u = p.uniforms([1024.0, 1024.0]);
        assert_eq!(u.search_mode, [1.0, 4.0, 960.0, 540.0]);
        assert_eq!(u.screen_info, [960.0, 540.0, 12.0, -7.0]);
        assert_eq!(u.param_info, [1.5, 64.0, 128.0, 0.85]);
        assert_eq!(u.var_info, [1.0, 0.3, 0.0, 0.0]);
        assert_eq!(
            (u.night_vision_goggles, u.exterior, u.zoom),
            (1.0, 1.0, 0.0)
        );
        assert_eq!(
            (u.bgl_rendered_texture_width, u.texture_size),
            (960.0, [1024.0, 1024.0])
        );
    }

    /// `WeatherShader.startMainThread` / `startThumbnail` then
    /// `startRenderThread`, run on the original classes with these inputs
    /// (a GL20 stand-in recording the uniforms): the same uniforms, and the
    /// timer carried across the calls.
    #[test]
    fn uniforms_match_the_original_shader() {
        let mut timer = ScreenTimer::default();
        // One player, 1280x720, tile scale 2, zoom 1, 60 fps.
        let single = ScreenParams {
            light: [0.9, 0.8, 0.7],
            light_intensity: 0.35,
            desaturation: 0.42,
            night_value: 0.3,
            exterior: true,
            offscreen_width: 1280.0,
            offscreen_height: 720.0,
            zoom: 1.0,
            tile_width: 64.0,
            ..Default::default()
        };
        let u = timer.apply(single, 60).uniforms([2048.0, 1024.0]);
        assert_eq!(
            (
                u.bgl_rendered_texture_width,
                u.bgl_rendered_texture_height,
                u.night_value
            ),
            (1280.0, 720.0, 0.3)
        );
        assert_eq!(
            (
                u.light,
                u.light_intensity,
                u.desaturation_val,
                u.night_vision_goggles,
                u.exterior
            ),
            ([0.9, 0.8, 0.7], 0.35, 0.42, 0.0, 1.0)
        );
        assert_eq!(
            (u.timer, u.timer_wrap, u.texture_size, u.zoom),
            (0.0, -1.0, [2048.0, 1024.0], 0.0)
        );
        assert_eq!(
            (u.search_mode, u.screen_info, u.param_info, u.var_info),
            (
                [0.0; 4],
                [1280.0, 720.0, 0.0, 0.0],
                [1.0, 64.0, 0.0, 0.0],
                [0.0; 4]
            )
        );
        // Search mode (interior values), goggles, drunk, blur, right-click
        // offset; 1920x1080 at tile scale 1, zoom 1.5 (Core.getZoom 0.75).
        let search = ScreenParams {
            light: [0.5, 0.6, 1.0],
            light_intensity: 0.8,
            desaturation: 0.1,
            night_vision_goggles: true,
            night_value: 0.65,
            drunk_factor: 0.37,
            blur_factor: 0.55,
            search: SearchModeShader {
                enabled: true,
                blur: 1.0,
                radius: 4.0,
                gradient_width: 3.5,
                desat: 0.85,
                darkness: 0.25,
            },
            offscreen_width: 1440.0,
            offscreen_height: 810.0,
            right_click: [33.0, -21.0],
            zoom: 0.75,
            tile_width: 32.0,
            ..Default::default()
        };
        let u = timer.apply(search, 60).uniforms([4096.0, 2048.0]);
        assert_eq!(
            (
                u.night_vision_goggles,
                u.exterior,
                u.drunk_factor,
                u.blur_factor
            ),
            (1.0, 0.0, 0.37, 0.55)
        );
        assert_eq!((u.timer, u.timer_wrap, u.zoom), (0.0, 1.0, 0.0));
        assert_eq!(u.search_mode, [1.0, 4.0, 0.0, 0.0]);
        assert_eq!(u.screen_info, [1440.0, 810.0, 33.0, -21.0]);
        assert_eq!(u.param_info, [0.75, 32.0, 56.0, 0.85]);
        assert_eq!(u.var_info, [1.0, 0.25, 0.0, 0.0]);
        // Player 3 of four on 2560x1440 (initial width 1920: display zoom
        // 0.5625), 30 fps.
        let split = ScreenParams {
            light: [1.0; 3],
            exterior: true,
            search: SearchModeShader {
                enabled: true,
                blur: 0.5,
                radius: 2.5,
                gradient_width: 1.0,
                desat: 0.4,
                darkness: 0.6,
            },
            offscreen_left: 1280.0,
            offscreen_top: 720.0,
            offscreen_width: 720.0,
            offscreen_height: 405.0,
            right_click: [-5.0, 7.0],
            zoom: 0.5625,
            tile_width: 64.0,
            ..Default::default()
        };
        let u = timer.apply(split, 30).uniforms([2048.0, 1024.0]);
        assert_eq!((u.timer, u.timer_wrap), (0.0, 1.0));
        assert_eq!(u.search_mode, [0.5, 2.5, 1280.0, 720.0]);
        assert_eq!(u.screen_info, [720.0, 405.0, -5.0, 7.0]);
        assert_eq!(u.param_info, [0.5625, 64.0, 32.0, 0.4]);
        assert_eq!(u.var_info, [1.0, 0.6, 0.0, 0.0]);
        // The save thumbnail (256x256 at zoom 1) of a 1280x720 screen at
        // zoom 2.
        let thumb = ScreenParams {
            light: [0.9, 0.8, 0.7],
            light_intensity: 0.35,
            desaturation: 0.2,
            exterior: true,
            search: SearchModeShader {
                enabled: true,
                blur: 1.0,
                radius: 4.0,
                gradient_width: 4.0,
                desat: 0.85,
                darkness: 0.0,
            },
            offscreen_width: 2560.0,
            offscreen_height: 1440.0,
            right_click: [12.0, 8.0],
            zoom: 2.0,
            tile_width: 64.0,
            ..Default::default()
        }
        .thumbnail(256, 256, 1.0);
        let u = timer.apply(thumb, 60).uniforms([4096.0, 2048.0]);
        assert_eq!((u.timer, u.timer_wrap, u.zoom), (1.0, 1.0, 0.0));
        assert_eq!(u.screen_info, [2560.0, 1440.0, 1152.0, -592.0]);
        assert_eq!(u.param_info, [1.0, 64.0, 128.0, 0.85]);
        // Six more calls: three at 60 fps, three at 30.
        let timers: Vec<_> = [60, 60, 60, 30, 30, 30]
            .iter()
            .map(|&fps| timer.next(fps))
            .collect();
        assert_eq!(
            timers,
            [
                (1.0, 1.0),
                (2.0, 1.0),
                (2.0, 1.0),
                (2.0, 1.0),
                (3.0, 1.0),
                (4.0, 1.0)
            ]
        );
    }

    #[test]
    fn zoom_flag_matches_single_col() {
        let f = |z: f32| {
            ScreenParams {
                zoom: z,
                ..Default::default()
            }
            .zoom_flag()
        };
        assert_eq!(
            [f(1.0), f(1.75), f(1.9), f(2.0), f(2.25)],
            [false, true, true, false, true]
        );
    }

    #[test]
    fn thumbnail_centres_the_search_circle() {
        let p = ScreenParams {
            offscreen_width: 1280.0,
            offscreen_height: 720.0,
            zoom: 1.5,
            right_click: [5.0, 5.0],
            ..Default::default()
        };
        let t = p.thumbnail(256, 256, 1.0);
        assert_eq!(t.right_click, [512.0, -232.0]);
        assert_eq!(t.zoom, 1.0);
    }
}
