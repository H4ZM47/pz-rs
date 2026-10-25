//! `ParticlesFire` (with `Particles` and `ParticlesArray`): the GPU
//! fire-and-smoke particle system drawn by `ModelManager.RenderParticles`
//! with the `fire` and `smoke` shaders (`FireShader`, `SmokeShader`).
//!
//! Particles are spawned on the CPU into emitter zones (`ParticlesProcess`)
//! and uploaded as one `vec4` each (x, y, id, slot / size); the vertex
//! shaders animate them from `FireTime` and the wind in `FireParam`.
//!
//! In build 42 nothing calls `ParticlesFire.render()` (its zones are a fixed
//! demo layout around the screen centre at creation); fires in the world
//! are `IsoFire` sprites ([`crate::fire`]). This port keeps the system
//! available with the original's update order and uniforms.

use pz_render::{gl, DrawCmd, ParticleProgram, ParticlesDraw, Texture};

use crate::fire::RandSource;

/// `ParticlesArray`: a list with holes that reuses free slots and is
/// compacted before upload.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticlesArray<T> {
    pub items: Vec<Option<T>>,
    need_to_update: bool,
    count: usize,
    last: usize,
}

impl<T> Default for ParticlesArray<T> {
    fn default() -> Self {
        ParticlesArray {
            items: Vec::new(),
            need_to_update: true,
            count: 0,
            last: 0,
        }
    }
}

impl<T> ParticlesArray<T> {
    /// `addParticle(p)`: appended when there are no holes, else into the
    /// first hole from the last one filled (wrapping).
    pub fn add(&mut self, p: T) -> Option<usize> {
        if self.items.len() == self.count {
            self.items.push(Some(p));
            self.count += 1;
            self.need_to_update = true;
            return Some(self.items.len() - 1);
        }
        let hole = (self.last..self.items.len())
            .chain(0..self.last)
            .find(|&i| self.items[i].is_none())?;
        self.last = hole;
        self.items[hole] = Some(p);
        self.count += 1;
        self.need_to_update = true;
        Some(hole)
    }

    /// `deleteParticle(k)`.
    pub fn delete(&mut self, k: i32) -> bool {
        if k >= 0 && (k as usize) < self.items.len() && self.items[k as usize].is_some() {
            self.items[k as usize] = None;
            self.count -= 1;
            self.need_to_update = true;
            return true;
        }
        false
    }

    /// `defragmentParticle()`, moves and all.
    pub fn defragment(&mut self) {
        self.need_to_update = false;
        let size = self.items.len();
        if self.count == size || size == 0 {
            return;
        }
        let mut hole = self.items.iter().position(Option::is_none).expect("a hole");
        for i in (0..size).rev() {
            if self.items[i].is_none() {
                continue;
            }
            let p = self.items[i].take();
            self.items[hole] = p;
            if let Some(k) = (hole..size).find(|&k| self.items[k].is_none()) {
                hole = k;
            }
            if hole + 1 < i {
                continue;
            }
            self.last = hole;
            break;
        }
    }

    /// `getCount()`.
    pub fn count(&self) -> usize {
        self.count
    }

    /// `getNeedToUpdate()`.
    pub fn need_to_update(&self) -> bool {
        self.need_to_update
    }
}

/// `ParticlesFire.ZoneType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneType {
    Rectangle,
    Circle,
    Line,
}

/// `ParticlesFire.Zone`: an emitter keeping `intensity` particles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    pub kind: ZoneType,
    pub intensity: i32,
    pub current_particles: i32,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub r: f32,
}

impl Zone {
    /// The circle constructor.
    pub fn circle(intensity: i32, x: f32, y: f32, r: f32) -> Self {
        Zone {
            kind: ZoneType::Circle,
            intensity,
            current_particles: 0,
            x0: x,
            y0: y,
            x1: 0.0,
            y1: 0.0,
            r,
        }
    }

    /// The rectangle constructor (corners sorted).
    pub fn rectangle(intensity: i32, x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Zone {
            kind: ZoneType::Rectangle,
            intensity,
            current_particles: 0,
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
            r: 0.0,
        }
    }

