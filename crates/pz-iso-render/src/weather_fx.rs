//! `IsoWeatherFX`: the screen-space weather layers drawn over the world —
//! clouds, fog, snow and rain particles tiled across the view
//! (`ParticleRectangle`, `WeatherParticle` and its `CloudParticle`,
//! `FogParticle`, `SnowParticle` and `RainParticle`), the fog circle
//! around the player, and the per-layer batching of
//! `WeatherParticleDrawer`.
//!
//! The climate drives it through [`IsoWeatherFx::update_fx`]
//! (`ClimateManager.updateFx`, once per climate update) with
//! [`WeatherFxClimate::from_render_values`]; [`IsoWeatherFx::update`] runs
//! once per `IsoCell.update` and the draw happens in `IsoWorld.renderWeatherFX`
//! (see [`crate::weather_fx_mask`], which keeps the effects out of the
//! player's building). Random numbers come from `pz_core::random`'s `Rand`
//! in the original's call order: pass the shared generator
//! (`pz_core::random::with_standard`) or a seeded one.

use pz_core::random::Rand;
use pz_core::weather::ClimateRenderValues;
use pz_render::{DrawCmd, Rgba, Texture};

/// `SteppedUpdateFloat`: a value that moves towards its target by a fixed
/// step per update tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteppedUpdateFloat {
    current: f32,
    step: f32,
    target: f32,
    min: f32,
    max: f32,
}

impl SteppedUpdateFloat {
    pub const fn new(current: f32, step: f32, min: f32, max: f32) -> Self {
        SteppedUpdateFloat {
            current,
            step,
            target: current,
            min,
            max,
        }
    }

    pub fn value(&self) -> f32 {
        self.current
    }

    /// `setTarget`, clamped to the range.
    pub fn set_target(&mut self, target: f32) {
        self.target = target.min(self.max).max(self.min);
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// `overrideCurrentValue`.
    pub fn override_current_value(&mut self, f: f32) {
        self.current = f;
    }

    pub fn update(&mut self, delta: f32) {
        if self.current != self.target {
            if self.target > self.current {
                self.current += self.step * delta;
                if self.current > self.target {
                    self.current = self.target;
                }
            } else if self.target < self.current {
                self.current -= self.step * delta;
                if self.current < self.target {
                    self.current = self.target;
                }
            }
        }
    }
}

/// The layers, in `IsoWeatherFX`'s id order (`cloudId` .. `rainId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Cloud = 0,
    Fog = 1,
    Snow = 2,
    Rain = 3,
}

impl Layer {
    pub const ALL: [Layer; 4] = [Layer::Cloud, Layer::Fog, Layer::Snow, Layer::Rain];
}

/// The particle textures `IsoWeatherFX.init` and `WeatherFxMask.init` load
/// (`media/textures/weather/...`); `None` for a missing one.
#[derive(Clone, Debug, Default)]
pub struct WeatherFxTextures {
    /// `clouds_0.png` .. `clouds_5.png`.
    pub clouds: [Option<Texture>; 6],
    /// `fog_0.png` .. `fog_5.png`.
    pub fog: [Option<Texture>; 6],
    /// `snow_1.png` .. `snow_3.png`.
    pub snow: [Option<Texture>; 3],
    /// `fogcircle_tex.png` (flags 35).
    pub fog_circle: Option<Texture>,
    /// `fogwhite_tex.png` (flags 35): the rain streak and the fog border.
    pub fog_white: Option<Texture>,
}

impl WeatherFxTextures {
    /// Every path `init` loads, with the flags it passes (0 meaning the
    /// default, `getSharedTexture(name)`).
    pub fn paths() -> Vec<(String, i32)> {
        let mut v = Vec::new();
        v.extend((0..6).map(|i| (format!("media/textures/weather/clouds_{i}.png"), 0)));
        v.push(("media/textures/weather/fogcircle_tex.png".into(), 35));
        v.push(("media/textures/weather/fogwhite_tex.png".into(), 35));
        v.extend((0..6).map(|i| (format!("media/textures/weather/fog_{i}.png"), 0)));
        v.extend((1..=3).map(|i| (format!("media/textures/weather/snow_{i}.png"), 0)));
        v
    }

    /// Loads them by path with `load(path, flags)`.
    pub fn load(mut load: impl FnMut(&str, i32) -> Option<Texture>) -> Self {
        WeatherFxTextures {
            clouds: std::array::from_fn(|i| {
                load(&format!("media/textures/weather/clouds_{i}.png"), 0)
            }),
            fog_circle: load("media/textures/weather/fogcircle_tex.png", 35),
            fog_white: load("media/textures/weather/fogwhite_tex.png", 35),
            fog: std::array::from_fn(|i| load(&format!("media/textures/weather/fog_{i}.png"), 0)),
            snow: std::array::from_fn(|i| {
                load(&format!("media/textures/weather/snow_{}.png", i + 1), 0)
            }),
        }
    }
}

fn tex_size(t: &Option<Texture>) -> (i32, i32) {
    t.as_ref()
        .map_or((0, 0), |t| (t.width as i32, t.height as i32))
}

/// `RainParticle.RenderPoints`: the streak's corners, rotated.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderPoints {
    /// `origx/origy` and the rotated `x/y` of each corner.
    pub orig: [[f64; 2]; 4],
    pub points: [[f64; 2]; 4],
}

impl RenderPoints {
    fn set_dimensions(&mut self, w: f32, h: f32) {
        let c = [
            [-w / 2.0, -h / 2.0],
            [w / 2.0, -h / 2.0],
            [w / 2.0, h / 2.0],
            [-w / 2.0, h / 2.0],
        ];
        for (i, [x, y]) in c.into_iter().enumerate() {
            self.orig[i] = [f64::from(x), f64::from(y)];
            self.points[i] = self.orig[i];
        }
    }

    fn rotate(&mut self, angle: f64) {
        let (sin, cos) = (angle.sin(), angle.cos());
        for i in 0..4 {
            let [ox, oy] = self.orig[i];
            self.points[i] = [ox * cos - oy * sin, ox * sin + oy * cos];
        }
    }
}

/// The subclass state of a [`WeatherParticle`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParticleKind {
    Cloud,
    Fog,
    Snow {
        life: f32,
        incarnate_alpha: f32,
    },
    Rain {
        life: f32,
        incarnate_alpha: f32,
        last_angle: f32,
        last_intensity: f32,
        rp: RenderPoints,
    },
}

