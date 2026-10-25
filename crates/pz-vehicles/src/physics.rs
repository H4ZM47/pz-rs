//! Every `zombie.core.physics.Bullet` native the vehicle code calls
//! (`BaseVehicle`, `CarController`, `WorldSimulation`, `VehicleScript`,
//! `IsoChunk.updatePhysicsForLevel`), as one trait whose methods mirror the
//! natives one to one: same names in Rust style, same argument order and
//! meaning, same float-array layouts and `ToBullet` buffers, the same Java
//! exceptions ([`JniError`]) and the same callbacks into Java during
//! `stepSimulation` ([`JavaCallbacks`]). `pz_bullet::PzBullet` implements
//! it (see [`bullet`]).
//!
//! Coordinates are Bullet's: `x`/`z` are the horizontal world axes relative
//! to `WorldSimulation.offsetX/offsetY` except where a native takes world
//! coordinates (`addVehicle`, `teleportVehicle`, `getOwnVehiclePhysics`
//! and `setOwnVehiclePhysics` use world `x`, world `y` and the physics
//! height), `y` is up in physics units (one level is `3 * 0.8164967`).
//!
//! The [`Bullet`] thread-local holds the installed implementation, the way
//! the original calls the static natives. A native that throws is recorded
//! ([`take_exceptions`]) and answers its default, where the original's
//! exception would end the Java call that made it.

pub use pz_bullet::api::{JavaCallbacks, JniError, NoCallbacks};
use std::cell::RefCell;

/// The result of a native: its value, or the Java exception it throws.
pub type PhysicsResult<T> = Result<T, JniError>;

/// Floats per vehicle `getVehiclePhysics` writes before the wheels: `id +
/// 0.1, x, y, z, qx, qy, qz, qw, vx, vy, vz, speedKmHour, isCollide (cleared
/// once read), wheelCount + 0.1`, then `steering, rotation, skidInfo,
/// suspensionLength` per wheel. The array holds `len / 30` vehicles.
pub const VEHICLE_PHYSICS_HEADER: usize = 14;
/// Floats per wheel in `getVehiclePhysics` and `getOwnVehiclePhysics`.
pub const VEHICLE_PHYSICS_WHEEL: usize = 4;
/// Floats per vehicle slot the `getVehiclePhysics` array is divided into.
pub const VEHICLE_PHYSICS_STRIDE: usize = 30;
/// `BaseVehicle.PHYSICS_PARAM_COUNT`: the `getOwnVehiclePhysics` /
/// `setOwnVehiclePhysics` buffer (`x, y, height, qx, qy, qz, qw, vx, vy, vz,
/// wheelCount + 0.1`, then 4 per wheel, for 4 wheels).
pub const PHYSICS_PARAM_COUNT: usize = 27;
/// `BaseVehicle.wheelParams`: the `setVehicleParams` array (the native
/// reads 6 floats per actual wheel: `[0] > 0.1` tire present, `[1]`
/// inflation, `[2]` friction (unused), `[3]` damping relaxation, `[4]`
/// damping compression, `[5]` added to the suspension rest length).
pub const WHEEL_PARAMS: usize = 24;

