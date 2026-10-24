//! Every `zombie.core.physics.Bullet` native the vehicle code calls
//! (`BaseVehicle`, `CarController`, `WorldSimulation`, `VehicleScript`),
//! as one trait whose methods mirror the natives one to one: same names in
//! Rust style, same argument order and meaning, same float-array layouts.
//!
//! Coordinates are Bullet's: `x`/`z` are the horizontal world axes relative
//! to `WorldSimulation.offsetX/offsetY` except where a native takes world
//! coordinates (`addVehicle`, `teleportVehicle` take world `x`, world `y`
//! and the physics height), `y` is up in physics units (one level is
//! `3 * 0.8164967 = 2.44949`).
//!
//! The [`Bullet`] thread-local holds the installed implementation, the way
//! the original calls the static natives.

use std::cell::RefCell;

/// Floats per vehicle in `getVehiclePhysics` before the wheels:
/// `id, x, y, z, qx, qy, qz, qw, vx, vy, vz, speedKmHour, isCollide,
/// wheelCount`, then `steering, rotation, skidInfo, suspensionLength` per
/// wheel.
pub const VEHICLE_PHYSICS_HEADER: usize = 14;
/// Floats per wheel in `getVehiclePhysics`.
pub const VEHICLE_PHYSICS_WHEEL: usize = 4;
/// `BaseVehicle.PHYSICS_PARAM_COUNT`: the `getOwnVehiclePhysics` /
/// `setOwnVehiclePhysics` buffer.
pub const PHYSICS_PARAM_COUNT: usize = 27;
/// `setVehicleParams`: 6 floats per wheel for 4 wheels (`present`,
/// `inflation`, `friction`, `suspensionDamping`, `suspensionCompression`,
/// `bump`).
pub const WHEEL_PARAMS: usize = 24;

/// `zombie.core.physics.Bullet` natives used by vehicles.
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
    );
    /// `destroyWorld()`.
    fn destroy_world(&mut self);
    /// `activateChunkMap(playerIndex, wx, wy, chunkGridWidth)`.
    fn activate_chunk_map(&mut self, player_index: i32, wx: i32, wy: i32, chunk_grid_width: i32);
    /// `deactivateChunkMap(playerIndex)`.
    fn deactivate_chunk_map(&mut self, player_index: i32);
    /// `scrollChunkMap(playerIndex, dir)` (0 left, 1 right, 2 up, 3 down).
    fn scroll_chunk_map(&mut self, player_index: i32, dir: i32);
    /// `defineVehicleScript(fullName, params)` (`VehicleScript.toBullet`).
    fn define_vehicle_script(&mut self, full_name: &str, params: &[f32]);
    /// `defineVehiclePhysicsMesh(fullName, shapeIndex, points)`.
    fn define_vehicle_physics_mesh(&mut self, full_name: &str, shape_index: i32, points: &[f32]);
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
    );
    /// `removeVehicle(id)`.
    fn remove_vehicle(&mut self, id: i32);
    /// `controlVehicle(id, engineForce, brakingForce, vehicleSteering)`.
    fn control_vehicle(&mut self, id: i32, engine_force: f32, braking_force: f32, steering: f32);
    /// `setVehicleActive(id, active)`.
    fn set_vehicle_active(&mut self, id: i32, active: bool);
    /// `setVehicleStatic(id, static)`.
    fn set_vehicle_static(&mut self, id: i32, is_static: bool) -> i32;
    /// `applyCentralForceToVehicle(id, x, y, z)`.
    fn apply_central_force_to_vehicle(&mut self, id: i32, x: f32, y: f32, z: f32);
    /// `applyTorqueToVehicle(id, x, y, z)`.
    fn apply_torque_to_vehicle(&mut self, id: i32, x: f32, y: f32, z: f32);
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
    );
    /// `setTireInflation(id, wheelIndex, inflation)`.
    fn set_tire_inflation(&mut self, id: i32, wheel_index: i32, inflation: f32);
    /// `setTireRemoved(id, wheelIndex, removed)`.
    fn set_tire_removed(&mut self, id: i32, wheel_index: i32, removed: bool);
    /// `stepSimulation(timeStep, maxSubSteps, fixedTimeStep)`.
    fn step_simulation(&mut self, time_step: f32, max_sub_steps: i32, fixed_time_step: f32);
    /// `getVehicleCount()`.
    fn get_vehicle_count(&mut self) -> i32;
    /// `getVehiclePhysics(offset, out)`: fills `out` with the vehicles from
    /// `offset` on (see [`VEHICLE_PHYSICS_HEADER`]) and returns how many.
    fn get_vehicle_physics(&mut self, offset: i32, out: &mut [f32]) -> i32;
    /// `getOwnVehiclePhysics(id, out)`.
    fn get_own_vehicle_physics(&mut self, id: i32, out: &mut [f32]) -> i32;
    /// `setOwnVehiclePhysics(id, params, force)`.
    fn set_own_vehicle_physics(&mut self, id: i32, params: &[f32], force: bool) -> i32;
    /// `setVehicleParams(id, params)` (see [`WHEEL_PARAMS`]).
    fn set_vehicle_params(&mut self, id: i32, params: &[f32]) -> i32;
    /// `setVehicleMass(id, mass)`.
    fn set_vehicle_mass(&mut self, id: i32, mass: f32) -> i32;
    /// `getObjectPhysics(out)`: `id, x, y, z` per physics object.
    fn get_object_physics(&mut self, out: &mut [f32]) -> i32;
    /// `setVehicleVelocityMultiplier(id, maxSpeed, multiplier)`.
    fn set_vehicle_velocity_multiplier(&mut self, id: i32, max_speed: f32, multiplier: f32);
    /// `add6DofConstraint(idA, idB, pivotA xyz, pivotB xyz, linear lower
    /// xyz, linear upper xyz, angular lower xyz, angular upper xyz)`.
    fn add_6dof_constraint(&mut self, id_a: i32, id_b: i32, p: [f32; 18]) -> i32;
    /// `addRopeConstraint(idA, idB, pivotA xyz, pivotB xyz, maxDistance)`.
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
    ) -> i32;
    /// `setConstraintERP(constraintId, erp, axis)`.
    fn set_constraint_erp(&mut self, constraint_id: i32, erp: f32, axis: i32);
    /// `removeConstraint(constraintId)`.
    fn remove_constraint(&mut self, constraint_id: i32);
}