/// `WeatherParticle` with its subclass fields.
#[derive(Clone, Debug)]
pub struct WeatherParticle {
    pub kind: ParticleKind,
    pub texture: Option<Texture>,
    /// `bounds` (an `org.lwjgl.util.Rectangle`): x, y, width, height.
    pub bounds: [i32; 4],
    /// `color` (white but for rain).
    pub color: [f32; 4],
    pub position: [f32; 2],
    pub velocity: [f32; 2],
    pub alpha: f32,
    pub speed: f32,
    pub angle_offset: f32,
    pub alpha_fade_mod: SteppedUpdateFloat,
    pub render_alpha: f32,
    pub o_width: f32,
    pub o_height: f32,
    pub zoom_multi_w: f32,
    pub zoom_multi_h: f32,
    pub recalc_size_on_zoom: bool,
    pub last_zoom_mod: f32,
    /// `lastAngle` of the cloud, fog and snow subclasses.
    pub last_angle: f32,
}

impl WeatherParticle {
    fn new(kind: ParticleKind, texture: Option<Texture>, w: i32, h: i32) -> Self {
        WeatherParticle {
            kind,
            texture,
            bounds: [0, 0, w, h],
            color: [1.0; 4],
            position: [0.0; 2],
            velocity: [0.0; 2],
            alpha: 1.0,
            speed: 0.0,
            angle_offset: 0.0,
            alpha_fade_mod: SteppedUpdateFloat::new(0.0, 0.1, 0.0, 1.0),
            render_alpha: 0.0,
            o_width: w as f32,
            o_height: h as f32,
            zoom_multi_w: 0.0,
            zoom_multi_h: 0.0,
            recalc_size_on_zoom: false,
            last_zoom_mod: -1.0,
            last_angle: -1.0,
        }
    }

    /// `new RainParticle(texture, height)`.
    fn new_rain(rand: &Rand, texture: Option<Texture>, height: i32) -> Self {
        let (tw, th) = tex_size(&texture);
        let mut p = WeatherParticle::new(ParticleKind::Cloud, texture, tw, th);
        let w = if height > 6 { rand.next_range(1, 2) } else { 1 };
        p.bounds[2] = w;
        p.bounds[3] = height;
        p.o_width = w as f32;
        p.o_height = height as f32;
        p.recalc_size_on_zoom = true;
        p.zoom_multi_w = 0.0;
        p.zoom_multi_h = 2.0;
        let life = rain_life(rand);
        let mut rp = RenderPoints::default();
        rp.set_dimensions(p.o_width, p.o_height);
        p.kind = ParticleKind::Rain {
            life,
            incarnate_alpha: 1.0,
            last_angle: -1.0,
            last_intensity: -1.0,
            rp,
        };
        p
    }

    /// `update(delta, doBounds)`: fades, wraps into the rectangle.
    fn update_base(&mut self, delta: f32, do_bounds: bool, pw: i32, ph: i32) {
        self.alpha_fade_mod.update(delta);
        self.position[0] = wrap(self.position[0], pw);
        self.position[1] = wrap(self.position[1], ph);
        if do_bounds {
            self.bounds[0] = self.position[0] as i32 - self.bounds[2] / 2;
            self.bounds[1] = self.position[1] as i32 - self.bounds[3] / 2;
        }
    }

    /// `updateZoomSize()`.
    fn update_zoom_size(&mut self, zoom_mod: f32) -> bool {
        if self.recalc_size_on_zoom && self.last_zoom_mod != zoom_mod {
            self.last_zoom_mod = zoom_mod;
            self.o_width = self.bounds[2] as f32;
            self.o_height = self.bounds[3] as f32;
            if self.last_zoom_mod > 0.0 {
                self.o_width *= 1.0 + zoom_mod * self.zoom_multi_w;
                self.o_height *= 1.0 + zoom_mod * self.zoom_multi_h;
            }
            return true;
        }
        false
    }

    /// `isOnScreen(offsetx, offsety)` for an offscreen buffer of `sw` x `sh`.
    pub fn is_on_screen(&self, offset_x: f32, offset_y: f32, sw: i32, sh: i32) -> bool {
        let x1 = offset_x + self.bounds[0] as f32;
        let y1 = offset_y + self.bounds[1] as f32;
        let x2 = x1 + self.o_width;
        let y2 = y1 + self.o_height;
        if x1 >= sw as f32 || x2 <= 0.0 {
            return false;
        }
        y1 < sh as f32 && y2 > 0.0
    }

    /// `velocity.set(cos(a) * speed, sin(a) * speed)` for an angle in
    /// degrees.
    fn set_velocity(&mut self, deg: f32) -> f64 {
        let r = f64::from(deg).to_radians();
        self.velocity = [r.cos() as f32 * self.speed, r.sin() as f32 * self.speed];
        r
    }
}

/// `WeatherParticle.update`'s wrap of a coordinate into `0..=size`.
fn wrap(v: f32, size: i32) -> f32 {
    let s = size as f32;
    if v > s {
        v - ((v / s) as i32 * size) as f32
    } else if v < 0.0 {
        v - (((v - s) / s) as i32 * size) as f32
    } else {
        v
    }
}

fn wrap_angle(mut a: f32) -> f32 {
    if a > 360.0 {
        a -= 360.0;
    }
    if a < 0.0 {
        a += 360.0;
    }
    a
}

/// `RainParticle.setLife()`.
fn rain_life(rand: &Rand) -> f32 {
    rand.next_range(20, 60) as f32
}

/// `SnowParticle.setLife()`.
fn snow_life(rand: &Rand) -> f32 {
    80.0 + rand.next_range(60, 500) as f32
}

/// `ParticleRectangle`: a tile of particles repeated across the screen.
#[derive(Clone, Debug)]
pub struct ParticleRectangle {
    pub id: Layer,
    pub width: i32,
    pub height: i32,
    pub particles: Vec<WeatherParticle>,
    pub particles_to_render: i32,
    pub particles_req_upd_cnt: i32,
}

impl ParticleRectangle {
    fn new(id: Layer, width: i32, height: i32, particles: Vec<WeatherParticle>) -> Self {
        let n = particles.len() as i32;
        ParticleRectangle {
            id,
            width,
            height,
            particles,
            particles_to_render: n,
            particles_req_upd_cnt: 0,
        }
    }

    /// `SetParticlesStrength(str)`.
    pub fn set_particles_strength(&mut self, s: f32) {
        self.particles_to_render = (self.particles.len() as f32 * s) as i32;
    }

    /// `requiresUpdate()`.
    pub fn requires_update(&self) -> bool {
        self.particles_to_render > 0 || self.particles_req_upd_cnt > 0
    }
}

/// What a particle update reads from `IsoWeatherFX.instance`.
#[derive(Clone, Copy, Debug)]
struct FxShared {
    wind_angle: f32,
    wind_angle_clouds: f32,
    wind_precip_intensity: f32,
    wind_intensity: f32,
    wind_speed: f32,
    wind_speed_fog: f32,
    fog_intensity: f32,
    cloud_intensity: f32,
    indoors_alpha_mod: f32,
    player_indoors: bool,
    zoom_mod: f32,
    precipitation_speed: f32,
}