/// `zombie.core.physics.Bullet` natives used by vehicles and chunks.
#[allow(clippy::too_many_arguments)]
pub trait VehiclePhysics {
    /// `initWorld(minCellX, minCellY, maxCellX, maxCellY, offsetX, offsetY,
    /// isServer)`.
    fn init_world(
        &mut self,
        min_cell_x: i32,
        min_cell_y: i32,
        max_cell_x: i32,
        max_cell_y: i32,
        offset_x: i32,
        offset_y: i32,
        is_server: bool,
    ) -> PhysicsResult<()>;
    /// `destroyWorld()`.
    fn destroy_world(&mut self);
    /// `activateChunkMap(playerIndex, wx, wy, chunkGridWidth)`.
    fn activate_chunk_map(
        &mut self,
        player_index: i32,
        wx: i32,
        wy: i32,
        chunk_grid_width: i32,
    ) -> PhysicsResult<()>;
    /// `deactivateChunkMap(playerIndex)`.
    fn deactivate_chunk_map(&mut self, player_index: i32) -> PhysicsResult<()>;
    /// `scrollChunkMap(playerIndex, dir)` (0 left, 1 right, 2 up, 3 down).
    fn scroll_chunk_map(&mut self, player_index: i32, dir: i32) -> PhysicsResult<()>;
    /// `setChunkMinMaxLevel(wx, wy, minLevel, maxLevel)`.
    fn set_chunk_min_max_level(
        &mut self,
        wx: i32,
        wy: i32,
        min_level: i32,
        max_level: i32,
    ) -> PhysicsResult<()>;
    /// `ToBullet(buffer)`: chunk shape records
    /// (`Bullet.beginUpdateChunk` / `updateChunk` / `endUpdateChunk`).
    fn to_bullet(&mut self, buf: &[u8]);
    /// `defineVehicleScript(fullName, params)` (`VehicleScript.toBullet`).
    fn define_vehicle_script(&mut self, full_name: &str, params: &[f32]) -> PhysicsResult<()>;
    /// `defineVehiclePhysicsMesh(fullName, shapeIndex, points)`.
    fn define_vehicle_physics_mesh(
        &mut self,
        full_name: &str,
        shape_index: i32,
        points: &[f32],
    ) -> PhysicsResult<()>;
    /// `addVehicle(id, worldX, worldY, physicsZ, qx, qy, qz, qw, scriptName)`.
    fn add_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
        script_name: &str,
    ) -> PhysicsResult<()>;
    /// `removeVehicle(id)` (remove its constraints first).
    fn remove_vehicle(&mut self, id: i32) -> PhysicsResult<()>;
    /// `controlVehicle(id, engineForce, brakingForce, vehicleSteering)`.
    fn control_vehicle(
        &mut self,
        id: i32,
        engine_force: f32,
        braking_force: f32,
        steering: f32,
    ) -> PhysicsResult<()>;
    /// `setVehicleActive(id, active)` (throws for an unknown id when
    /// activating).
    fn set_vehicle_active(&mut self, id: i32, active: bool) -> PhysicsResult<()>;
    /// `setVehicleStatic(id, static)`.
    fn set_vehicle_static(&mut self, id: i32, is_static: bool) -> PhysicsResult<i32>;
    /// `applyCentralForceToVehicle(id, x, y, z)`.
    fn apply_central_force_to_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
    ) -> PhysicsResult<()>;
    /// `applyTorqueToVehicle(id, x, y, z)`.
    fn apply_torque_to_vehicle(&mut self, id: i32, x: f32, y: f32, z: f32) -> PhysicsResult<()>;
    /// `teleportVehicle(id, worldX, worldY, physicsZ, qx, qy, qz, qw)`.
    fn teleport_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) -> PhysicsResult<()>;
    /// `setTireInflation(id, wheelIndex, inflation)`.
    fn set_tire_inflation(
        &mut self,
        id: i32,
        wheel_index: i32,
        inflation: f32,
    ) -> PhysicsResult<()>;
    /// `setTireRemoved(id, wheelIndex, removed)`.
    fn set_tire_removed(&mut self, id: i32, wheel_index: i32, removed: bool) -> PhysicsResult<()>;
    /// `stepSimulation(timeStep, maxSubSteps, fixedTimeStep)`; the native
    /// calls back `updatePhysicsForLevelIfNeeded` and
    /// `onVehicleConstraintImpulse` through `cb`.
    fn step_simulation(
        &mut self,
        time_step: f32,
        max_sub_steps: i32,
        fixed_time_step: f32,
        cb: &mut dyn JavaCallbacks,
    ) -> PhysicsResult<()>;
    /// `getVehicleCount()`.
    fn get_vehicle_count(&mut self) -> PhysicsResult<i32>;
    /// `getVehiclePhysics(offset, out)`: vehicles written.
    fn get_vehicle_physics(&mut self, offset: i32, out: &mut [f32]) -> PhysicsResult<i32>;
    /// `getOwnVehiclePhysics(id, out)`.
    fn get_own_vehicle_physics(&mut self, id: i32, out: &mut [f32]) -> PhysicsResult<i32>;
    /// `setOwnVehiclePhysics(id, params, force)`: the vehicle-map size, or
    /// -2 (no world), -4 (unknown id), -5 (wheel count); `force` sweep-tests
    /// the move.
    fn set_own_vehicle_physics(
        &mut self,
        id: i32,
        params: &mut [f32],
        force: bool,
    ) -> PhysicsResult<i32>;
    /// `setVehicleParams(id, params)`: 0, or -1 for an unknown id.
    fn set_vehicle_params(&mut self, id: i32, params: &mut [f32]) -> PhysicsResult<i32>;
    /// `setVehicleMass(id, mass)`.
    fn set_vehicle_mass(&mut self, id: i32, mass: f32) -> PhysicsResult<i32>;
    /// `getObjectPhysics(out)`.
    fn get_object_physics(&mut self, out: &mut [f32]) -> PhysicsResult<i32>;
    /// `setVehicleVelocityMultiplier(id, maxSpeed, multiplier)`.
    fn set_vehicle_velocity_multiplier(
        &mut self,
        id: i32,
        max_speed: f32,
        multiplier: f32,
    ) -> PhysicsResult<()>;
    /// `addVehicleConstraint6DOF(idA, idB, params[18])`.
    fn add_6dof_constraint(&mut self, id_a: i32, id_b: i32, p: &[f32; 18]) -> PhysicsResult<i32>;
    /// `addRopeConstraint(idA, idB, ax, ay, az, bx, by, bz, maxDistance)`
    /// (throws for the same id twice or a missing vehicle).
    fn add_rope_constraint(
        &mut self,
        id_a: i32,
        id_b: i32,
        ax: f32,
        ay: f32,
        az: f32,
        bx: f32,
        by: f32,
        bz: f32,
        max_distance: f32,
    ) -> PhysicsResult<i32>;
    /// `setConstraintERP(constraintId, erp, axis)`.
    fn set_constraint_erp(&mut self, constraint_id: i32, erp: f32, axis: i32) -> PhysicsResult<()>;
    /// `removeConstraint(constraintId)`.
    fn remove_constraint(&mut self, constraint_id: i32) -> PhysicsResult<()>;
}

