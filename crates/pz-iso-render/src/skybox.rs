//! `SkyBox`: the sky the water and puddles reflect. `update` turns the
//! climate into the sky shader's parameters (`SkyBox.update`, called from
//! `ClimateManager.updateFx`); at most once a second `render` draws the sky
//! into the older of two 512x512 textures and makes it current
//! (`SkyBox.render` at the end of `IsoWorld.render`, then
//! `ModelManager.RenderSkyBox` and `swapTextureFBO`). Between renders the
//! water blends from the previous texture to the current one by
//! [`SkyBox::texture_shift`].
//!
//! The clock is passed in as milliseconds (`System.currentTimeMillis()`).

use pz_render::{DrawCmd, SkyBoxDraw, SkyBoxUniforms, Texture};

use crate::weather::{SkyReflection, SurfaceTextures};

/// `zombie.core.utils.UpdateLimit`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UpdateLimit {
    pub delay: i64,
    pub last: i64,
    pub last_period: i64,
}

impl UpdateLimit {
    pub fn new(delay: i64, now_ms: i64) -> Self {
        UpdateLimit { delay, last: now_ms, last_period: now_ms }
    }

    fn advance(&mut self, now_ms: i64) -> bool {
        if now_ms - self.last > self.delay {
            self.last = if now_ms - self.last > 3 * self.delay { now_ms } else { self.last + self.delay };
            return true;
        }
        false
    }

    /// `Check()`.
    pub fn check(&mut self, now_ms: i64) -> bool {
        self.advance(now_ms)
    }

    /// `updateTimePeriod()`.
    pub fn update_time_period(&mut self, now_ms: i64) {
        self.advance(now_ms);
        self.last_period = now_ms;
    }

    /// `getTimePeriod()`: 0..1 through the period since the last
    /// `update_time_period`.
    pub fn time_period(&self, now_ms: i64) -> f64 {
        ((now_ms as f64 - self.last_period as f64) / self.delay as f64).min(1.0)
    }
}

/// The `ClimateManager` and `GameTime` values `SkyBox.update` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyInputs {
    /// `GameTime.getTimeOfDay()`.
    pub time_of_day: f32,
    /// `getCurrentDay().season.getDawn()`, `getDusk()`, `getDayHighNoon()`.
    pub dawn: f32,
    pub dusk: f32,
    pub day_high_noon: f32,
    /// `getGlobalLight().getExterior()` (r, g, b, a).
    pub exterior_light: [f32; 4],
    /// `getNightStrength()`.
    pub night_strength: f32,
    /// `getCloudIntensity()`, `getPrecipitationIntensity()`,
    /// `getTemperature()`, `getFogIntensity()`.
    pub cloud_intensity: f32,
    pub precipitation_intensity: f32,
    pub temperature: f32,
    pub fog_intensity: f32,
    /// `getWindAngleIntensity()` (not corrected) and `getWindIntensity()`.
    pub wind_angle_intensity: f32,
    pub wind_intensity: f32,
}

type Rgb = [f32; 4];

fn interp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / l)
}

const SKY_H_DAY: Rgb = [0.1, 0.1, 0.4, 1.0];
const SKY_H_DAWN: Rgb = [0.2, 0.2, 0.3, 1.0];
const SKY_H_DUSK: Rgb = [0.2, 0.2, 0.3, 1.0];
const SKY_H_NIGHT: Rgb = [0.01, 0.01, 0.04, 1.0];
const SKY_L_DAY: Rgb = [0.1, 0.45, 0.7, 1.0];
const SKY_L_DAWN: Rgb = [0.1, 0.4, 0.6, 1.0];
const SKY_L_DUSK: Rgb = [0.1, 0.4, 0.6, 1.0];
const SKY_L_NIGHT: Rgb = [0.01, 0.045, 0.07, 1.0];

/// `SkyBox`'s state.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyBox {
    /// `isCurrentA`.
    pub current_a: bool,
    pub render_limit: UpdateLimit,
    is_updated: bool,
    /// `skyBoxTime` (`SBTime`), counted in renders.
    pub time: i32,
    pub cloud_count: f32,
    pub cloud_size: f32,
    pub sun_light: [f32; 3],
    pub sun_colour: Rgb,
    pub sky_h: Rgb,
    pub sky_l: Rgb,
    pub cloud_light: f32,
    pub stars: f32,
    pub fog: f32,
    pub wind: [f32; 3],
    /// `skyBoxParamWindInt`: the accumulated wind the clouds drift by.
    pub wind_int: [f32; 3],
    set_avg: bool,
}