/// A world without physics: every query returns zero / nothing.
#[derive(Default)]
pub struct NoPhysics;

impl VehiclePhysics for NoPhysics {
    fn init_world(&mut self, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: bool) {}
    fn destroy_world(&mut self) {}
    fn activate_chunk_map(&mut self, _: i32, _: i32, _: i32, _: i32) {}
    fn deactivate_chunk_map(&mut self, _: i32) {}
    fn scroll_chunk_map(&mut self, _: i32, _: i32) {}
    fn define_vehicle_script(&mut self, _: &str, _: &[f32]) {}
    fn define_vehicle_physics_mesh(&mut self, _: &str, _: i32, _: &[f32]) {}
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
    ) {
    }
    fn remove_vehicle(&mut self, _: i32) {}
    fn control_vehicle(&mut self, _: i32, _: f32, _: f32, _: f32) {}
    fn set_vehicle_active(&mut self, _: i32, _: bool) {}
    fn set_vehicle_static(&mut self, _: i32, _: bool) -> i32 {
        0
    }
    fn apply_central_force_to_vehicle(&mut self, _: i32, _: f32, _: f32, _: f32) {}
    fn apply_torque_to_vehicle(&mut self, _: i32, _: f32, _: f32, _: f32) {}
    fn teleport_vehicle(&mut self, _: i32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
    }
    fn set_tire_inflation(&mut self, _: i32, _: i32, _: f32) {}
    fn set_tire_removed(&mut self, _: i32, _: i32, _: bool) {}
    fn step_simulation(&mut self, _: f32, _: i32, _: f32) {}
    fn get_vehicle_count(&mut self) -> i32 {
        0
    }
    fn get_vehicle_physics(&mut self, _: i32, _: &mut [f32]) -> i32 {
        0
    }
    fn get_own_vehicle_physics(&mut self, _: i32, _: &mut [f32]) -> i32 {
        -1
    }
    fn set_own_vehicle_physics(&mut self, _: i32, _: &[f32], _: bool) -> i32 {
        0
    }
    fn set_vehicle_params(&mut self, _: i32, _: &[f32]) -> i32 {
        0
    }
    fn set_vehicle_mass(&mut self, _: i32, _: f32) -> i32 {
        0
    }
    fn get_object_physics(&mut self, _: &mut [f32]) -> i32 {
        0
    }
    fn set_vehicle_velocity_multiplier(&mut self, _: i32, _: f32, _: f32) {}
    fn add_6dof_constraint(&mut self, _: i32, _: i32, _: [f32; 18]) -> i32 {
        -1
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
    ) -> i32 {
        -1
    }
    fn set_constraint_erp(&mut self, _: i32, _: f32, _: i32) {}
    fn remove_constraint(&mut self, _: i32) {}
}

thread_local! {
    static BULLET: RefCell<Box<dyn VehiclePhysics>> = RefCell::new(Box::new(NoPhysics));
}

/// The installed physics (the original's static `Bullet` natives).
pub struct Bullet;

impl Bullet {
    /// Installs `physics`, returning the previous implementation.
    pub fn install(physics: Box<dyn VehiclePhysics>) -> Box<dyn VehiclePhysics> {
        BULLET.with(|b| std::mem::replace(&mut *b.borrow_mut(), physics))
    }

    /// Runs `f` on the installed physics. Physics calls never re-enter
    /// vehicle code.
    pub fn with<R>(f: impl FnOnce(&mut dyn VehiclePhysics) -> R) -> R {
        BULLET.with(|b| f(&mut **b.borrow_mut()))
    }
}

pub mod replay;