/// No physics: every native succeeds and reports nothing.
#[derive(Default)]
pub struct NoPhysics;

impl VehiclePhysics for NoPhysics {
    fn init_world(
        &mut self,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: bool,
    ) -> PhysicsResult<()> {
        Ok(())
    }
    fn destroy_world(&mut self) {}
    fn activate_chunk_map(&mut self, _: i32, _: i32, _: i32, _: i32) -> PhysicsResult<()> {
        Ok(())
    }
    fn deactivate_chunk_map(&mut self, _: i32) -> PhysicsResult<()> {
        Ok(())
    }
    fn scroll_chunk_map(&mut self, _: i32, _: i32) -> PhysicsResult<()> {
        Ok(())
    }
    fn set_chunk_min_max_level(&mut self, _: i32, _: i32, _: i32, _: i32) -> PhysicsResult<()> {
        Ok(())
    }
    fn to_bullet(&mut self, _: &[u8]) {}
    fn define_vehicle_script(&mut self, _: &str, _: &[f32]) -> PhysicsResult<()> {
        Ok(())
    }
    fn define_vehicle_physics_mesh(&mut self, _: &str, _: i32, _: &[f32]) -> PhysicsResult<()> {
        Ok(())
    }
    fn add_vehicle(
        &mut self,
        _: i32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: &str,
    ) -> PhysicsResult<()> {
        Ok(())
    }
    fn remove_vehicle(&mut self, _: i32) -> PhysicsResult<()> {
        Ok(())
    }
    fn control_vehicle(&mut self, _: i32, _: f32, _: f32, _: f32) -> PhysicsResult<()> {
        Ok(())
    }
    fn set_vehicle_active(&mut self, _: i32, _: bool) -> PhysicsResult<()> {
        Ok(())
    }
    fn set_vehicle_static(&mut self, _: i32, _: bool) -> PhysicsResult<i32> {
        Ok(0)
    }
    fn apply_central_force_to_vehicle(
        &mut self,
        _: i32,
        _: f32,
        _: f32,
        _: f32,
    ) -> PhysicsResult<()> {
        Ok(())
    }
    fn apply_torque_to_vehicle(&mut self, _: i32, _: f32, _: f32, _: f32) -> PhysicsResult<()> {
        Ok(())
    }
    fn teleport_vehicle(
        &mut self,
        _: i32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
    ) -> PhysicsResult<()> {
        Ok(())
    }
    fn set_tire_inflation(&mut self, _: i32, _: i32, _: f32) -> PhysicsResult<()> {
        Ok(())
    }
    fn set_tire_removed(&mut self, _: i32, _: i32, _: bool) -> PhysicsResult<()> {
        Ok(())
    }
    fn step_simulation(
        &mut self,
        _: f32,
        _: i32,
        _: f32,
        _: &mut dyn JavaCallbacks,
    ) -> PhysicsResult<()> {
        Ok(())
    }
    fn get_vehicle_count(&mut self) -> PhysicsResult<i32> {
        Ok(0)
    }
    fn get_vehicle_physics(&mut self, _: i32, _: &mut [f32]) -> PhysicsResult<i32> {
        Ok(0)
    }
    fn get_own_vehicle_physics(&mut self, _: i32, _: &mut [f32]) -> PhysicsResult<i32> {
        Ok(-1)
    }
    fn set_own_vehicle_physics(&mut self, _: i32, _: &mut [f32], _: bool) -> PhysicsResult<i32> {
        Ok(0)
    }
    fn set_vehicle_params(&mut self, _: i32, _: &mut [f32]) -> PhysicsResult<i32> {
        Ok(0)
    }
    fn set_vehicle_mass(&mut self, _: i32, _: f32) -> PhysicsResult<i32> {
        Ok(0)
    }
    fn get_object_physics(&mut self, _: &mut [f32]) -> PhysicsResult<i32> {
        Ok(0)
    }
    fn set_vehicle_velocity_multiplier(&mut self, _: i32, _: f32, _: f32) -> PhysicsResult<()> {
        Ok(())
    }
    fn add_6dof_constraint(&mut self, _: i32, _: i32, _: &[f32; 18]) -> PhysicsResult<i32> {
        Ok(-1)
    }
    fn add_rope_constraint(
        &mut self,
        _: i32,
        _: i32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
    ) -> PhysicsResult<i32> {
        Ok(-1)
    }
    fn set_constraint_erp(&mut self, _: i32, _: f32, _: i32) -> PhysicsResult<()> {
        Ok(())
    }
    fn remove_constraint(&mut self, _: i32) -> PhysicsResult<()> {
        Ok(())
    }
}