impl SkyBox {
    /// `new SkyBox()`.
    pub fn new(now_ms: i64) -> Self {
        SkyBox {
            current_a: false,
            render_limit: UpdateLimit::new(1000, now_ms),
            is_updated: false,
            time: 0,
            cloud_count: 0.3,
            cloud_size: 0.2,
            sun_light: [0.35, 0.22, 0.3],
            sun_colour: [1.0, 0.86, 0.7, 1.0],
            sky_h: [0.1, 0.1, 0.4, 1.0],
            sky_l: [0.1, 0.45, 0.7, 1.0],
            cloud_light: 0.99,
            stars: 0.0,
            fog: 0.0,
            wind: [0.0; 3],
            wind_int: [0.0; 3],
            set_avg: false,
        }
    }

    /// `SkyBox.update(ClimateManager)`: once per render.
    pub fn update(&mut self, c: &SkyInputs) {
        if self.is_updated {
            return;
        }
        self.is_updated = true;
        let (dawn, dusk, noon, time) = (c.dawn, c.dusk, c.day_high_noon, c.time_of_day);
        let scale = |v: [f32; 3], s: f32| v.map(|x| x * s);
        if time < dawn || time > dusk {
            let total = 24.0 - dusk + dawn;
            let (t, f) = if time > dusk {
                let t = (time - dusk) / total;
                (t, (t * 5.0).min(1.0))
            } else {
                let t = (24.0 - dusk + time) / total;
                (t, ((1.0 - t) * 5.0).min(1.0))
            };
            self.sky_h = interp(SKY_H_DUSK, SKY_H_DAWN, t);
            self.sky_l = interp(SKY_L_DUSK, SKY_L_DAWN, t);
            self.sun_light = scale(normalize([0.35, 0.22, 0.3]), f);
            let e = c.exterior_light;
            self.sun_colour = e.map(|v| v * c.night_strength);
        } else if time < noon {
            let t = (time - dawn) / (noon - dawn);
            self.sky_h = interp(SKY_H_DAWN, SKY_H_DAY, t);
            self.sky_l = interp(SKY_L_DAWN, SKY_L_DAY, t);
            self.sun_light = scale(normalize([4.0 * t - 4.0, 0.22, 0.3]), (t * 10.0).min(1.0));
            self.sun_colour = c.exterior_light;
        } else {
            let t = (time - noon) / (dusk - noon);
            self.sky_h = interp(SKY_H_DAY, SKY_H_DUSK, t);
            self.sky_l = interp(SKY_L_DAY, SKY_L_DUSK, t);
            self.sun_light = scale(normalize([4.0 * t, 0.22, 0.3]), ((1.0 - t) * 10.0).min(1.0));
            self.sun_colour = c.exterior_light;
        }
        self.sky_h = interp(self.sky_h, SKY_H_NIGHT, c.night_strength);
        self.sky_l = interp(self.sky_l, SKY_L_NIGHT, c.night_strength);
        self.cloud_count = c.cloud_intensity.max(c.precipitation_intensity * 2.0).min(0.999);
        self.cloud_size = 0.02 + c.temperature / 70.0;
        self.fog = c.fog_intensity;
        self.stars = c.night_strength;
        self.cloud_light = (1.0 - (1.0 - 1000.0f64.powf((-c.precipitation_intensity - c.night_strength) as f64))) as f32;
        let mut angle = (1.0 - (c.wind_angle_intensity + 1.0) * 0.5 + 0.25) % 1.0;
        angle *= 360.0;
        let r = (angle as f64).to_radians();
        self.wind = [r.cos() as f32 * c.wind_intensity, 0.0, r.sin() as f32 * c.wind_intensity];
        // The averaged parameters the original also keeps are never read.
        if !self.set_avg {
            self.set_avg = true;
            self.wind_int = self.wind;
        } else {
            for i in 0..3 {
                self.wind_int[i] += self.wind[i];
            }
        }
    }

