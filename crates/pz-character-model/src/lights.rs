//! `ModelInstance.updateLights` / `PlayerData` / `FrameLightInfo` /
//! `EffectLight`: the five model lights a character gets each frame, faded
//! in and out as the square's result lights (`IsoGridSquare.ResultLight`,
//! from LightingJNI; pz-lighting decodes them) come and go, plus the
//! ambient colour and the two fill lights.
//!
//! [`EffectLight`]s become the shader's `LightDirection`/`LightColour` via
//! [`pz_iso_render::models::encode_light`] (`Shader.setLightInst`).

use pz_render::model::ModelLight;

/// `IsoGridSquare.ResultLight`, as `LightingJNI.JNILighting` decodes it
/// from `getSquareLighting` (pz-lighting's
/// [`pz_lighting::jni::SquareLight::lights`]).
pub use pz_lighting::jni::ResultLightInfo as ResultLight;

pub const RLF_ROOMLIGHT: i32 = 1;
pub const RLF_TORCH: i32 = 2;

/// `ModelInstance.EffectLight`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectLight {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub radius: i32,
}

impl EffectLight {
    pub fn set(&mut self, x: f32, y: f32, z: f32, r: f32, g: f32, b: f32, radius: i32) {
        *self = EffectLight {
            x,
            y,
            z,
            r,
            g,
            b,
            radius,
        };
    }

    pub fn clear(&mut self) {
        *self = EffectLight::default();
    }

    /// `Shader.setLightInst` for a model at `obj` drawn at `angle`
    /// (`animPlayer.getRenderedAngle()`; `None` for direction lights).
    pub fn to_model_light(&self, obj: [f32; 3], angle: Option<f32>) -> ModelLight {
        pz_iso_render::models::encode_light(
            [self.x, self.y, self.z],
            [self.r, self.g, self.b],
            self.radius as f32,
            obj,
            angle,
        )
    }
}

/// `FrameLightBlendStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FrameLightBlendStatus {
    In,
    During,
    Out,
}

/// `FrameLightInfo`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLightInfo {
    pub stage: FrameLightBlendStatus,
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub dist_sq: f32,
    pub radius: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub flags: i32,
    pub current_color: [f32; 3],
    pub target_color: [f32; 3],
    pub active: bool,
    pub found_this_frame: bool,
}

impl Default for FrameLightInfo {
    fn default() -> Self {
        FrameLightInfo {
            stage: FrameLightBlendStatus::In,
            id: 0,
            x: 0,
            y: 0,
            z: 0,
            dist_sq: 0.0,
            radius: 0,
            r: 0.0,
            g: 0.0,
            b: 0.0,
            flags: 0,
            current_color: [0.0; 3],
            target_color: [0.0; 3],
            active: false,
            found_this_frame: false,
        }
    }
}

