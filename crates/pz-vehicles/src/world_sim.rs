//! `zombie.core.physics.WorldSimulation`: steps the physics at a fixed
//! 0.01 s and copies the vehicles' transforms, velocities, speeds and
//! wheel states back from `Bullet.getVehiclePhysics`.

use crate::env;
use crate::jmath::{Matrix3f, Quaternionf, Transform, Vector3f};
use crate::physics::{Bullet, WHEEL_PARAMS};
use std::cell::{Cell, RefCell};

/// `WorldSimulation` state.
#[derive(Clone, Debug, Default)]
pub struct WorldSimulation {
    pub created: bool,
    pub time: i64,
    pub local_time: f32,
    pub period_sec: f32,
    pub bullet_frame_no: i32,
}

thread_local! {
    static SIM: RefCell<WorldSimulation> = RefCell::new(WorldSimulation { bullet_frame_no: -1, ..Default::default() });
    static WHEEL_PARAMS_BUF: RefCell<[f32; WHEEL_PARAMS]> = const { RefCell::new([0.0; WHEEL_PARAMS]) };
    static AIR_TEMP: Cell<f32> = const { Cell::new(20.0) };
    static META_BOUNDS: Cell<(i32, i32, i32, i32)> = const { Cell::new((0, 0, 0, 0)) };
    static CHUNK_MAP: Cell<(i32, i32, i32)> = const { Cell::new((0, 0, 13)) };
}

/// The world bounds `create()` passes to `Bullet.initWorld`
/// (`metaGrid` min/max cells) and the player chunk map
/// (`getWorldXMin`, `getWorldYMin`, `chunkGridWidth`).
pub fn set_world_bounds(
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    chunk_map: (i32, i32, i32),
) {
    META_BOUNDS.with(|c| c.set((min_x, min_y, max_x, max_y)));
    CHUNK_MAP.with(|c| c.set(chunk_map));
}

/// `ClimateManager.getAirTemperatureForSquare` as the engine start reads it.
pub fn set_air_temperature(t: f32) {
    AIR_TEMP.with(|c| c.set(t));
}

pub fn air_temperature() -> f32 {
    AIR_TEMP.with(|c| c.get())
}

/// `WorldSimulation.instance.created`.
pub fn created() -> bool {
    SIM.with(|s| s.borrow().created)
}

/// Marks the simulation created with the given offsets, without calling
/// `initWorld` (for a physics world the caller set up).
pub fn set_created(offset_x: f32, offset_y: f32) {
    env::set_offset(offset_x, offset_y);
    SIM.with(|s| s.borrow_mut().created = true);
}

/// Resets the simulation (a new world).
pub fn reset() {
    SIM.with(|s| {
        *s.borrow_mut() = WorldSimulation {
            bullet_frame_no: -1,
            ..Default::default()
        }
    });
    WHEEL_PARAMS_BUF.with(|w| *w.borrow_mut() = [0.0; WHEEL_PARAMS]);
}

/// A copy of the state.
pub fn state() -> WorldSimulation {
    SIM.with(|s| s.borrow().clone())
}

/// `BaseVehicle.wheelParams` (static, shared by all vehicles).
pub(crate) fn with_wheel_params<R>(f: impl FnOnce(&mut [f32; WHEEL_PARAMS]) -> R) -> R {
    WHEEL_PARAMS_BUF.with(|w| f(&mut w.borrow_mut()))
}

/// `create()`.
pub fn create() {
    if created() {
        return;
    }
    let (min_x, min_y, max_x, max_y) = META_BOUNDS.with(|c| c.get());
    let ox = (min_x * 256) as f32;
    let oy = (min_y * 256) as f32;
    env::set_offset(ox, oy);
    let (wx, wy, w) = CHUNK_MAP.with(|c| c.get());
    Bullet::call(|b| {
        b.init_world(min_x, min_y, max_x, max_y, ox as i32, oy as i32, false)?;
        b.activate_chunk_map(0, wx, wy, w)
    });
    SIM.with(|s| {
        let mut s = s.borrow_mut();
        s.time = env::time().server_time_mills;
        s.created = true;
    });
}

/// `destroy()`.
pub fn destroy() {
    Bullet::with(|b| b.destroy_world());
}

/// What the natives call back during `stepSimulation`.
struct StepCallbacks;

impl crate::physics::JavaCallbacks for StepCallbacks {
    /// `Bullet.updatePhysicsForLevelIfNeeded(wx, wy, level)`.
    fn update_physics_for_level_if_needed(
        &mut self,
        wx: i32,
        wy: i32,
        level: i32,
        to_bullet: &mut Vec<Vec<u8>>,
    ) -> bool {
        env::world().update_physics_for_level_if_needed(wx, wy, level, to_bullet)
    }
    /// `Bullet.onVehicleConstraintImpulse(constraintID, vidA, vidB, impulse)`.
    fn on_vehicle_constraint_impulse(&mut self, c: i32, a: i32, b: i32, impulse: f32) {
        on_vehicle_constraint_impulse(c, a, b, impulse);
    }
}