type Deferred = Box<dyn FnOnce(&mut dyn VehiclePhysics)>;

thread_local! {
    static BULLET: RefCell<Box<dyn VehiclePhysics>> = RefCell::new(Box::new(NoPhysics));
    static EXCEPTIONS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static DEFERRED: RefCell<Vec<Deferred>> = RefCell::new(Vec::new());
}

/// The Java exceptions natives threw since the last call, as
/// `class: message`.
pub fn take_exceptions() -> Vec<String> {
    EXCEPTIONS.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

fn record(e: JniError) {
    EXCEPTIONS.with(|x| x.borrow_mut().push(e.class_and_message()));
}

/// The installed natives.
pub struct Bullet;

impl Bullet {
    /// Installs `physics`, returning the previous implementation.
    pub fn install(physics: Box<dyn VehiclePhysics>) -> Box<dyn VehiclePhysics> {
        BULLET.with(|b| std::mem::replace(&mut *b.borrow_mut(), physics))
    }

    /// Runs `f` on the natives (not from inside `stepSimulation`'s
    /// callbacks: see [`Bullet::call_or_defer`]).
    pub fn with<R>(f: impl FnOnce(&mut dyn VehiclePhysics) -> R) -> R {
        BULLET.with(|b| f(&mut **b.borrow_mut()))
    }

    /// Calls a native; a thrown exception is recorded and the default
    /// returned.
    pub fn call<R: Default>(f: impl FnOnce(&mut dyn VehiclePhysics) -> PhysicsResult<R>) -> R {
        match Self::with(f) {
            Ok(r) => r,
            Err(e) => {
                record(e);
                R::default()
            }
        }
    }

    /// `stepSimulation` with the callbacks. While the step runs, natives
    /// the callbacks call (`breakConstraint` from
    /// `onVehicleConstraintImpulse`) are queued and run, in order, as soon
    /// as the step returns: the native's own state is not reachable from
    /// inside its step here, and nothing else touches it in between.
    pub fn step_simulation(
        time_step: f32,
        max_sub_steps: i32,
        fixed_time_step: f32,
        cb: &mut dyn JavaCallbacks,
    ) {
        let mut real =
            BULLET.with(|b| std::mem::replace(&mut *b.borrow_mut(), Box::new(Deferring)));
        let r = real.step_simulation(time_step, max_sub_steps, fixed_time_step, cb);
        let queued = BULLET.with(|b| std::mem::replace(&mut *b.borrow_mut(), real));
        if let Err(e) = r {
            record(e);
        }
        drop(queued);
        DEFERRED.with(|q| {
            let ops: Vec<Deferred> = std::mem::take(&mut *q.borrow_mut());
            for f in ops {
                Self::with(|b| f(b));
            }
        });
    }
}

/// The natives as seen from inside a step: calls are queued for when the
/// step returns; readbacks are not available.
#[derive(Default)]
struct Deferring;

fn defer(
    f: impl FnOnce(&mut dyn VehiclePhysics) -> PhysicsResult<()> + 'static,
) -> PhysicsResult<()> {
    DEFERRED.with(|d| {
        d.borrow_mut()
            .push(Box::new(move |p: &mut dyn VehiclePhysics| {
                if let Err(e) = f(p) {
                    record(e);
                }
            }))
    });
    Ok(())
}

fn defer_i(
    f: impl FnOnce(&mut dyn VehiclePhysics) -> PhysicsResult<i32> + 'static,
) -> PhysicsResult<i32> {
    defer(move |p| f(p).map(|_| ()))?;
    Ok(0)
}

impl VehiclePhysics for Deferring {
    fn init_world(
        &mut self,
        a: i32,
        b: i32,
        c: i32,
        d: i32,
        e: i32,
        f: i32,
        s: bool,
    ) -> PhysicsResult<()> {
        defer(move |p| p.init_world(a, b, c, d, e, f, s))
    }
    fn destroy_world(&mut self) {
        let _ = defer(|p| {
            p.destroy_world();
            Ok(())
        });
    }
    fn activate_chunk_map(&mut self, i: i32, wx: i32, wy: i32, w: i32) -> PhysicsResult<()> {
        defer(move |p| p.activate_chunk_map(i, wx, wy, w))
    }
    fn deactivate_chunk_map(&mut self, i: i32) -> PhysicsResult<()> {
        defer(move |p| p.deactivate_chunk_map(i))
    }
    fn scroll_chunk_map(&mut self, i: i32, dir: i32) -> PhysicsResult<()> {
        defer(move |p| p.scroll_chunk_map(i, dir))
    }
    fn set_chunk_min_max_level(&mut self, wx: i32, wy: i32, a: i32, b: i32) -> PhysicsResult<()> {
        defer(move |p| p.set_chunk_min_max_level(wx, wy, a, b))
    }
    fn to_bullet(&mut self, buf: &[u8]) {
        let buf = buf.to_vec();
        let _ = defer(move |p| {
            p.to_bullet(&buf);
            Ok(())
        });
    }
    fn define_vehicle_script(&mut self, name: &str, params: &[f32]) -> PhysicsResult<()> {
        let (name, params) = (name.to_string(), params.to_vec());
        defer(move |p| p.define_vehicle_script(&name, &params))
    }
    fn define_vehicle_physics_mesh(
        &mut self,
        name: &str,
        i: i32,
        pts: &[f32],
    ) -> PhysicsResult<()> {
        let (name, pts) = (name.to_string(), pts.to_vec());
        defer(move |p| p.define_vehicle_physics_mesh(&name, i, &pts))
    }
    fn add_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
        script: &str,
    ) -> PhysicsResult<()> {
        let script = script.to_string();
        defer(move |p| p.add_vehicle(id, x, y, z, qx, qy, qz, qw, &script))
    }
    fn remove_vehicle(&mut self, id: i32) -> PhysicsResult<()> {
        defer(move |p| p.remove_vehicle(id))
    }
    fn control_vehicle(&mut self, id: i32, e: f32, b: f32, s: f32) -> PhysicsResult<()> {
        defer(move |p| p.control_vehicle(id, e, b, s))
    }
    fn set_vehicle_active(&mut self, id: i32, a: bool) -> PhysicsResult<()> {
        defer(move |p| p.set_vehicle_active(id, a))
    }
    fn set_vehicle_static(&mut self, id: i32, s: bool) -> PhysicsResult<i32> {
        defer_i(move |p| p.set_vehicle_static(id, s))
    }
    fn apply_central_force_to_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
    ) -> PhysicsResult<()> {
        defer(move |p| p.apply_central_force_to_vehicle(id, x, y, z))
    }
    fn apply_torque_to_vehicle(&mut self, id: i32, x: f32, y: f32, z: f32) -> PhysicsResult<()> {
        defer(move |p| p.apply_torque_to_vehicle(id, x, y, z))
    }
    fn teleport_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) -> PhysicsResult<()> {
        defer(move |p| p.teleport_vehicle(id, x, y, z, qx, qy, qz, qw))
    }
    fn set_tire_inflation(&mut self, id: i32, w: i32, v: f32) -> PhysicsResult<()> {
        defer(move |p| p.set_tire_inflation(id, w, v))
    }
    fn set_tire_removed(&mut self, id: i32, w: i32, r: bool) -> PhysicsResult<()> {
        defer(move |p| p.set_tire_removed(id, w, r))
    }
    fn step_simulation(
        &mut self,
        _: f32,
        _: i32,
        _: f32,
        _: &mut dyn JavaCallbacks,
    ) -> PhysicsResult<()> {
        Err(JniError::Runtime("stepSimulation inside stepSimulation"))
    }
    fn get_vehicle_count(&mut self) -> PhysicsResult<i32> {
        Err(JniError::Runtime("getVehicleCount inside stepSimulation"))
    }
    fn get_vehicle_physics(&mut self, _: i32, _: &mut [f32]) -> PhysicsResult<i32> {
        Err(JniError::Runtime("getVehiclePhysics inside stepSimulation"))
    }
    fn get_own_vehicle_physics(&mut self, _: i32, _: &mut [f32]) -> PhysicsResult<i32> {
        Err(JniError::Runtime(
            "getOwnVehiclePhysics inside stepSimulation",
        ))
    }
    fn set_own_vehicle_physics(
        &mut self,
        id: i32,
        params: &mut [f32],
        force: bool,
    ) -> PhysicsResult<i32> {
        let mut params = params.to_vec();
        defer_i(move |p| p.set_own_vehicle_physics(id, &mut params, force))
    }
    fn set_vehicle_params(&mut self, id: i32, params: &mut [f32]) -> PhysicsResult<i32> {
        let mut params = params.to_vec();
        defer_i(move |p| p.set_vehicle_params(id, &mut params))
    }
    fn set_vehicle_mass(&mut self, id: i32, m: f32) -> PhysicsResult<i32> {
        defer_i(move |p| p.set_vehicle_mass(id, m))
    }
    fn get_object_physics(&mut self, _: &mut [f32]) -> PhysicsResult<i32> {
        Err(JniError::Runtime("getObjectPhysics inside stepSimulation"))
    }
    fn set_vehicle_velocity_multiplier(
        &mut self,
        id: i32,
        max: f32,
        mul: f32,
    ) -> PhysicsResult<()> {
        defer(move |p| p.set_vehicle_velocity_multiplier(id, max, mul))
    }
    fn add_6dof_constraint(&mut self, a: i32, b: i32, c: &[f32; 18]) -> PhysicsResult<i32> {
        let c = *c;
        defer_i(move |p| p.add_6dof_constraint(a, b, &c))
    }
    fn add_rope_constraint(
        &mut self,
        a: i32,
        b: i32,
        ax: f32,
        ay: f32,
        az: f32,
        bx: f32,
        by: f32,
        bz: f32,
        d: f32,
    ) -> PhysicsResult<i32> {
        defer_i(move |p| p.add_rope_constraint(a, b, ax, ay, az, bx, by, bz, d))
    }
    fn set_constraint_erp(&mut self, c: i32, erp: f32, axis: i32) -> PhysicsResult<()> {
        defer(move |p| p.set_constraint_erp(c, erp, axis))
    }
    fn remove_constraint(&mut self, c: i32) -> PhysicsResult<()> {
        defer(move |p| p.remove_constraint(c))
    }
}

pub mod bullet;
pub mod replay;