impl ParticleRectangle {
    /// `update(delta)`.
    fn update(&mut self, rand: &Rand, delta: f32, fx: &FxShared) {
        self.particles_req_upd_cnt = 0;
        let (pw, ph) = (self.width, self.height);
        for i in 0..self.particles.len() {
            let p = &mut self.particles[i];
            p.alpha_fade_mod
                .set_target(if (i as i32) < self.particles_to_render {
                    1.0
                } else {
                    0.0
                });
            update_particle(p, rand, delta, fx, pw, ph);
            if p.render_alpha > 0.0 {
                self.particles_req_upd_cnt += 1;
            }
        }
    }
}

/// The subclasses' `update(delta)`.
fn update_particle(
    p: &mut WeatherParticle,
    rand: &Rand,
    delta: f32,
    fx: &FxShared,
    pw: i32,
    ph: i32,
) {
    match p.kind {
        ParticleKind::Cloud => {
            // The `lastIntensity` test is always true: every tick.
            let a = wrap_angle(fx.wind_angle_clouds);
            p.set_velocity(a);
            p.last_angle = fx.wind_angle_clouds;
            p.position[0] += p.velocity[0] * fx.wind_speed_fog * delta;
            p.position[1] += p.velocity[1] * fx.wind_speed_fog * delta;
            p.update_base(delta, true, pw, ph);
            let alpha_mod = fx.cloud_intensity * 0.3;
            p.render_alpha = p.alpha * alpha_mod * p.alpha_fade_mod.value() * fx.indoors_alpha_mod;
        }
        ParticleKind::Fog => {
            let a = wrap_angle(
                fx.wind_angle + (p.angle_offset - p.angle_offset * 1.0 * fx.wind_intensity),
            );
            p.set_velocity(a);
            p.last_angle = fx.wind_angle;
            p.position[0] += p.velocity[0] * fx.wind_speed_fog * delta;
            p.position[1] += p.velocity[1] * fx.wind_speed_fog * delta;
            p.update_base(delta, true, pw, ph);
            let alpha_mod = fx.fog_intensity;
            p.render_alpha = p.alpha * alpha_mod * p.alpha_fade_mod.value() * fx.indoors_alpha_mod;
        }
        ParticleKind::Snow {
            mut life,
            mut incarnate_alpha,
        } => {
            let a = wrap_angle(
                fx.wind_angle + (p.angle_offset - p.angle_offset * 0.5 * fx.wind_precip_intensity),
            );
            p.set_velocity(a);
            p.last_angle = fx.wind_angle;
            if life >= 80.0 {
                p.position[0] += p.velocity[0] * fx.wind_speed * delta * fx.precipitation_speed;
                p.position[1] += p.velocity[1] * fx.wind_speed * delta * fx.precipitation_speed;
            } else {
                incarnate_alpha = life / 80.0;
            }
            life -= 1.0;
            if life < 0.0 {
                life = snow_life(rand);
                incarnate_alpha = 0.0;
                let x = rand.next_range(0, pw);
                let y = rand.next_range(0, ph);
                p.position = [x as f32, y as f32];
            }
            if incarnate_alpha < 1.0 {
                incarnate_alpha += 0.05;
                if incarnate_alpha > 1.0 {
                    incarnate_alpha = 1.0;
                }
            }
            p.kind = ParticleKind::Snow {
                life,
                incarnate_alpha,
            };
            p.update_base(delta, true, pw, ph);
            p.update_zoom_size(fx.zoom_mod);
            let alpha_mod = 1.0 - 0.2 * fx.wind_intensity;
            p.render_alpha = p.alpha
                * alpha_mod
                * p.alpha_fade_mod.value()
                * fx.indoors_alpha_mod
                * incarnate_alpha;
            p.render_alpha *= 0.7;
        }
        ParticleKind::Rain {
            mut life,
            mut incarnate_alpha,
            mut last_angle,
            mut last_intensity,
            mut rp,
        } => {
            let mut angle_update = false;
            if p.update_zoom_size(fx.zoom_mod) {
                rp.set_dimensions(p.o_width, p.o_height);
                angle_update = true;
            }
            let mut tmp_angle = 0.0f32;
            let recompute = angle_update
                || last_angle != fx.wind_angle
                || last_intensity != fx.wind_precip_intensity;
            if recompute {
                tmp_angle = wrap_angle(
                    fx.wind_angle
                        + (p.angle_offset - p.angle_offset * 0.5 * fx.wind_precip_intensity),
                );
                p.set_velocity(tmp_angle);
                last_angle = fx.wind_angle;
                last_intensity = fx.wind_precip_intensity;
                angle_update = true;
            }
            let k = 1.0 + fx.wind_speed * 0.1;
            p.position[0] += p.velocity[0] * k * delta * fx.precipitation_speed;
            p.position[1] += p.velocity[1] * k * delta * fx.precipitation_speed;
            life -= 1.0;
            if life < 0.0 {
                life = rain_life(rand);
                incarnate_alpha = 0.0;
                let x = rand.next_range(0, pw);
                let y = rand.next_range(0, ph);
                p.position = [x as f32, y as f32];
            }
            if incarnate_alpha < 1.0 {
                incarnate_alpha += 0.035;
                if incarnate_alpha > 1.0 {
                    incarnate_alpha = 1.0;
                }
            }
            p.update_base(delta, false, pw, ph);
            p.bounds[0] = p.position[0] as i32;
            p.bounds[1] = p.position[1] as i32;
            if angle_update {
                // Always recomputed above when set.
                let a = wrap_angle(tmp_angle + 90.0);
                rp.rotate(f64::from(a).to_radians());
            }
            let alpha_mod = 1.0 - 0.2 * fx.wind_intensity;
            p.render_alpha = p.alpha
                * alpha_mod
                * p.alpha_fade_mod.value()
                * fx.indoors_alpha_mod
                * incarnate_alpha;
            p.render_alpha *= 0.55;
            if fx.player_indoors {
                p.render_alpha *= 0.5;
            }
            p.kind = ParticleKind::Rain {
                life,
                incarnate_alpha,
                last_angle,
                last_intensity,
                rp,
            };
        }
    }
}

/// One `WeatherParticleDrawer` quad: corners clockwise from the top left
/// and the colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleQuad {
    pub x: [f32; 4],
    pub y: [f32; 4],
    pub rgba: [f32; 4],
}

/// `WeatherParticleDrawer`: one layer's particles for a frame, grouped by
/// texture in order of first use.
#[derive(Clone, Debug, Default)]
pub struct WeatherParticleDrawer {
    pub textures: Vec<Option<Texture>>,
    pub by_texture: Vec<Vec<ParticleQuad>>,
}

impl WeatherParticleDrawer {
    /// `startFrame()`.
    pub fn start_frame(&mut self) {
        self.textures.clear();
        self.by_texture.clear();
    }