/// What `updateLights` reads from the character and its square.
#[derive(Clone, Debug)]
pub struct LightContext {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `lighting.getResultLight(i)` of the current square (all of them;
    /// the first four are used).
    pub result_lights: Vec<ResultLight>,
    /// `interpolateLight` of the square at the character's fraction,
    /// already blended with the square above when `z` is more than 0.2
    /// above the floor.
    pub target_ambient: [f32; 3],
    /// `square.isOutside()`.
    pub outside: bool,
    /// `square.getRoom() != null`.
    pub in_room: bool,
    /// `room.findRoomLightByID(id) != null` for the current room.
    pub room_light_ids: Vec<i32>,
    /// `Some(playerIndex)` for an `IsoPlayer`.
    pub player_index: Option<i32>,
    /// `getTorchStrength()` (players only).
    pub torch_strength: f32,
    pub forward: [f32; 2],
    /// `GameTime.getMultiplier()`.
    pub multiplier: f32,
}

/// `ModelInstance.PlayerData`: one camera's view of a character's lights.
#[derive(Clone, Debug, Default)]
pub struct PlayerLightData {
    pub frame_lights: [Option<FrameLightInfo>; 5],
    chosen: Vec<ResultLight>,
    pub target_ambient: [f32; 3],
    pub current_ambient: [f32; 3],
    pub effect_lights_main: [EffectLight; 5],
    initialised: bool,
}

fn step(current: f32, target: f32, add: f32) -> f32 {
    let clamp = |min: f32, max: f32, v: f32| v.min(max).max(min);
    if current < target {
        clamp(0.0, target, current + add)
    } else if current > target {
        clamp(target, 1.0, current + add)
    } else {
        current
    }
}

fn signum(v: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}

impl PlayerLightData {
    /// `ModelInstance.updateLights` (a first call is the original's
    /// `bNeverDone`).
    pub fn update(&mut self, ctx: &LightContext) {
        let never_done = !self.initialised;
        self.initialised = true;
        // initFrameLightsForFrame
        for e in &mut self.effect_lights_main {
            e.radius = -1;
        }
        self.chosen.clear();
        let count = ctx.result_lights.len().min(4);
        self.chosen.extend_from_slice(&ctx.result_lights[..count]);
        if never_done {
            for f in self.frame_lights.iter_mut().flatten() {
                f.active = false;
            }
        }
        self.complete_frame_lights(ctx.multiplier);
        self.target_ambient = ctx.target_ambient;
        let m = ctx.multiplier;
        for i in 0..3 {
            let (c, t) = (self.current_ambient[i], self.target_ambient[i]);
            self.current_ambient[i] = step(c, t, (t - c) / (10.0 * m));
        }
        if never_done {
            for f in self.frame_lights.iter_mut().flatten() {
                f.current_color = f.target_color;
            }
            self.current_ambient = self.target_ambient;
        }
        self.sort_lights(ctx);
        for i in 0..5 {
            let Some(light) = self.frame_lights[i] else {
                continue;
            };
            if !light.active {
                continue;
            }
            let e = &mut self.effect_lights_main[i];
            let mut flags = light.flags;
            if flags & RLF_ROOMLIGHT != 0
                && !(ctx.in_room && ctx.room_light_ids.contains(&light.id))
            {
                flags &= !RLF_ROOMLIGHT;
            }
            let c = light.current_color;
            if flags & RLF_ROOMLIGHT != 0 {
                e.set(
                    ctx.x,
                    ctx.y,
                    (crate::fastfloor(ctx.z) + 1) as f32,
                    c[0] * 0.7,
                    c[1] * 0.7,
                    c[2] * 0.7,
                    light.radius,
                );
                continue;
            }
            if light.flags & RLF_TORCH != 0 {
                if let Some(pi) = ctx.player_index {
                    let min_id = pi * 4 + 1;
                    let max_id = pi * 4 + 4;
                    if light.id >= min_id && light.id <= max_id {
                        continue;
                    }
                    e.set(
                        light.x as f32,
                        light.y as f32,
                        light.z as f32,
                        c[0],
                        c[1],
                        c[2],
                        light.radius,
                    );
                    continue;
                }
                e.set(
                    light.x as f32,
                    light.y as f32,
                    light.z as f32,
                    c[0] * 2.0,
                    c[1],
                    c[2],
                    light.radius,
                );
                continue;
            }
            e.set(
                light.x as f32 + 0.5,
                light.y as f32 + 0.5,
                light.z as f32 + 0.5,
                c[0] * 0.7,
                c[1] * 0.7,
                c[2] * 0.7,
                light.radius,
            );
        }
        if count <= 3 && ctx.player_index.is_some() && ctx.torch_strength > 0.0 {
            self.effect_lights_main[2].set(
                ctx.x + ctx.forward[0] * 0.5,
                ctx.y + ctx.forward[1] * 0.5,
                ctx.z + 0.25,
                1.0,
                1.0,
                1.0,
                2,
            );
        }
        let lerp = |f: f32, t: f32, d: f32| f + (t - f) * d;
        let mut ar = lerp(0.0, 1.0, self.current_ambient[0]);
        let mut ag = lerp(0.0, 1.0, self.current_ambient[1]);
        let mut ab = lerp(0.0, 1.0, self.current_ambient[2]);
        if ctx.outside || ctx.in_room {
            ar *= 1.7;
            ag *= 1.7;
            ab *= 1.7;
            self.effect_lights_main[3].set(
                ctx.x - 2.0,
                ctx.y - 2.0,
                ctx.z + 1.0,
                ar / 4.0,
                ag / 4.0,
                ab / 4.0,
                5000,
            );
            self.effect_lights_main[4].set(
                ctx.x + 2.0,
                ctx.y + 2.0,
                ctx.z + 1.0,
                ar / 4.0,
                ag / 4.0,
                ab / 4.0,
                5000,
            );
        }
    }