/// `Bullet.onVehicleConstraintImpulse` (single player): a towing
/// constraint pulled harder than 1500 (both upright) or 500 breaks, unless
/// it changed in the last 2 s.
pub fn on_vehicle_constraint_impulse(constraint_id: i32, vid_a: i32, vid_b: i32, impulse: f32) {
    let (Some(va), Some(vb)) = (
        crate::manager::vehicle_by_id(vid_a as i16),
        crate::manager::vehicle_by_id(vid_b as i16),
    ) else {
        return;
    };
    let break_impulse = if va.up_vector_dot() > 0.8 && vb.up_vector_dot() > 0.8 {
        1500.0
    } else {
        500.0
    };
    if impulse < break_impulse {
        return;
    }
    let recently = va.st().constraint_changed_time + 2000 > env::now_ms();
    if va.st().constraint_towing != constraint_id || recently {
        return;
    }
    va.break_constraint(true, false);
    env::events().play_sound("BreakMetalItem");
}

/// `updatePhysic()`.
fn update_physic() {
    let fixed = 0.01f32;
    let mut n = 0;
    let lt = SIM.with(|s| {
        let mut s = s.borrow_mut();
        s.local_time += env::time().physics_seconds_since_last_update;
        s.local_time
    });
    if lt >= fixed {
        n = (lt / fixed) as i32;
        SIM.with(|s| s.borrow_mut().local_time -= n as f32 * fixed);
        for _ in 0..n {
            for v in crate::manager::cell_vehicles() {
                v.apply_accumulated_impulses_from_hit_objects();
                v.apply_all_impulses_from_prone_characters();
                v.check_surrounding_chunks();
            }
            Bullet::step_simulation(fixed, 0, 0.0, &mut StepCallbacks);
        }
        SIM.with(|s| {
            let mut s = s.borrow_mut();
            s.period_sec = n as f32 * fixed;
            s.bullet_frame_no += 1;
        });
    }
    let server = env::time().server_time_mills;
    SIM.with(|s| {
        let mut s = s.borrow_mut();
        s.time = if (s.time - server).abs() > 100 {
            server
        } else {
            s.time + (10.0 * n as f32) as i64
        };
    });
}

/// `compareTransform(t1, t2)`: whether the footprint moved.
fn compare_transform(t1: &Transform, t2: &Transform) -> bool {
    if (t1.origin.x - t2.origin.x).abs() > 0.01
        || (t1.origin.z - t2.origin.z).abs() > 0.01
        || t1.origin.y as i32 != t2.origin.y as i32
    {
        return true;
    }
    let a = t1.basis.column(2);
    let b = t2.basis.column(2);
    (a.x - b.x).abs() > 0.001 || (a.z - b.z).abs() > 0.001
}

/// `update()` (single player).
pub fn update() {
    if !created() {
        return;
    }
    update_physic();
    let mut ff = vec![0.0f32; 8192];
    let total = Bullet::call(|b| b.get_vehicle_count());
    let mut offset = 0;
    while offset < total {
        let n = Bullet::call(|b| b.get_vehicle_physics(offset, &mut ff));
        if n <= 0 {
            break;
        }
        offset += n;
        let mut f = 0usize;
        for _ in 0..n {
            let id = ff[f] as i32;
            let origin = Vector3f::new(ff[f + 1], ff[f + 2], ff[f + 3]);
            let q = Quaternionf::new(ff[f + 4], ff[f + 5], ff[f + 6], ff[f + 7]);
            let vel = Vector3f::new(ff[f + 8], ff[f + 9], ff[f + 10]);
            let speed = ff[f + 11];
            let is_collide = ff[f + 12];
            let wheel_count = ff[f + 13] as i32;
            f += 14;
            let mut wheels = [[0.0f32; 4]; 4];
            for w in 0..wheel_count.max(0) as usize {
                if let Some(slot) = wheels.get_mut(w) {
                    *slot = [ff[f], ff[f + 1], ff[f + 2], ff[f + 3]];
                }
                f += 4;
            }
            let t = Transform {
                basis: Matrix3f::from_quat(&q),
                origin,
            };
            let Some(v) = crate::manager::vehicle_by_id(id as i16) else {
                continue;
            };
            v.set_speed_kmh(speed);
            let poly_t = v.st().poly.t;
            if compare_transform(&t, &poly_t) {
                v.st_mut().poly_dirty = true;
            }
            let mut st = v.st_mut();
            st.jni_transform = t;
            st.jni_linear_velocity = vel;
            st.jni_is_collide = is_collide > 0.5;
            for (info, w) in st
                .wheel_info
                .iter_mut()
                .zip(wheels.iter())
                .take(wheel_count.clamp(0, 4) as usize)
            {
                info.steering = w[0];
                info.rotation = w[1];
                info.skid_info = w[2];
                info.suspension_length = w[3];
            }
        }
    }
    // Physics objects (`physicsObjectMap`) are not vehicles; the call is
    // made for the replay order.
    Bullet::call(|b| b.get_object_physics(&mut ff));
}