    /// `addParticle(texture, x1, y1, .. x4, y4, r, g, b, a)`.
    pub fn add_particle(&mut self, texture: &Option<Texture>, q: ParticleQuad) {
        let i = match self.textures.iter().position(|t| t == texture) {
            Some(i) => i,
            None => {
                self.textures.push(texture.clone());
                self.by_texture.push(Vec::new());
                self.textures.len() - 1
            }
        };
        self.by_texture[i].push(q);
    }

    pub fn is_empty(&self) -> bool {
        self.by_texture.iter().all(Vec::is_empty)
    }

    /// `endFrame()` then the drawer's `render()`: the quads through the
    /// `vboRenderer_PositionColorUV` shader with float colours, the alpha
    /// test off between `glPushAttrib` and `glPopAttrib`, in a view of
    /// `width` x `height` units (`ortho2D(0, w, h, 0)`).
    pub fn end_frame(&self, width: f32, height: f32, out: &mut Vec<DrawCmd>) {
        if self.is_empty() {
            return;
        }
        out.push(DrawCmd::PushAttrib);
        out.push(DrawCmd::GlDisable(pz_render::gl::ALPHA_TEST));
        for (tex, quads) in self.textures.iter().zip(&self.by_texture) {
            // A missing texture would have thrown in the original.
            let Some(t) = tex else { continue };
            let uv = [
                [t.x_start, t.y_start],
                [t.x_end, t.y_start],
                [t.x_end, t.y_end],
                [t.x_start, t.y_end],
            ];
            for q in quads {
                let pos = std::array::from_fn(|i| {
                    [2.0 * q.x[i] / width - 1.0, 1.0 - 2.0 * q.y[i] / height, 0.5]
                });
                out.push(DrawCmd::ProjectedQuadF32 {
                    texture: t.clone(),
                    pos,
                    uv,
                    color: q.rgba,
                    nearest: false,
                });
            }
        }
        out.push(DrawCmd::PopAttrib);
    }
}

/// The `ClimateManager` values `updateFx` passes to the effects.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeatherFxClimate {
    pub precipitation_intensity: f32,
    pub wind_intensity: f32,
    /// `(float)windTickFinal * (float)windTickFinal`.
    pub wind_precip_intensity: f32,
    pub wind_angle_intensity: f32,
    pub fog_intensity: f32,
    pub cloud_intensity: f32,
    pub precipitation_is_snow: bool,
}

impl WeatherFxClimate {
    /// The final values `ClimateManager.updateFx` reads.
    pub fn from_render_values(v: &ClimateRenderValues) -> Self {
        WeatherFxClimate {
            precipitation_intensity: v.precipitation_intensity,
            wind_intensity: v.wind_intensity,
            wind_precip_intensity: v.wind_precip_intensity,
            wind_angle_intensity: v.wind_angle_intensity,
            fog_intensity: v.cm_fog_intensity,
            cloud_intensity: v.cloud_intensity,
            precipitation_is_snow: v.precipitation_is_snow,
        }
    }
}

/// The sandbox caps on the effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeatherFxSandbox {
    /// `SandboxOptions.maxFogIntensity` (1 normal, 2 moderate, 3 low, 4
    /// none).
    pub max_fog_intensity: i32,
    /// `SandboxOptions.maxRainFxIntensity` (1 normal, 2 moderate, 3 low).
    pub max_rain_fx_intensity: i32,
}

impl Default for WeatherFxSandbox {
    fn default() -> Self {
        WeatherFxSandbox {
            max_fog_intensity: 1,
            max_rain_fx_intensity: 1,
        }
    }
}

/// What `IsoWeatherFX.update` reads each tick besides its own state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherFxFrame {
    /// `GameTime.getMultiplier()`.
    pub multiplier: f32,
    /// The camera character's square exists and is not `exterior`.
    pub player_indoors: bool,
    /// `WeatherFxMask.playerHasMaskToDraw(playerIndex)`.
    pub player_has_mask_to_draw: bool,
    /// `Core.getZoom(IsoPlayer.getInstance().getIndex())`.
    pub zoom: f32,
    /// `Core.isZoomEnabled()`.
    pub zoom_enabled: bool,
    /// `Core.getOptionPrecipitationSpeedMultiplier()`.
    pub precipitation_speed: f32,
}

impl Default for WeatherFxFrame {
    fn default() -> Self {
        WeatherFxFrame {
            multiplier: 1.0,
            player_indoors: false,
            player_has_mask_to_draw: false,
            zoom: 1.0,
            zoom_enabled: true,
            precipitation_speed: 1.0,
        }
    }
}

/// The view a frame's effects are drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherFxView {
    /// `IsoCamera.frameState.offX/offY`.
    pub off_x: f32,
    pub off_y: f32,
    /// `IsoCamera.frameState.offscreenWidth/Height`: the zoomed view size.
    pub offscreen_width: i32,
    pub offscreen_height: i32,
    /// `IsoCamera.getScreenWidth/Height(player)`.
    pub screen_width: i32,
    pub screen_height: i32,
    /// `Core.getCurrentPlayerZoom()`.
    pub zoom: f32,
    /// `IsoCamera.getRightClickOffX/Y()`.
    pub right_click_off_x: f32,
    pub right_click_off_y: f32,
    /// `PerformanceSettings.fogQuality` (2 draws the fog particles and
    /// circle).
    pub fog_quality: i32,
    /// `Core.getOptionRenderPrecipitation()` (1 always, 2 outdoors only,
    /// 3 never).
    pub render_precipitation: i32,
}

impl WeatherFxView {
    /// A player view of `width` x `height` pixels at `zoom` with the
    /// camera offset `off`.
    pub fn new(off_x: f32, off_y: f32, width: i32, height: i32, zoom: f32) -> Self {
        WeatherFxView {
            off_x,
            off_y,
            offscreen_width: (width as f32 * zoom) as i32,
            offscreen_height: (height as f32 * zoom) as i32,
            screen_width: width,
            screen_height: height,
            zoom,
            right_click_off_x: 0.0,
            right_click_off_y: 0.0,
            fog_quality: 2,
            render_precipitation: 1,
        }
    }
}

/// `IsoWeatherFX`.
#[derive(Clone, Debug)]
pub struct IsoWeatherFx {
    /// The four layers by [`Layer`] id.
    pub rects: [ParticleRectangle; 4],
    pub drawers: [WeatherParticleDrawer; 4],
    pub player_indoors: bool,
    pub wind_precip_intensity: SteppedUpdateFloat,
    pub wind_intensity: SteppedUpdateFloat,
    pub wind_angle_intensity: SteppedUpdateFloat,
    pub precipitation_intensity: SteppedUpdateFloat,
    pub precipitation_intensity_snow: SteppedUpdateFloat,
    pub precipitation_intensity_rain: SteppedUpdateFloat,
    pub cloud_intensity: SteppedUpdateFloat,
    pub fog_intensity: SteppedUpdateFloat,
    pub wind_angle_mod: SteppedUpdateFloat,
    pub precipitation_is_snow: bool,
    pub fog_overlay_alpha: f32,
    pub wind_speed: f32,
    pub wind_speed_fog: f32,
    pub wind_angle: f32,
    pub wind_angle_clouds: f32,
    pub indoors_alpha_mod: SteppedUpdateFloat,
    pub wind_upd_counter: f32,
    /// The static `zoomMod` (1 until the first update).
    pub zoom_mod: f32,
    pub fog_circle: Option<Texture>,
    pub fog_white: Option<Texture>,
}