    /// The line constructor (ends ordered by x).
    pub fn line(intensity: i32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Self {
        let (x0, y0, x1, y1) = if x0 < x1 {
            (x0, y0, x1, y1)
        } else {
            (x1, y1, x0, y0)
        };
        Zone {
            kind: ZoneType::Line,
            intensity,
            current_particles: 0,
            x0,
            y0,
            x1,
            y1,
            r,
        }
    }
}

/// `ParticlesFire.Particle`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub id: f32,
    pub x: f32,
    pub y: f32,
    pub t_shift: f32,
    pub vx: f32,
    pub vy: f32,
    /// Index of its zone.
    pub zone: usize,
}

/// The `IsoCamera.frameState` values the system reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticlesCamera {
    pub off_x: f32,
    pub off_y: f32,
    pub offscreen_width: i32,
    pub offscreen_height: i32,
}

/// `Particles.orthogonal(left, right, bottom, top, near, far)`, column
/// major (`m[col][row]`).
pub fn orthogonal(
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    near: f32,
    far: f32,
) -> [[f32; 4]; 4] {
    let mut m = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    m[0][0] = 2.0 / (right - left);
    m[1][1] = 2.0 / (top - bottom);
    m[2][2] = -2.0 / (far - near);
    m[3][2] = (-far - near) / (far - near);
    m[3][0] = (-right - left) / (right - left);
    m[3][1] = (-top - bottom) / (top - bottom);
    m
}

/// `ParticlesFire` with its `Particles` base.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticlesFire {
    pub particles: ParticlesArray<Particle>,
    pub zones: Vec<Zone>,
    /// `particuleData`: what the last upload held.
    pub data: Vec<[f32; 4]>,
    pub wind_x: f32,
    pub wind_y: f32,
    /// `particlesTime` (`FireTime`).
    pub time: f32,
    /// `mvpMatrix`, column major.
    pub mvp: [[f32; 4]; 4],
}

/// `ClimateManager.getWindAngleIntensity()` and `getWindIntensity()`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wind {
    pub angle_intensity: f32,
    pub intensity: f32,
}

/// `fastfloor`.
fn fastfloor(x: f32) -> f32 {
    pz_render::iso::fastfloor(x) as f32
}

impl ParticlesFire {
    /// The constructor: five demo zones around the screen centre.
    pub fn new(cam: &ParticlesCamera) -> Self {
        let cx = fastfloor(cam.off_x + (cam.offscreen_width / 2) as f32);
        let cy = fastfloor(cam.off_y + (cam.offscreen_height / 2) as f32);
        let zones = vec![
            Zone::rectangle(10, cx - 30.0, cy - 10.0, cx + 30.0, cy + 10.0),
            Zone::circle(10, cx - 200.0, cy, 50.0),
            Zone::circle(40, cx + 200.0, cy, 100.0),
            Zone::line(60, cx - 150.0, cy - 300.0, cx + 250.0, cy - 300.0, 10.0),
            Zone::line(10, cx - 350.0, cy - 200.0, cx - 350.0, cy - 300.0, 10.0),
        ];
        ParticlesFire {
            particles: ParticlesArray::default(),
            zones,
            data: Vec::new(),
            wind_x: 0.0,
            wind_y: 0.0,
            time: 0.0,
            mvp: [[0.0; 4]; 4],
        }
    }

    /// `ParticlesProcess()`: each zone tops itself up by a tenth of what it
    /// lacks, or deletes random slots when over.
    pub fn process(&mut self, rand: &dyn RandSource) {
        for k in 0..self.zones.len() {
            let z = self.zones[k];
            let new_particles = ((z.intensity - z.current_particles) as f32 * 0.1).ceil() as i32;
            for _ in 0..new_particles.max(0) {
                let (x, y) = match z.kind {
                    ZoneType::Rectangle => {
                        let x = rand.next_float(z.x0, z.x1);
                        (x, rand.next_float(z.y0, z.y1))
                    }
                    ZoneType::Circle => {
                        let a = rand.next_float(0.0, std::f32::consts::PI * 2.0);
                        let r = rand.next_float(0.0, z.r);
                        (
                            (z.x0 as f64 + r as f64 * (a as f64).cos()) as f32,
                            (z.y0 as f64 + r as f64 * (a as f64).sin()) as f32,
                        )
                    }
                    ZoneType::Line => {
                        let a = rand.next_float(0.0, std::f32::consts::PI * 2.0);
                        let r = rand.next_float(0.0, z.r);
                        let h = rand.next_float(0.0, 1.0);
                        let bx = z.x0 * h + z.x1 * (1.0 - h);
                        let by = z.y0 * h + z.y1 * (1.0 - h);
                        (
                            (bx as f64 + r as f64 * (a as f64).cos()) as f32,
                            (by as f64 + r as f64 * (a as f64).sin()) as f32,
                        )
                    }
                };
                let vx = rand.next_float(-3.0, 3.0);
                let vy = rand.next_float(1.0, 5.0);
                let id = rand.next_float(-1_000_000.0, 1_000_000.0);
                self.zones[k].current_particles += 1;
                self.particles.add(Particle {
                    id,
                    x,
                    y,
                    t_shift: 0.0,
                    vx,
                    vy,
                    zone: k,
                });
            }
            if new_particles >= 0 {
                continue;
            }
            for _ in 0..-new_particles {
                self.zones[k].current_particles -= 1;
                let i = rand.next_range(0, self.particles.count() as i32 + 1);
                self.particles.delete(i);
            }
        }
    }