    /// One frame of light fading on its own (`initFrameLightsForFrame`,
    /// `registerFrameLight` for each of `chosen`,
    /// `completeFrameLightsForFrame`).
    pub fn fade_frame_lights(&mut self, chosen: &[ResultLight], multiplier: f32) {
        for e in &mut self.effect_lights_main {
            e.radius = -1;
        }
        self.chosen.clear();
        self.chosen.extend_from_slice(chosen);
        self.complete_frame_lights(multiplier);
    }

    fn complete_frame_lights(&mut self, multiplier: f32) {
        for f in self.frame_lights.iter_mut().flatten() {
            f.found_this_frame = false;
        }
        'chosen: for cl in &self.chosen {
            let found = (0..5).find(|&x| {
                let Some(f) = &self.frame_lights[x] else {
                    return false;
                };
                f.active
                    && if cl.id != -1 {
                        cl.id == f.id
                    } else {
                        f.x == cl.x && f.y == cl.y && f.z == cl.z
                    }
            });
            if let Some(x) = found {
                let f = self.frame_lights[x].as_mut().unwrap();
                f.found_this_frame = true;
                f.x = cl.x;
                f.y = cl.y;
                f.z = cl.z;
                f.flags = cl.flags;
                f.radius = cl.radius;
                f.target_color = [cl.r, cl.g, cl.b];
                f.stage = FrameLightBlendStatus::In;
                continue;
            }
            for x in 0..5 {
                if self.frame_lights[x].is_some_and(|f| f.active) {
                    continue;
                }
                let f = self.frame_lights[x].get_or_insert_with(FrameLightInfo::default);
                f.x = cl.x;
                f.y = cl.y;
                f.z = cl.z;
                f.r = cl.r;
                f.g = cl.g;
                f.b = cl.b;
                f.flags = cl.flags;
                f.radius = cl.radius;
                f.id = cl.id;
                f.current_color = [0.0; 3];
                f.target_color = [cl.r, cl.g, cl.b];
                f.stage = FrameLightBlendStatus::In;
                f.active = true;
                f.found_this_frame = true;
                continue 'chosen;
            }
        }
        for f in self.frame_lights.iter_mut().flatten() {
            if !f.active {
                continue;
            }
            if !f.found_this_frame {
                f.target_color = [0.0; 3];
                f.stage = FrameLightBlendStatus::Out;
            }
            for i in 0..3 {
                let (c, t) = (f.current_color[i], f.target_color[i]);
                f.current_color[i] = step(c, t, signum(t - c) / (60.0 * multiplier));
            }
            if f.stage == FrameLightBlendStatus::Out && f.current_color.iter().all(|&c| c < 0.01) {
                f.active = false;
            }
        }
    }

    fn sort_lights(&mut self, ctx: &LightContext) {
        for f in self.frame_lights.iter_mut().flatten() {
            f.dist_sq = if !f.active {
                f32::MAX
            } else {
                let (dx, dy, dz) = (
                    ctx.x - (f.x as f32 + 0.5),
                    ctx.y - (f.y as f32 + 0.5),
                    ctx.z - f.z as f32,
                );
                dx * dx + dy * dy + dz * dz
            };
        }
        let none = |f: &Option<FrameLightInfo>| f.is_none_or(|f| f.radius == -1 || !f.active);
        self.frame_lights.sort_by(|a, b| {
            use std::cmp::Ordering::*;
            match (none(a), none(b)) {
                (true, true) => Equal,
                (true, false) => Greater,
                (false, true) => Less,
                _ => {
                    let (a, b) = (a.unwrap(), b.unwrap());
                    a.stage
                        .cmp(&b.stage)
                        .then_with(|| match signum(a.dist_sq - b.dist_sq) as i32 {
                            -1 => Less,
                            1 => Greater,
                            _ => Equal,
                        })
                }
            }
        });
    }
}