impl IsoWeatherFx {
    /// `new IsoWeatherFX()` then `init()`.
    pub fn new(rand: &Rand, tex: &WeatherFxTextures) -> Self {
        let mut clouds = Vec::with_capacity(16);
        let (cw, ch) = (8192, 4096);
        for _ in 0..16 {
            let t = tex.clouds[rand.next(6) as usize].clone();
            let (w, h) = tex_size(&t);
            let mut p = WeatherParticle::new(ParticleKind::Cloud, t, w * 8, h * 8);
            p.position = [rand.next_range(0, cw) as f32, rand.next_range(0, ch) as f32];
            p.speed = rand.next_float(0.01, 0.1);
            p.angle_offset = 180.0 - rand.next_float(0.0, 360.0);
            p.alpha = rand.next_float(0.25, 0.75);
            clouds.push(p);
        }
        let mut fog = Vec::with_capacity(16);
        let (fw, fh) = (2048, 1024);
        for _ in 0..16 {
            let t = tex.fog[rand.next(6) as usize].clone();
            let (w, h) = tex_size(&t);
            let mut p = WeatherParticle::new(ParticleKind::Fog, t, w * 2, h * 2);
            p.position = [rand.next_range(0, fw) as f32, rand.next_range(0, fh) as f32];
            p.speed = rand.next_float(0.01, 0.1);
            p.angle_offset = 180.0 - rand.next_float(0.0, 360.0);
            p.alpha = rand.next_float(0.05, 0.25);
            fog.push(p);
        }
        let mut snow = Vec::with_capacity(1024);
        for _ in 0..1024 {
            let t = tex.snow[rand.next(3) as usize].clone();
            let (w, h) = tex_size(&t);
            let mut p = WeatherParticle::new(
                ParticleKind::Snow {
                    life: 0.0,
                    incarnate_alpha: 1.0,
                },
                t,
                w,
                h,
            );
            p.recalc_size_on_zoom = true;
            p.zoom_multi_w = 1.0;
            p.zoom_multi_h = 1.0;
            p.position = [
                rand.next_range(0, 512) as f32,
                rand.next_range(0, 512) as f32,
            ];
            p.speed = rand.next_float(1.0, 2.0);
            p.angle_offset = 15.0 - rand.next_float(0.0, 30.0);
            p.alpha = rand.next_float(0.25, 0.6);
            snow.push(p);
        }
        let mut rain = Vec::with_capacity(1024);
        for _ in 0..1024 {
            let height = rand.next_range(5, 12);
            let mut p = WeatherParticle::new_rain(rand, tex.fog_white.clone(), height);
            p.position = [
                rand.next_range(0, 512) as f32,
                rand.next_range(0, 512) as f32,
            ];
            p.speed = rand.next_range(7, 12) as f32;
            p.angle_offset = 3.0 - rand.next_float(0.0, 6.0);
            p.alpha = rand.next_float(0.5, 0.8);
            let r = rand.next_float(0.75, 0.8);
            let g = rand.next_float(0.85, 0.9);
            let b = rand.next_float(0.95, 1.0);
            p.color = [r, g, b, 1.0];
            rain.push(p);
        }
        let mut cloud_rect = ParticleRectangle::new(Layer::Cloud, cw, ch, clouds);
        cloud_rect.set_particles_strength(1.0);
        let mut fog_rect = ParticleRectangle::new(Layer::Fog, fw, fh, fog);
        fog_rect.set_particles_strength(1.0);
        IsoWeatherFx {
            rects: [
                cloud_rect,
                fog_rect,
                ParticleRectangle::new(Layer::Snow, 512, 512, snow),
                ParticleRectangle::new(Layer::Rain, 512, 512, rain),
            ],
            drawers: Default::default(),
            player_indoors: false,
            wind_precip_intensity: SteppedUpdateFloat::new(0.0, 0.025, 0.0, 1.0),
            wind_intensity: SteppedUpdateFloat::new(0.0, 0.005, 0.0, 1.0),
            wind_angle_intensity: SteppedUpdateFloat::new(0.0, 0.005, -1.0, 1.0),
            precipitation_intensity: SteppedUpdateFloat::new(0.0, 0.005, 0.0, 1.0),
            precipitation_intensity_snow: SteppedUpdateFloat::new(0.0, 0.005, 0.0, 1.0),
            precipitation_intensity_rain: SteppedUpdateFloat::new(0.0, 0.005, 0.0, 1.0),
            cloud_intensity: SteppedUpdateFloat::new(0.0, 0.005, 0.0, 1.0),
            fog_intensity: SteppedUpdateFloat::new(0.0, 0.005, 0.0, 1.0),
            wind_angle_mod: SteppedUpdateFloat::new(0.0, 0.005, 0.0, 1.0),
            precipitation_is_snow: true,
            fog_overlay_alpha: 0.0,
            wind_speed: 0.0,
            wind_speed_fog: 0.0,
            wind_angle: 90.0,
            wind_angle_clouds: 90.0,
            indoors_alpha_mod: SteppedUpdateFloat::new(1.0, 0.05, 0.0, 1.0),
            wind_upd_counter: 0.0,
            zoom_mod: 1.0,
            fog_circle: tex.fog_circle.clone(),
            fog_white: tex.fog_white.clone(),
        }
    }

    /// `setFogIntensity`, with the sandbox cap.
    pub fn set_fog_intensity(&mut self, mut intensity: f32, sandbox: &WeatherFxSandbox) {
        match sandbox.max_fog_intensity {
            2 => intensity = intensity.min(0.75),
            3 => intensity = intensity.min(0.5),
            4 => intensity = intensity.min(0.0),
            _ => {}
        }
        self.fog_intensity.set_target(intensity);
    }

    /// `setPrecipitationIntensity`, with the sandbox cap.
    pub fn set_precipitation_intensity(&mut self, mut intensity: f32, sandbox: &WeatherFxSandbox) {
        match sandbox.max_rain_fx_intensity {
            2 => intensity *= 0.75,
            3 => intensity *= 0.5,
            _ => {}
        }
        if intensity > 0.0 {
            intensity = 0.05 + 0.95 * intensity;
        }
        self.precipitation_intensity.set_target(intensity);
    }