    /// `SkyBox.render()`: with shaders on and the sky not disabled
    /// (`Core.getPerfSkybox()` 0 or 1), once the second is up the sky is
    /// drawn into the previous texture, which then becomes current
    /// (`swapTextureFBO`). `multiplier` is `GameTime.getMultiplier()`.
    pub fn render(&mut self, now_ms: i64, multiplier: f32, use_shaders: bool, perf_skybox: i32) -> Option<DrawCmd> {
        if !use_shaders || perf_skybox == 2 {
            return None;
        }
        if !self.render_limit.check(now_ms) {
            if multiplier >= 20.0 {
                self.time += 1;
            }
            return None;
        }
        self.time += 1;
        let target = self.prev_index();
        let cmd = DrawCmd::SkyBox(Box::new(SkyBoxDraw { target, hires: perf_skybox == 0, uniforms: self.uniforms() }));
        self.is_updated = false;
        // swapTextureFBO, after the render thread has drawn it.
        self.render_limit.update_time_period(now_ms);
        self.current_a = !self.current_a;
        Some(cmd)
    }

    /// `SkyBoxShader.startRenderThread`'s uniforms.
    pub fn uniforms(&self) -> SkyBoxUniforms {
        let rgb = |c: Rgb| [c[0], c[1], c[2]];
        SkyBoxUniforms {
            time: self.time as f32,
            cloud_count: self.cloud_count,
            cloud_size: self.cloud_size,
            sun_light: self.sun_light,
            sun_colour: rgb(self.sun_colour),
            sky_h_colour: rgb(self.sky_h),
            sky_l_colour: rgb(self.sky_l),
            cloud_light: self.cloud_light,
            stars: self.stars,
            fog: self.fog,
            wind: self.wind_int,
        }
    }

    fn current_index(&self) -> usize {
        if self.current_a {
            0
        } else {
            1
        }
    }

    fn prev_index(&self) -> usize {
        1 - self.current_index()
    }

    /// `getTextureCurrent`, `getTexturePrev` and `getTextureShift`: the
    /// rendered textures, or `ref_am.png` (and the day/night blend) with
    /// shaders off or the sky disabled. `night` is `GameTime.getNight()`.
    pub fn reflection(&self, now_ms: i64, use_shaders: bool, perf_skybox: i32, night: f32, textures: &dyn SurfaceTextures) -> SkyReflection {
        if !use_shaders || perf_skybox == 2 {
            let am: Option<Texture> = textures.texture("media/textures/CMVehicleReflection/ref_am.png");
            return SkyReflection { current: am.clone(), prev: am, shift: 1.0 - night };
        }
        SkyReflection {
            current: Some(pz_render::skybox_texture(self.current_index())),
            prev: Some(pz_render::skybox_texture(self.prev_index())),
            shift: self.render_limit.time_period(now_ms) as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noon() -> SkyInputs {
        SkyInputs {
            time_of_day: 12.0,
            dawn: 6.0,
            dusk: 20.0,
            day_high_noon: 13.0,
            exterior_light: [1.0, 0.9, 0.8, 1.0],
            night_strength: 0.0,
            cloud_intensity: 0.2,
            precipitation_intensity: 0.0,
            temperature: 14.0,
            fog_intensity: 0.0,
            wind_angle_intensity: 0.0,
            wind_intensity: 0.5,
        }
    }

    #[test]
    fn renders_once_a_second_into_the_older_texture() {
        let mut sky = SkyBox::new(0);
        sky.update(&noon());
        assert!((sky.cloud_size - (0.02 + 0.2)).abs() < 1e-6);
        assert_eq!(sky.cloud_light, 1.0);
        assert!(sky.render(500, 1.0, true, 1).is_none());
        let Some(DrawCmd::SkyBox(d)) = sky.render(1001, 1.0, true, 1) else { panic!("no sky") };
        // Drawn into A (the previous texture while B is current), which is
        // now current.
        assert_eq!(d.target, 0);
        assert!(!d.hires);
        assert!(sky.current_a);
        struct None_;
        impl SurfaceTextures for None_ {
            fn texture(&self, _: &str) -> Option<Texture> {
                None
            }
        }
        let r = sky.reflection(1501, true, 1, 0.0, &None_);
        assert_eq!(r.current.unwrap().id, Some(pz_render::SKYBOX_TEXTURE_IDS[0]));
        assert!((r.shift - 0.5).abs() < 1e-6);
        // The update after a render takes the new climate.
        sky.update(&SkyInputs { cloud_intensity: 0.9, ..noon() });
        assert!((sky.cloud_count - 0.9).abs() < 1e-6);
        // Disabled: the static reflection, blended by night.
        assert_eq!(sky.reflection(0, true, 2, 0.25, &None_).shift, 0.75);
    }
}
