//! `PZVehicleScript`: the vehicle definitions the game sends with
//! `Bullet.defineVehicleScript` (`VehicleScript.toBullet`) and
//! `Bullet.defineVehiclePhysicsMesh`.
//!
//! The scripts are a process-wide list in the original
//! (`PZVehicleScript::m_scripts`); they outlive worlds and are never
//! deleted. Vehicles read their script's values live, so redefining a
//! script changes existing vehicles too.

use crate::bt::linear_math::Vec3;

/// Wheel slots in the script (`m_wheels[8]`).
pub const MAX_WHEELS: usize = 8;
/// Shape slots in the script (`m_shapes[10]`).
pub const MAX_SHAPES: usize = 10;

/// One wheel of a script.
#[derive(Clone, Debug)]
pub struct ScriptWheel {
    /// Chassis connection point (`+0x00`).
    pub offset: Vec3,
    /// Wheel radius (`+0x20`).
    pub radius: f64,
    /// Front wheel (`+0x28`): steered, not driven.
    pub front: bool,
}

/// One collision shape of a script.
#[derive(Clone, Debug, Default)]
pub struct ScriptShape {
    /// 1 box, 2 sphere, 3 convex hull (mesh).
    pub kind: i32,
    pub offset: Vec3,
    /// Box rotation in degrees (x, y, z).
    pub rotate: Vec3,
    /// Box full extents.
    pub extents: Vec3,
    /// Sphere radius.
    pub radius: f64,
    /// Hull points (x, y, z triples), appended by every
    /// `defineVehiclePhysicsMesh`.
    pub points: Vec<f64>,
}

/// `PZVehicleScript`.
#[derive(Clone, Debug)]
pub struct VehicleScript {
    pub name: String,
    pub model_scale: f64,
    pub mass: f64,
    pub roll_influence: f64,
    pub suspension_stiffness: f64,
    pub suspension_compression: f64,
    pub suspension_damping: f64,
    pub max_suspension_travel_cm: f64,
    pub suspension_rest_length: f64,
    pub wheel_friction: f64,
    pub stopping_movement_force: f64,
    pub num_shapes: i32,
    pub shapes: Vec<ScriptShape>,
    pub num_wheels: i32,
    pub wheels: Vec<ScriptWheel>,
}

/// `(int)(v + (v >= 0 ? 0.5f : -0.5f))` in float.
fn round_f(v: f32) -> i32 {
    let h = if 0.0 <= v { 0.5f32 } else { -0.5f32 };
    (v + h) as i32
}

impl VehicleScript {
    /// The values a freshly allocated script starts with.
    pub fn new_default() -> VehicleScript {
        let wheel = |x: f64, z: f64, front: bool| ScriptWheel {
            offset: Vec3::new(x, -0.180_000_007_152_557_37, z),
            radius: 0.300_000_011_920_928_96,
            front,
        };
        let mut wheels = vec![
            wheel(0.25, 0.629_999_995_231_628_4, true),
            wheel(-0.25, 0.629_999_995_231_628_4, true),
            wheel(0.25, -0.589_999_973_773_956_3, false),
            wheel(-0.25, -0.589_999_973_773_956_3, false),
        ];
        while wheels.len() < MAX_WHEELS {
            wheels.push(ScriptWheel {
                offset: Vec3::ZERO,
                radius: 0.0,
                front: false,
            });
        }
        VehicleScript {
            name: String::new(),
            model_scale: 2.150_000_095_367_431_6,
            mass: 800.0,
            roll_influence: 1.0,
            suspension_stiffness: 20.0,
            suspension_compression: 1.830_000_042_915_344_2,
            suspension_damping: 1.879_999_995_231_628_4,
            max_suspension_travel_cm: 100.0,
            suspension_rest_length: 1.0,
            wheel_friction: 2.0,
            stopping_movement_force: 1.0,
            num_shapes: 0,
            shapes: vec![ScriptShape::default(); MAX_SHAPES],
            num_wheels: 4,
            wheels,
        }
    }

    /// `PZVehicleScript::fromJava(params, count)`: `false` when the array
    /// ends early (the fields read so far stay changed).
    pub fn from_java(&mut self, p: &[f32]) -> bool {
        let n = p.len();
        let mut i = 0usize;
        macro_rules! next_f {
            () => {{
                if i >= n {
                    return false;
                }
                i += 1;
                p[i - 1]
            }};
        }
        macro_rules! next_v {
            () => {{
                if i + 3 > n {
                    return false;
                }
                let v = Vec3::new(p[i] as f64, p[i + 1] as f64, p[i + 2] as f64);
                i += 3;
                v
            }};
        }
        self.model_scale = next_f!() as f64;
        self.mass = next_f!() as f64;
        self.roll_influence = next_f!() as f64;
        self.suspension_stiffness = next_f!() as f64;
        self.suspension_compression = next_f!() as f64;
        self.suspension_damping = next_f!() as f64;
        self.max_suspension_travel_cm = next_f!() as f64;
        self.suspension_rest_length = next_f!() as f64;
        self.wheel_friction = next_f!() as f64;
        self.stopping_movement_force = next_f!() as f64;
        self.num_wheels = round_f(next_f!());
        for w in 0..self.num_wheels.max(0) as usize {
            let front = next_f!() != 0.0;
            let wheel = &mut self.wheels[w];
            wheel.front = front;
            wheel.offset = next_v!();
            let r = next_f!();
            self.wheels[w].radius = r as f64;
        }
        self.num_shapes = round_f(next_f!());
        for s in 0..self.num_shapes.max(0) as usize {
            let kind = round_f(next_f!());
            self.shapes[s].kind = kind;
            self.shapes[s].offset = next_v!();
            if kind == 1 {
                self.shapes[s].extents = next_v!();
                self.shapes[s].rotate = next_v!();
            }
            if kind == 2 {
                self.shapes[s].radius = next_f!() as f64;
            }
        }
        true
    }

    /// `PZVehicleScript::definePhysicsMesh(shapeIndex, numPoints, points)`:
    /// makes the shape a hull and appends the points.
    pub fn define_physics_mesh(&mut self, shape_index: i32, num_points: i32, points: &[f32]) {
        if shape_index < 0 || shape_index >= self.num_shapes {
            return;
        }
        let s = &mut self.shapes[shape_index as usize];
        s.kind = 3;
        if num_points > 0 {
            for &f in &points[..(num_points as usize * 3)] {
                s.points.push(f as f64);
            }
        }
    }
}