    /// `ClimateManager.updateFx`'s calls.
    pub fn update_fx(&mut self, c: &WeatherFxClimate, sandbox: &WeatherFxSandbox) {
        self.set_precipitation_intensity(c.precipitation_intensity, sandbox);
        self.wind_intensity.set_target(c.wind_intensity);
        self.wind_precip_intensity
            .set_target(c.wind_precip_intensity);
        self.wind_angle_intensity.set_target(c.wind_angle_intensity);
        self.set_fog_intensity(c.fog_intensity, sandbox);
        self.cloud_intensity.set_target(c.cloud_intensity);
        self.precipitation_is_snow = c.precipitation_is_snow;
    }

    /// `update()`, once per cell update.
    pub fn update(&mut self, rand: &Rand, f: &WeatherFxFrame) {
        self.player_indoors = f.player_indoors;
        let delta = f.multiplier;
        if !f.player_has_mask_to_draw {
            let v = self.indoors_alpha_mod.value();
            if self.player_indoors && v > 0.0 {
                self.indoors_alpha_mod.set_target(v - 0.05 * delta);
            } else if !self.player_indoors && v < 1.0 {
                self.indoors_alpha_mod.set_target(v + 0.05 * delta);
            }
        } else {
            self.indoors_alpha_mod.set_target(1.0);
        }
        self.indoors_alpha_mod.update(delta);
        self.cloud_intensity.update(delta);
        self.wind_intensity.update(delta);
        self.wind_precip_intensity.update(delta);
        self.wind_angle_intensity.update(delta);
        self.precipitation_intensity.update(delta);
        self.fog_intensity.update(delta);
        let p = self.precipitation_intensity.target();
        self.precipitation_intensity_snow
            .set_target(if self.precipitation_is_snow { p } else { 0.0 });
        self.precipitation_intensity_rain
            .set_target(if self.precipitation_is_snow { 0.0 } else { p });
        self.wind_angle_mod
            .set_target(if self.precipitation_is_snow { 0.3 } else { 0.6 });
        self.precipitation_intensity_snow.update(delta);
        self.precipitation_intensity_rain.update(delta);
        self.wind_angle_mod.update(delta);
        self.fog_overlay_alpha =
            0.8 * (self.fog_intensity.value() * self.indoors_alpha_mod.value());
        self.wind_upd_counter += 1.0;
        if self.wind_upd_counter > 15.0 {
            self.wind_upd_counter = 0.0;
            let wai = self.wind_angle_intensity.value();
            let wpi = self.wind_precip_intensity.value();
            if wai > 0.0 {
                self.wind_angle = lerp(wpi, 90.0, 0.0 + 54.0 * self.wind_angle_mod.value());
                self.wind_angle_clouds = if wai < 0.5 {
                    lerp(wai * 2.0, 90.0, 0.0)
                } else {
                    lerp((wai - 0.5) * 2.0, 360.0, 270.0)
                };
            } else if wai < 0.0 {
                self.wind_angle = lerp(wpi.abs(), 90.0, 180.0 - 54.0 * self.wind_angle_mod.value());
                self.wind_angle_clouds = lerp(wai.abs(), 90.0, 270.0);
            } else {
                self.wind_angle = 90.0;
            }
            self.wind_speed = 6.0 * wpi;
            self.wind_speed_fog = 6.0 * self.wind_intensity.value() * (4.0 + 16.0 * wai.abs());
            if self.wind_speed < 1.0 {
                self.wind_speed = 1.0;
            }
            if self.wind_speed_fog < 1.0 {
                self.wind_speed_fog = 1.0;
            }
        }
        let zoom = f.zoom;
        let m = 1.0 - (zoom - 0.5) * 0.5 * 0.75;
        self.zoom_mod = 0.0;
        if f.zoom_enabled && zoom > 1.0 {
            self.zoom_mod = ((zoom - 1.0) * 0.666_666_7).clamp(0.0, 1.0);
        }
        let cloud = if self.cloud_intensity.value() <= 0.0 {
            0.0
        } else {
            1.0
        };
        self.rects[Layer::Cloud as usize].set_particles_strength(cloud);
        let fog = if self.fog_intensity.value() <= 0.0 {
            0.0
        } else {
            1.0
        };
        self.rects[Layer::Fog as usize].set_particles_strength(fog);
        self.rects[Layer::Snow as usize]
            .set_particles_strength(self.precipitation_intensity_snow.value() * m);
        self.rects[Layer::Rain as usize]
            .set_particles_strength(self.precipitation_intensity_rain.value() * m);
        let shared = FxShared {
            wind_angle: self.wind_angle,
            wind_angle_clouds: self.wind_angle_clouds,
            wind_precip_intensity: self.wind_precip_intensity.value(),
            wind_intensity: self.wind_intensity.value(),
            wind_speed: self.wind_speed,
            wind_speed_fog: self.wind_speed_fog,
            fog_intensity: self.fog_intensity.value(),
            cloud_intensity: self.cloud_intensity.value(),
            indoors_alpha_mod: self.indoors_alpha_mod.value(),
            player_indoors: self.player_indoors,
            zoom_mod: self.zoom_mod,
            precipitation_speed: f.precipitation_speed,
        };
        for r in &mut self.rects {
            if r.requires_update() {
                r.update(rand, delta, &shared);
            }
        }
    }

    /// `hasCloudsToRender()`.
    pub fn has_clouds_to_render(&self) -> bool {
        self.cloud_intensity.value() > 0.0 || self.rects[Layer::Cloud as usize].requires_update()
    }

    /// `hasPrecipitationToRender()`.
    pub fn has_precipitation_to_render(&self) -> bool {
        self.precipitation_intensity.value() > 0.0
            || self.rects[Layer::Snow as usize].requires_update()
            || self.rects[Layer::Rain as usize].requires_update()
    }

    /// `hasFogToRender()`.
    pub fn has_fog_to_render(&self) -> bool {
        self.fog_intensity.value() > 0.0 || self.rects[Layer::Fog as usize].requires_update()
    }

    /// The drawers' `startFrame` (`IsoWorld.renderWeatherFX`).
    pub fn start_frame(&mut self) {
        self.drawers
            .iter_mut()
            .for_each(WeatherParticleDrawer::start_frame);
    }

    /// `ParticleRectangle.render()` into the layer's drawer: the tile
    /// repeated over the view from the camera offset.
    pub fn render_rect(&mut self, layer: Layer, v: &WeatherFxView) {
        let r = &self.rects[layer as usize];
        let d = &mut self.drawers[layer as usize];
        let (sw, sh) = (v.offscreen_width, v.offscreen_height);
        let cells_w = sw / r.width + 2;
        let cells_h = sh / r.height + 2;
        let grid_x = if v.off_x >= 0.0 {
            v.off_x as i32 % r.width
        } else {
            r.width - (v.off_x.abs() as i32) % r.width
        };
        let grid_y = if v.off_y >= 0.0 {
            v.off_y as i32 % r.height
        } else {
            r.height - (v.off_y.abs() as i32) % r.height
        };
        for cy in -1..cells_h {
            for cx in -1..cells_w {
                let x = -grid_x + cx * r.width;
                let y = -grid_y + cy * r.height;
                for p in &r.particles {
                    let (xf, yf) = (x as f32, y as f32);
                    if p.render_alpha <= 0.0 || !p.is_on_screen(xf, yf, sw, sh) {
                        continue;
                    }
                    d.add_particle(&p.texture, particle_quad(p, xf, yf));
                }
            }
        }
    }