    /// `getParametersFire()`: the twelve floats uploaded (transposed) as
    /// the `FireParam` mat3.
    pub fn parameters_fire(&self) -> [f32; 12] {
        let mut b = [0.0; 12];
        b[0] = self.wind_x;
        b[3] = self.wind_y;
        b
    }

    /// `FireParam` as the shader sees it (`FireParam[col][row]`): the
    /// buffer's first nine floats, row major.
    pub fn fire_param(&self) -> [[f32; 3]; 3] {
        let b = self.parameters_fire();
        std::array::from_fn(|c| std::array::from_fn(|r| b[r * 3 + c]))
    }

    /// `updateParticleParams()` without the GL calls: the wind, a
    /// `ParticlesProcess`, and the data rebuilt if the particles changed.
    pub fn update_particle_params(&mut self, wind: Wind, rand: &dyn RandSource) {
        self.wind_x = ((wind.angle_intensity * 6.0) as f64).sin() as f32 * wind.intensity;
        self.wind_y = ((wind.angle_intensity * 6.0) as f64).cos() as f32 * wind.intensity;
        self.process(rand);
        if self.particles.need_to_update() {
            self.particles.defragment();
            let size = self.particles.items.len() as f32;
            self.data = self
                .particles
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.map(|p| [p.x, p.y, p.id, i as f32 / size]))
                .collect();
        }
    }

    /// `Particles.render()`'s state step: the time (`multiplier` is
    /// `GameTime.getMultiplier()`) and the projection over the offscreen
    /// buffer.
    pub fn advance(&mut self, cam: &ParticlesCamera, multiplier: f32) {
        self.time += 0.0166 * multiplier;
        self.mvp = orthogonal(
            cam.off_x,
            cam.off_x + cam.offscreen_width as f32,
            cam.off_y + cam.offscreen_height as f32,
            cam.off_y,
            -1.0,
            1.0,
        );
    }

    /// `Particles.render()` and `ModelManager.RenderParticles`: the flames
    /// (additive, `FireFlame.png`) then the smoke (alpha blended,
    /// `FireSmokes.png`). Each pass sets its uniforms before
    /// `updateParticleParams` runs, so it draws with the previous wind.
    pub fn render(
        &mut self,
        cam: &ParticlesCamera,
        multiplier: f32,
        wind: Wind,
        flame: Option<Texture>,
        smoke: Option<Texture>,
        rand: &dyn RandSource,
    ) -> Vec<DrawCmd> {
        self.advance(cam, multiplier);
        let mut out = Vec::new();
        for (program, texture, blend) in [
            (ParticleProgram::Fire, flame, gl::ONE),
            (ParticleProgram::Smoke, smoke, gl::ONE_MINUS_SRC_ALPHA),
        ] {
            let param = self.fire_param();
            self.update_particle_params(wind, rand);
            let n = self.particles.count().min(self.data.len());
            out.push(DrawCmd::Particles(Box::new(ParticlesDraw {
                program,
                mvp: self.mvp,
                time: self.time,
                param,
                texture,
                particles: self.data[..n].to_vec(),
                blend_func: [gl::SRC_ALPHA, blend],
                viewport: (
                    cam.offscreen_width.max(0) as u32,
                    cam.offscreen_height.max(0) as u32,
                ),
            })));
        }
        out
    }
}

#[cfg(test)]
mod tests;