    /// One layer as `render`/`renderLayered` draw it: the rectangle and
    /// the drawer's `endFrame`.
    fn render_layer(&mut self, layer: Layer, v: &WeatherFxView, out: &mut Vec<DrawCmd>) {
        self.render_rect(layer, v);
        self.drawers[layer as usize].end_frame(
            v.offscreen_width as f32,
            v.offscreen_height as f32,
            out,
        );
    }

    /// `render()`: every layer straight into the current target (no mask).
    pub fn render(&mut self, v: &WeatherFxView, out: &mut Vec<DrawCmd>) {
        for layer in Layer::ALL {
            if layer == Layer::Fog {
                if v.fog_quality != 2 {
                    continue;
                }
                self.render_fog_circle(v, out);
            }
            if (matches!(layer, Layer::Rain | Layer::Snow) && v.render_precipitation > 2)
                || !self.rects[layer as usize].requires_update()
            {
                continue;
            }
            self.render_layer(layer, v, out);
        }
    }

    /// `renderClouds()`.
    pub fn render_clouds(&mut self, v: &WeatherFxView, out: &mut Vec<DrawCmd>) {
        if self.rects[Layer::Cloud as usize].requires_update() {
            self.render_layer(Layer::Cloud, v, out);
        }
    }

    /// `renderFog()`.
    pub fn render_fog(&mut self, v: &WeatherFxView, out: &mut Vec<DrawCmd>) {
        self.render_fog_circle(v, out);
        if self.rects[Layer::Fog as usize].requires_update() {
            self.render_layer(Layer::Fog, v, out);
        }
    }

    /// `renderPrecipitation()`.
    pub fn render_precipitation(&mut self, v: &WeatherFxView, out: &mut Vec<DrawCmd>) {
        for layer in [Layer::Snow, Layer::Rain] {
            if self.rects[layer as usize].requires_update() {
                self.render_layer(layer, v, out);
            }
        }
    }

    /// `renderLayered(doClouds, doFog, doPrecip)`.
    pub fn render_layered(
        &mut self,
        clouds: bool,
        fog: bool,
        precip: bool,
        v: &WeatherFxView,
        out: &mut Vec<DrawCmd>,
    ) {
        if clouds {
            self.render_clouds(v, out);
        } else if fog {
            self.render_fog(v, out);
        } else if precip {
            self.render_precipitation(v, out);
        }
    }

    /// The fog circle's five rectangles (`renderi` calls) in screen pixels:
    /// the circle, then the white borders left, top, right and bottom.
    pub fn fog_circle_rects(&self, v: &WeatherFxView) -> [[i32; 4]; 5] {
        let (sw, sh) = (v.screen_width, v.screen_height);
        let fi = self.fog_intensity.value();
        let mut cw = 2048 - (512.0 * fi) as i32;
        let mut ch = 1024 - (256.0 * fi) as i32;
        cw = (cw as f32 / v.zoom) as i32;
        ch = (ch as f32 / v.zoom) as i32;
        let mut cx = sw / 2 - cw / 2;
        let mut cy = sh / 2 - ch / 2;
        cx = (cx as f32 - v.right_click_off_x / v.zoom) as i32;
        cy = (cy as f32 - v.right_click_off_y / v.zoom) as i32;
        let (ex, ey) = (cx + cw, cy + ch);
        [
            [cx, cy, cw, ch],
            [0, 0, cx, sh],
            [cx, 0, cw, cy],
            [ex, 0, sw - ex, sh],
            [cx, ey, cw, sh - ey],
        ]
    }

    /// `renderFogCircle()`: the `fogCircle` shader over the screen, its
    /// `u_mvp` an `ortho(0, screenW, screenH, 0)`.
    pub fn render_fog_circle(&self, v: &WeatherFxView, out: &mut Vec<DrawCmd>) {
        if self.fog_overlay_alpha <= 0.0 {
            return;
        }
        let rects = self.fog_circle_rects(v);
        let c = Rgba(1.0, 1.0, 1.0, self.fog_overlay_alpha);
        let mut dl = pz_render::DrawList::default();
        for (i, [x, y, w, h]) in rects.into_iter().enumerate() {
            let t = if i == 0 {
                &self.fog_circle
            } else {
                &self.fog_white
            };
            // SpriteRenderer.renderi skips a null texture's quad only
            // when it is not ready; a missing one draws untextured.
            dl.renderi(t.as_ref(), x, y, w, h, c);
        }
        out.push(DrawCmd::GlDisable(pz_render::gl::DEPTH_TEST));
        for cmd in dl.cmds {
            if let DrawCmd::Sprite(s) = cmd {
                out.push(DrawCmd::FogCircle {
                    sprite: s,
                    width: v.screen_width as f32,
                    height: v.screen_height as f32,
                });
            }
        }
    }
}

/// The quad `WeatherParticle.render` / `RainParticle.render` add at a
/// tile offset.
fn particle_quad(p: &WeatherParticle, ox: f32, oy: f32) -> ParticleQuad {
    let rgba = [p.color[0], p.color[1], p.color[2], p.render_alpha];
    if let ParticleKind::Rain { rp, .. } = &p.kind {
        let x = f64::from(ox + p.bounds[0] as f32);
        let y = f64::from(oy + p.bounds[1] as f32);
        let pt = rp.points;
        return ParticleQuad {
            x: std::array::from_fn(|i| (x + pt[i][0]) as f32),
            y: std::array::from_fn(|i| (y + pt[i][1]) as f32),
            rgba,
        };
    }
    let x = ox + p.bounds[0] as f32;
    let y = oy + p.bounds[1] as f32;
    let (x2, y2) = (x + p.o_width, y + p.o_height);
    ParticleQuad {
        x: [x, x2, x2, x],
        y: [y, y, y2, y2],
        rgba,
    }
}

/// `IsoWeatherFX.lerp(t, a, b)`.
pub fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(name: &str, id: u32, w: u32, h: u32) -> Option<Texture> {
        let mut t = Texture::empty(name);
        t.id = Some(id);
        t.width = w;
        t.height = h;
        t.ready = true;
        Some(t)
    }

    fn textures() -> WeatherFxTextures {
        WeatherFxTextures {
            clouds: std::array::from_fn(|i| tex("cloud", 10 + i as u32, 256, 128)),
            fog: std::array::from_fn(|i| tex("fog", 20 + i as u32, 256, 128)),
            snow: std::array::from_fn(|i| tex("snow", 30 + i as u32, 6, 6)),
            fog_circle: tex("fogcircle", 40, 64, 64),
            fog_white: tex("fogwhite", 41, 8, 8),
        }
    }

    #[test]
    fn stepped_float_moves_by_its_step_and_clamps() {
        let mut f = SteppedUpdateFloat::new(0.0, 0.25, 0.0, 1.0);
        f.set_target(3.0);
        assert_eq!(f.target(), 1.0);
        f.update(1.0);
        assert_eq!(f.value(), 0.25);
        f.update(10.0);
        assert_eq!(f.value(), 1.0);
        f.set_target(0.9);
        f.update(1.0);
        assert_eq!(f.value(), 0.9);
    }

    #[test]
    fn sandbox_caps_fog_and_rain() {
        let mut fx = IsoWeatherFx::new(&Rand::new(1), &textures());
        let c = WeatherFxClimate {
            precipitation_intensity: 1.0,
            fog_intensity: 1.0,
            ..Default::default()
        };
        fx.update_fx(
            &c,
            &WeatherFxSandbox {
                max_fog_intensity: 3,
                max_rain_fx_intensity: 3,
            },
        );
        assert_eq!(fx.fog_intensity.target(), 0.5);
        assert_eq!(fx.precipitation_intensity.target(), 0.05 + 0.95 * 0.5);
        fx.update_fx(
            &c,
            &WeatherFxSandbox {
                max_fog_intensity: 4,
                max_rain_fx_intensity: 1,
            },
        );
        assert_eq!(fx.fog_intensity.target(), 0.0);
        assert_eq!(fx.precipitation_intensity.target(), 1.0);
        fx.update_fx(&WeatherFxClimate::default(), &WeatherFxSandbox::default());
        assert_eq!(fx.precipitation_intensity.target(), 0.0);
    }

    #[test]
    fn rain_falls_and_fades_indoors() {
        let rand = Rand::new(5);
        let mut fx = IsoWeatherFx::new(&rand, &textures());
        let c = WeatherFxClimate {
            precipitation_intensity: 1.0,
            wind_precip_intensity: 0.5,
            precipitation_is_snow: false,
            ..Default::default()
        };
        fx.update_fx(&c, &WeatherFxSandbox::default());
        let mut f = WeatherFxFrame::default();
        for _ in 0..300 {
            fx.update(&rand, &f);
        }
        let rain = &fx.rects[Layer::Rain as usize];
        // 1024 at full strength, times the zoom 1 factor 1 - 0.5 * 0.375.
        assert_eq!(rain.particles_to_render, 832);
        assert_eq!(fx.rects[Layer::Snow as usize].particles_to_render, 0);
        assert!(fx.has_precipitation_to_render() && !fx.has_fog_to_render());
        // The wind tilts the rain past 90 degrees after a wind update.
        assert!(fx.wind_angle != 90.0 || fx.wind_angle_intensity.value() == 0.0);
        let lit = rain
            .particles
            .iter()
            .filter(|p| p.render_alpha > 0.0)
            .count();
        assert!(lit >= 800, "{lit}");
        // Indoors without a mask the layers fade out at 0.05 a tick.
        f.player_indoors = true;
        for _ in 0..25 {
            fx.update(&rand, &f);
        }
        assert_eq!(fx.indoors_alpha_mod.value(), 0.0);
        assert!(fx.rects[Layer::Rain as usize]
            .particles
            .iter()
            .all(|p| p.render_alpha == 0.0));
        // With the mask drawing, it stays at 1.
        f.player_has_mask_to_draw = true;
        for _ in 0..25 {
            fx.update(&rand, &f);
        }
        assert_eq!(fx.indoors_alpha_mod.value(), 1.0);
    }

    #[test]
    fn particles_wrap_into_their_rectangle() {
        assert_eq!(wrap(513.5, 512), 1.5);
        assert_eq!(wrap(1100.0, 512), 76.0);
        assert_eq!(wrap(-3.0, 512), 509.0);
        assert_eq!(wrap(512.0, 512), 512.0);
    }

    #[test]
    fn render_tiles_quads_by_texture_in_view_space() {
        let rand = Rand::new(9);
        let mut fx = IsoWeatherFx::new(&rand, &textures());
        let c = WeatherFxClimate {
            precipitation_intensity: 1.0,
            cloud_intensity: 1.0,
            fog_intensity: 1.0,
            precipitation_is_snow: true,
            ..Default::default()
        };
        fx.update_fx(&c, &WeatherFxSandbox::default());
        for _ in 0..400 {
            fx.update(&rand, &WeatherFxFrame::default());
        }
        let v = WeatherFxView::new(-100.0, 250.0, 1280, 720, 1.0);
        let mut out = Vec::new();
        fx.start_frame();
        fx.render(&v, &mut out);
        // Fog circle (5 quads), then particles as projected quads.
        let fog_quads = out
            .iter()
            .filter(|c| matches!(c, DrawCmd::FogCircle { .. }))
            .count();
        assert_eq!(fog_quads, 5);
        let projected: Vec<_> = out
            .iter()
            .filter_map(|c| {
                if let DrawCmd::ProjectedQuadF32 { pos, texture, .. } = c {
                    Some((pos, texture))
                } else {
                    None
                }
            })
            .collect();
        assert!(projected.len() > 100, "{}", projected.len());
        for (pos, _) in &projected {
            // Every quad overlaps the view.
            let minx = pos.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
            let maxx = pos.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
            let miny = pos.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            let maxy = pos.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
            assert!(maxx > -1.0 && minx < 1.0 && maxy > -1.0 && miny < 1.0);
        }
        // Each layer's quads come grouped by texture.
        let d = &fx.drawers[Layer::Snow as usize];
        assert!(d.textures.len() <= 3 && !d.is_empty());
        // Precipitation off in the options: no snow.
        let mut out2 = Vec::new();
        fx.start_frame();
        fx.render(
            &WeatherFxView {
                render_precipitation: 3,
                fog_quality: 0,
                ..v
            },
            &mut out2,
        );
        assert!(out2.len() < out.len());
        assert!(!out2.iter().any(|c| matches!(c, DrawCmd::FogCircle { .. })));
    }

    #[test]
    fn fog_circle_borders_cover_the_screen() {
        let mut fx = IsoWeatherFx::new(&Rand::new(2), &textures());
        fx.fog_intensity.override_current_value(1.0);
        let v = WeatherFxView::new(0.0, 0.0, 1920, 1080, 2.0);
        let r = fx.fog_circle_rects(&v);
        // (2048 - 512) / 2 = 768 by (1024 - 256) / 2 = 384, centred.
        assert_eq!(r[0], [576, 348, 768, 384]);
        assert_eq!(r[1], [0, 0, 576, 1080]);
        assert_eq!(r[2], [576, 0, 768, 348]);
        assert_eq!(r[3], [1344, 0, 576, 1080]);
        assert_eq!(r[4], [576, 732, 768, 348]);
    }
}
