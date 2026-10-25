//! The `zombie.core.physics.Bullet` natives, one method each.
//!
//! [`PzBullet`] holds what the native library keeps in globals: the world
//! (`WorldSimulation::instance`), the vehicle scripts
//! (`PZVehicleScript::m_scripts`) and the vehicle list `getVehicleCount`
//! builds for `getVehiclePhysics`. Arguments and float-array layouts are the
//! natives'; arrays are updated in place like `ReleaseFloatArrayElements`
//! with mode 0. Where the original throws a Java exception the method
//! returns [`JniError`] with the same class and message.
//!
//! The ragdoll natives are in `api_ragdoll`, the ballistics natives in
//! `api_ballistics`.

use crate::bt::dynamics::generic_6dof::Generic6DofConstraint;
use crate::bt::dynamics::hinge::HingeConstraint;
use crate::bt::dynamics::point2point::Point2PointConstraint;
use crate::bt::dynamics::typed_constraint::ConstraintKind;
use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;
use crate::pz::script::VehicleScript;
use crate::pz::world::{WorldSimulation, LEVEL_SCALE};

/// The Java methods the native library calls back.
pub trait JavaCallbacks {
    /// `Bullet.updatePhysicsForLevelIfNeeded(wx, wy, level)`. Buffers the
    /// Java side passes to `ToBullet` during the call go into `to_bullet`
    /// (they are applied, in order, before the call returns).
    fn update_physics_for_level_if_needed(
        &mut self,
        wx: i32,
        wy: i32,
        level: i32,
        to_bullet: &mut Vec<Vec<u8>>,
    ) -> bool;
    /// `Bullet.onVehicleConstraintImpulse(constraintId, vehicleA, vehicleB,
    /// impulse)`.
    fn on_vehicle_constraint_impulse(
        &mut self,
        constraint_id: i32,
        vehicle_a: i32,
        vehicle_b: i32,
        impulse: f32,
    );
}

/// Callbacks that do nothing (`updatePhysicsForLevelIfNeeded` returns
/// `false`).
pub struct NoCallbacks;
impl JavaCallbacks for NoCallbacks {
    fn update_physics_for_level_if_needed(
        &mut self,
        _: i32,
        _: i32,
        _: i32,
        _: &mut Vec<Vec<u8>>,
    ) -> bool {
        false
    }
    fn on_vehicle_constraint_impulse(&mut self, _: i32, _: i32, _: i32, _: f32) {}
}

/// A Java exception the original throws (`ThrowNew`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JniError {
    NullPointer(&'static str),
    Runtime(&'static str),
    IllegalArgument(&'static str),
}

impl JniError {
    /// `getClass().getName() + ": " + getMessage()`.
    pub fn class_and_message(&self) -> String {
        match self {
            JniError::NullPointer(m) => format!("java.lang.NullPointerException: {m}"),
            JniError::Runtime(m) => format!("java.lang.RuntimeException: {m}"),
            JniError::IllegalArgument(m) => format!("java.lang.IllegalArgumentException: {m}"),
        }
    }
}

impl std::fmt::Display for JniError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.class_and_message())
    }
}
impl std::error::Error for JniError {}

const NULL_INSTANCE: &str = "WorldSimulation::instance is null";

/// The native library's global state.
#[derive(Default)]
pub struct PzBullet {
    pub world: Option<Box<WorldSimulation>>,
    pub scripts: Vec<VehicleScript>,
    /// `hack_vehicles`: vehicle ids, as `getVehicleCount` saw them.
    pub hack_vehicles: Vec<i32>,
    initialized: bool,
    /// `RagdollBuilder::instance`.
    pub ragdoll_builder: crate::pz::ragdoll_builder::RagdollBuilder,
    /// Ballistics statics (`api_ballistics`).
    pub ballistics: crate::api_ballistics::BallisticsGlobals,
}

fn quat_basis(qx: f32, qy: f32, qz: f32, qw: f32) -> Mat3 {
    let mut m = Mat3::IDENTITY;
    m.set_rotation(&Quat::new(qx as f64, qy as f64, qz as f64, qw as f64));
    m
}

macro_rules! world {
    ($s:expr, $e:expr) => {
        match $s.world.as_deref_mut() {
            Some(w) => w,
            None => return Err($e),
        }
    };
}

impl PzBullet {
    pub fn new() -> PzBullet {
        PzBullet::default()
    }

    fn find_script(&self, name: &str) -> Option<usize> {
        self.scripts.iter().position(|s| s.name == name)
    }

    /// `getPZBulletVersion()`.
    pub fn get_pz_bullet_version(&self) -> &'static str {
        "1.0.0.28"
    }

    /// `initPZBullet()`: sets up logging only.
    pub fn init_pz_bullet(&mut self) {
        self.initialized = true;
    }

    /// `isWorldInit()`.
    pub fn is_world_init(&self) -> bool {
        self.world.is_some()
    }

    /// `initWorld(minCellX, minCellY, maxCellX, maxCellY, offsetX, offsetY,
    /// isServer)`.
    #[allow(clippy::too_many_arguments)]
    pub fn init_world(
        &mut self,
        min_x: i32,
        min_y: i32,
        max_x: i32,
        max_y: i32,
        off_x: i32,
        off_y: i32,
        is_server: bool,
    ) -> Result<(), JniError> {
        if self.world.is_some() {
            return Err(JniError::Runtime("WorldSimulation::instance != null"));
        }
        self.world = Some(Box::new(WorldSimulation::new(
            min_x, min_y, max_x, max_y, off_x, off_y, is_server,
        )));
        self.ragdoll_builder.initialize();
        Ok(())
    }

    /// `destroyWorld()`.
    pub fn destroy_world(&mut self) {
        self.world = None;
    }

    /// `activateChunkMap(playerIndex, wx, wy, chunkGridWidth)`.
    pub fn activate_chunk_map(&mut self, i: i32, wx: i32, wy: i32, w: i32) -> Result<(), JniError> {
        let ws = world!(
            self,
            JniError::NullPointer("WorldSimulation::instance == null")
        );
        ws.activate_chunk_map(i, wx, wy, w);
        Ok(())
    }

    /// `deactivateChunkMap(playerIndex)`.
    pub fn deactivate_chunk_map(&mut self, i: i32) -> Result<(), JniError> {
        let ws = world!(
            self,
            JniError::NullPointer("WorldSimulation::instance == null")
        );
        ws.deactivate_chunk_map(i);
        Ok(())
    }

    /// `scrollChunkMap(playerIndex, dir)`: 0 left, 1 right, 2 up, 3 down.
    pub fn scroll_chunk_map(&mut self, i: i32, dir: i32) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer("chunkmap is null"));
        if !(0..4).contains(&i) || ws.chunk_maps[i as usize].is_none() {
            return Err(JniError::NullPointer("chunkmap is null"));
        }
        if (0..4).contains(&dir) {
            ws.scroll_chunk_map(i, dir);
        }
        Ok(())
    }

    /// `setChunkMinMaxLevel(wx, wy, minLevel, maxLevel)`.
    pub fn set_chunk_min_max_level(
        &mut self,
        wx: i32,
        wy: i32,
        min: i32,
        max: i32,
    ) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        for i in 0..4 {
            if let Some(c) = ws.get_chunk(i, wx, wy) {
                ws.chunk_set_min_max_level(c, min, max);
            }
        }
        Ok(())
    }

    /// `ToBullet(buffer)`: chunk shape records (no world: ignored).
    pub fn to_bullet(&mut self, buf: &[u8]) {
        if let Some(ws) = self.world.as_deref_mut() {
            ws.to_bullet(buf);
        }
    }

    /// `defineVehicleScript(fullName, params)`.
    pub fn define_vehicle_script(&mut self, name: &str, params: &[f32]) -> Result<(), JniError> {
        match self.find_script(name) {
            Some(i) => {
                self.scripts[i].from_java(params);
            }
            None => {
                let mut s = VehicleScript::new_default();
                if s.from_java(params) {
                    s.name = name.to_string();
                    self.scripts.push(s);
                }
            }
        }
        Ok(())
    }

    /// `defineVehiclePhysicsMesh(fullName, shapeIndex, points)`.
    pub fn define_vehicle_physics_mesh(
        &mut self,
        name: &str,
        index: i32,
        points: &[f32],
    ) -> Result<(), JniError> {
        match self.find_script(name) {
            Some(i) => {
                self.scripts[i].define_physics_mesh(index, (points.len() / 3) as i32, points);
                Ok(())
            }
            None => Err(JniError::Runtime("unknown vehicle script")),
        }
    }

    /// `clearPhysicsMeshes()`.
    pub fn clear_physics_meshes(&mut self) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        ws.clear_physics_meshes();
        Ok(())
    }

    /// `definePhysicsMesh(index, compound, points)`.
    pub fn define_physics_mesh(
        &mut self,
        index: i32,
        compound: bool,
        points: &[f32],
    ) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        ws.define_physics_mesh(index, compound, (points.len() / 3) as i32, points);
        Ok(())
    }

    /// `addVehicle(id, worldX, worldY, physicsZ, qx, qy, qz, qw, scriptName)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_vehicle(
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
    ) -> Result<(), JniError> {
        if self.world.is_none() {
            return Err(JniError::NullPointer(NULL_INSTANCE));
        }
        let Some(si) = self.find_script(script) else {
            return Err(JniError::Runtime("unknown vehicle script"));
        };
        let script = self.scripts[si].clone();
        let ws = self.world.as_deref_mut().unwrap();
        let origin = Vec3::new(
            (x - ws.offset_x as f32) as f64,
            z as f64,
            (y - ws.offset_y as f32) as f64,
        );
        let t = Transform::new(quat_basis(qx, qy, qz, qw), origin);
        ws.add_vehicle(id, &t, si, &script);
        Ok(())
    }

    /// `removeVehicle(id)`.
    pub fn remove_vehicle(&mut self, id: i32) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        ws.remove_vehicle(id);
        Ok(())
    }

    /// `controlVehicle(id, engineForce, brakingForce, steering)`.
    pub fn control_vehicle(
        &mut self,
        id: i32,
        engine: f32,
        brake: f32,
        steering: f32,
    ) -> Result<(), JniError> {
        let scripts = &self.scripts;
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if let Some(mut v) = ws.vehicles.remove(&id) {
            let s = &scripts[v.script];
            v.control(ws, engine, brake, steering, s);
            ws.vehicles.insert(id, v);
        }
        Ok(())
    }

    /// `setVehicleActive(id, active)`.
    pub fn set_vehicle_active(&mut self, id: i32, active: bool) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if active && !ws.vehicles.contains_key(&id) {
            return Err(JniError::Runtime("Vehicle not found"));
        }
        if let Some(mut v) = ws.vehicles.remove(&id) {
            v.set_active(ws, active);
            ws.vehicles.insert(id, v);
        }
        Ok(())
    }

    /// `setVehicleStatic(id, static)`: 1 when the vehicle exists.
    pub fn set_vehicle_static(&mut self, id: i32, is_static: bool) -> Result<i32, JniError> {
        let scripts = &self.scripts;
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        match ws.vehicles.remove(&id) {
            Some(mut v) => {
                let s = &scripts[v.script];
                v.set_static(ws, is_static, s);
                ws.vehicles.insert(id, v);
                Ok(1)
            }
            None => Ok(0),
        }
    }

    /// `applyCentralForceToVehicle(id, x, y, z)`.
    pub fn apply_central_force_to_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
    ) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if let Some(v) = ws.vehicles.get(&id) {
            let rb = ws.dw.cw.obj_mut(v.body).rb_mut();
            let f = Vec3::new(x as f64, y as f64, z as f64) * rb.linear_factor;
            rb.total_force += f;
        }
        Ok(())
    }

    /// `applyTorqueToVehicle(id, x, y, z)`: `applyTorque(torque * linearFactor)`.
    pub fn apply_torque_to_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
    ) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if let Some(v) = ws.vehicles.get(&id) {
            let rb = ws.dw.cw.obj_mut(v.body).rb_mut();
            let t = Vec3::new(x as f64, y as f64, z as f64) * rb.linear_factor * rb.angular_factor;
            rb.total_torque += t;
        }
        Ok(())
    }

    /// `teleportVehicle(id, worldX, worldY, physicsZ, qx, qy, qz, qw)`.
    #[allow(clippy::too_many_arguments)]
    pub fn teleport_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        let Some(body) = ws.vehicles.get(&id).map(|v| v.body) else {
            return Ok(());
        };
        ws.dw.remove_rigid_body(body);
        let origin = Vec3::new(
            (x - ws.offset_x as f32) as f64,
            z as f64,
            (y - ws.offset_y as f32) as f64,
        );
        let t = Transform::new(quat_basis(qx, qy, qz, qw), origin);
        let o = ws.dw.cw.obj_mut(body);
        o.set_center_of_mass_transform(&t);
        o.set_angular_velocity(&Vec3::ZERO);
        o.set_linear_velocity(&Vec3::ZERO);
        let mask = if ws.is_server { 13 } else { 15 };
        ws.dw.add_rigid_body_filtered(body, 2, mask);
        Ok(())
    }

    /// `setTireInflation(id, wheel, inflation)`.
    pub fn set_tire_inflation(
        &mut self,
        id: i32,
        wheel: i32,
        inflation: f32,
    ) -> Result<(), JniError> {
        let scripts = &self.scripts;
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if let Some(v) = ws.vehicles.get_mut(&id) {
            if wheel < 0 || wheel >= scripts[v.script].num_wheels {
                return Err(JniError::IllegalArgument("invalid wheel index"));
            }
            v.tires[wheel as usize].inflation = inflation;
        }
        Ok(())
    }

    /// `setTireRemoved(id, wheel, removed)`.
    pub fn set_tire_removed(&mut self, id: i32, wheel: i32, removed: bool) -> Result<(), JniError> {
        let scripts = &self.scripts;
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        let Some(mut v) = ws.vehicles.remove(&id) else {
            return Ok(());
        };
        let s = &scripts[v.script];
        if wheel < 0 || wheel >= s.num_wheels {
            ws.vehicles.insert(id, v);
            return Err(JniError::IllegalArgument("invalid wheel index"));
        }
        let w = wheel as usize;
        v.tires[w].removed = removed;
        let mut r = s.wheels[w].radius;
        let slip = if !removed {
            s.wheel_friction
        } else {
            r *= 0.66;
            0.4
        };
        v.with_rv(ws, |rv, objs| {
            rv.wheel_info[w].wheels_radius = r;
            rv.wheel_info[w].friction_slip = slip;
            rv.update_wheel_transforms_ws(objs, w, false);
            rv.update_wheel_transform(objs, w, false);
        });
        ws.vehicles.insert(id, v);
        Ok(())
    }

    /// `setVehicleParams(id, params)`: 6 floats per wheel (tire present,
    /// inflation, unused, damping relaxation, damping compression,
    /// suspension rest length offset). 0, or -1 when the vehicle is missing.
    pub fn set_vehicle_params(&mut self, id: i32, p: &mut [f32]) -> Result<i32, JniError> {
        let scripts = &self.scripts;
        let ws = world!(self, JniError::Runtime(NULL_INSTANCE));
        let Some(mut v) = ws.vehicles.remove(&id) else {
            return Ok(-1);
        };
        let s = &scripts[v.script];
        if (p.len() as i32) < s.num_wheels * 6 {
            ws.vehicles.insert(id, v);
            return Err(JniError::IllegalArgument("output array is too small"));
        }
        let n = s.num_wheels.max(0) as usize;
        let mut tires = v.tires;
        v.with_rv(ws, |rv, _| {
            for i in 0..n {
                let d = &p[i * 6..i * 6 + 6];
                tires[i].present = 0.1f32 < d[0];
                tires[i].inflation = d[1];
                let wi = &mut rv.wheel_info[i];
                wi.wheels_damping_compression = d[4] as f64;
                wi.wheels_damping_relaxation = d[3] as f64;
                wi.suspension_rest_length1 = s.suspension_rest_length + d[5] as f64;
                let mut r = s.wheels[i].radius;
                let slip = if !tires[i].present {
                    r *= 0.66;
                    0.4
                } else {
                    s.wheel_friction
                };
                wi.wheels_radius = r;
                wi.friction_slip = slip;
            }
        });
        v.tires = tires;
        ws.vehicles.insert(id, v);
        Ok(0)
    }

    /// `setVehicleMass(id, mass)`.
    pub fn set_vehicle_mass(&mut self, id: i32, mass: f32) -> Result<i32, JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        let Some(v) = ws.vehicles.get(&id) else {
            return Ok(-1);
        };
        let inertia = ws
            .dw
            .cw
            .shapes
            .get(v.compound)
            .calculate_local_inertia(mass as f64);
        ws.dw
            .cw
            .obj_mut(v.body)
            .set_mass_props(mass as f64, &inertia);
        Ok(0)
    }

    /// `setVehicleVelocityMultiplier(id, maxSpeed, multiplier)`.
    pub fn set_vehicle_velocity_multiplier(
        &mut self,
        id: i32,
        max_speed: f32,
        multiplier: f32,
    ) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if let Some(v) = ws.vehicles.get_mut(&id) {
            v.max_speed = max_speed as f64;
            v.velocity_multiplier = multiplier as f64;
        }
        Ok(())
    }

    /// `stepSimulation(timeStep, maxSubSteps, fixedTimeStep)`.
    pub fn step_simulation(
        &mut self,
        time_step: f32,
        max_sub_steps: i32,
        fixed: f32,
        cb: &mut dyn JavaCallbacks,
    ) -> Result<(), JniError> {
        let ws = world!(self, JniError::Runtime(NULL_INSTANCE));
        ws.step_simulation(time_step as f64, max_sub_steps, fixed as f64, cb);
        Ok(())
    }

    /// `getVehicleCount()`: also snapshots the vehicle list for
    /// `getVehiclePhysics`.
    pub fn get_vehicle_count(&mut self) -> Result<i32, JniError> {
        let ws = world!(self, JniError::Runtime(NULL_INSTANCE));
        self.hack_vehicles = ws.vehicles.keys().copied().collect();
        Ok(self.hack_vehicles.len() as i32)
    }

    /// `getVehiclePhysics(offset, out)`: per vehicle `id + 0.1, x, y, z,
    /// qx, qy, qz, qw, vx, vy, vz, speedKmHour, isCollide, wheelCount + 0.1`
    /// then `steering, rotation, skidInfo, suspensionLength` per wheel
    /// (positions relative to the world offset). At most `out.len() / 30`
    /// vehicles; returns how many were written.
    pub fn get_vehicle_physics(&mut self, offset: i32, out: &mut [f32]) -> Result<i32, JniError> {
        let ws = world!(self, JniError::Runtime(NULL_INSTANCE));
        if offset < 0 {
            return Err(JniError::IllegalArgument("offset is invalid"));
        }
        let n = self.hack_vehicles.len() as i32;
        if n <= offset {
            return Ok(0);
        }
        if out.len() < 30 {
            return Err(JniError::IllegalArgument("output array is too small"));
        }
        let max = (out.len() / 30) as i32;
        let mut k = 0usize;
        let mut written = 0;
        while written < max {
            let idx = offset + written;
            if n <= idx {
                break;
            }
            let id = self.hack_vehicles[idx as usize];
            if let Some(mut v) = ws.vehicles.remove(&id) {
                let t = ws.dw.cw.obj(v.body).world_transform;
                let lin = ws.dw.cw.obj(v.body).rb().linear_velocity;
                let q = t.basis.get_rotation();
                let rv = v.rv(ws);
                let head = [
                    id as f32 + 0.1f32,
                    t.origin.x as f32,
                    t.origin.y as f32,
                    t.origin.z as f32,
                    q.x as f32,
                    q.y as f32,
                    q.z as f32,
                    q.w as f32,
                    lin.x as f32,
                    lin.y as f32,
                    lin.z as f32,
                    rv.current_vehicle_speed_km_hour as f32,
                    if v.is_collide { 1.0 } else { 0.0 },
                    rv.wheel_info.len() as f32 + 0.1f32,
                ];
                out[k..k + 14].copy_from_slice(&head);
                k += 14;
                for w in &rv.wheel_info {
                    out[k] = w.steering as f32;
                    out[k + 1] = w.rotation as f32;
                    out[k + 2] = w.skid_info as f32;
                    out[k + 3] = w.raycast_info.suspension_length as f32;
                    k += 4;
                }
                v.is_collide = false;
                ws.vehicles.insert(id, v);
            }
            written += 1;
        }
        Ok(written)
    }

    /// `getOwnVehiclePhysics(id, out)`: `worldX, worldY, z, qx, qy, qz, qw,
    /// vx, vy, vz, wheelCount + 0.1`, then 4 floats per wheel as in
    /// `getVehiclePhysics`. 0, or an exception when the vehicle is missing.
    pub fn get_own_vehicle_physics(&mut self, id: i32, out: &mut [f32]) -> Result<i32, JniError> {
        let ws = world!(self, JniError::Runtime(NULL_INSTANCE));
        let Some(v) = ws.vehicles.get(&id) else {
            return Err(JniError::Runtime("Vehicle not found"));
        };
        let t = ws.dw.cw.obj(v.body).world_transform;
        let lin = ws.dw.cw.obj(v.body).rb().linear_velocity;
        let q = t.basis.get_rotation();
        let rv = v.rv(ws);
        let head = [
            (t.origin.x + ws.offset_x as f64) as f32,
            (t.origin.z + ws.offset_y as f64) as f32,
            t.origin.y as f32,
            q.x as f32,
            q.y as f32,
            q.z as f32,
            q.w as f32,
            lin.x as f32,
            lin.y as f32,
            lin.z as f32,
            rv.wheel_info.len() as f32 + 0.1f32,
        ];
        out[..11].copy_from_slice(&head);
        let mut k = 11;
        for w in &rv.wheel_info {
            out[k] = w.steering as f32;
            out[k + 1] = w.rotation as f32;
            out[k + 2] = w.skid_info as f32;
            out[k + 3] = w.raycast_info.suspension_length as f32;
            k += 4;
        }
        Ok(0)
    }

    /// `setOwnVehiclePhysics(id, params, sweep)`: the inverse of
    /// `getOwnVehiclePhysics`. With `sweep` the move is swept against the
    /// world first and stops at the first hit. Returns the number of
    /// vehicles, -4 when the vehicle is missing, -5 when the wheel count
    /// differs (the transform and velocity are set anyway).
    pub fn set_own_vehicle_physics(
        &mut self,
        id: i32,
        p: &mut [f32],
        sweep: bool,
    ) -> Result<i32, JniError> {
        let ws = world!(self, JniError::Runtime(NULL_INSTANCE));
        let Some(v) = ws.vehicles.get(&id) else {
            return Ok(-4);
        };
        let (body, compound) = (v.body, v.compound);
        let fx = p[0] - ws.offset_x as f32;
        let fz = p[1] - ws.offset_y as f32;
        let basis = quat_basis(p[3], p[4], p[5], p[6]);
        let mut target = Vec3::new(fx as f64, p[2] as f64, fz as f64);
        if sweep {
            let cur = ws.dw.cw.obj(body).world_transform;
            let nchildren = ws.dw.cw.shapes.get(compound).compound().children.len();
            if 0.0 < (target - cur.origin).length() && nchildren > 0 {
                let mut frac = 1.0f64;
                for i in 0..nchildren {
                    let (child_t, child_s) = {
                        let c = &ws.dw.cw.shapes.get(compound).compound().children[i];
                        (c.transform, c.child_shape)
                    };
                    let m = ws.dw.cw.shapes.get(child_s).get_margin();
                    ws.dw.cw.shapes.get_mut(child_s).set_margin(m + 0.1);
                    let from = cur * child_t;
                    let to = Transform::new(cur.basis, target) * child_t;
                    let mut cb =
                        crate::pz::sweep::PzVehicleClosestConvexResultCallback::new(compound);
                    let shape = ws.dw.cw.shapes.get(child_s).clone();
                    ws.dw.cw.convex_sweep_test(&shape, &from, &to, &mut cb, 0.0);
                    let hit = cb.closest_hit_fraction();
                    let better = hit < frac && hit < 1.0;
                    if better {
                        frac = hit;
                    }
                    ws.dw.cw.shapes.get_mut(child_s).set_margin(m);
                }
                if frac < 1.0 {
                    target = Vec3::new(
                        cur.origin.x + frac * (target.x - cur.origin.x),
                        cur.origin.y + frac * (target.y - cur.origin.y),
                        cur.origin.z + frac * (target.z - cur.origin.z),
                    );
                }
            }
        }
        let t = Transform::new(basis, target);
        let o = ws.dw.cw.obj_mut(body);
        if let Some(ms) = o.rb_mut().optional_motion_state.as_mut() {
            ms.set_world_transform(&t);
        }
        o.set_center_of_mass_transform(&t);
        o.set_linear_velocity(&Vec3::new(p[7] as f64, p[8] as f64, p[9] as f64));
        let mut v = ws.vehicles.remove(&id).unwrap();
        let ok = v.rv(ws).wheel_info.len() as i32 == p[10] as i32;
        if ok {
            let n = p[10] as i32;
            v.with_rv(ws, |rv, _| {
                for i in 0..n.max(0) as usize {
                    let w = &mut rv.wheel_info[i];
                    w.steering = p[11 + 4 * i] as f64;
                    w.rotation = p[12 + 4 * i] as f64;
                    w.skid_info = p[13 + 4 * i] as f64;
                    w.raycast_info.suspension_length = p[14 + 4 * i] as f64;
                }
            });
        }
        ws.vehicles.insert(id, v);
        Ok(if ok { ws.vehicles.len() as i32 } else { -5 })
    }

    /// `getObjectPhysics(out)`: `id, x, y, z` per physics object; returns
    /// how many.
    pub fn get_object_physics(&mut self, out: &mut [f32]) -> Result<i32, JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        let mut k = 0;
        for (&id, &b) in &ws.physics_objects {
            let o = ws.dw.cw.obj(b).world_transform.origin;
            out[k] = id as f32;
            out[k + 1] = o.x as f32;
            out[k + 2] = o.y as f32;
            out[k + 3] = o.z as f32;
            k += 4;
        }
        Ok(ws.physics_objects.len() as i32)
    }

    /// `createServerCell(cellX, cellY)`.
    pub fn create_server_cell(&mut self, x: i32, y: i32) -> Result<(), JniError> {
        let ws = world!(self, JniError::Runtime(NULL_INSTANCE));
        ws.create_server_cell(x, y);
        Ok(())
    }

    /// `removeServerCell(cellX, cellY)`.
    pub fn remove_server_cell(&mut self, x: i32, y: i32) -> Result<(), JniError> {
        let ws = world!(self, JniError::Runtime(NULL_INSTANCE));
        ws.remove_server_cell(x, y);
        Ok(())
    }

    /// `addPhysicsObject(worldX, worldY)`: returns the object id.
    pub fn add_physics_object(&mut self, x: f32, y: f32) -> Result<i32, JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        Ok(ws.add_physics_object(x, y))
    }

    fn two_vehicles(ws: &WorldSimulation, a: i32, b: i32) -> Option<(usize, usize)> {
        let va = ws.vehicles.get(&a)?.body;
        let vb = ws.vehicles.get(&b)?.body;
        Some((va, vb))
    }

    /// `addHingeConstraint(idA, idB, pivotA xyz, pivotB xyz)`: hinge about
    /// y; returns the constraint id or -1.
    #[allow(clippy::too_many_arguments)]
    pub fn add_hinge_constraint(
        &mut self,
        a: i32,
        b: i32,
        ax: f32,
        ay: f32,
        az: f32,
        bx: f32,
        by: f32,
        bz: f32,
    ) -> Result<i32, JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if a == b {
            return Ok(-1);
        }
        let Some((ba, bb)) = Self::two_vehicles(ws, a, b) else {
            return Ok(-1);
        };
        let ta = ws.dw.cw.obj(ba).world_transform;
        let axis = Vec3::new(0.0, 1.0, 0.0);
        let mut c = HingeConstraint::new_typed(
            ba,
            bb,
            &ta,
            Vec3::new(ax as f64, ay as f64, az as f64),
            Vec3::new(bx as f64, by as f64, bz as f64),
            axis,
            axis,
            false,
        );
        c.needs_feedback = true;
        Ok(ws.add_constraint(a, b, c))
    }

    /// `addPointConstraint(idA, idB, pivotA xyz, pivotB xyz)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_point_constraint(
        &mut self,
        a: i32,
        b: i32,
        ax: f32,
        ay: f32,
        az: f32,
        bx: f32,
        by: f32,
        bz: f32,
    ) -> Result<i32, JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if a == b {
            return Ok(-1);
        }
        let Some((ba, bb)) = Self::two_vehicles(ws, a, b) else {
            return Ok(-1);
        };
        let mut c = Point2PointConstraint::new_typed(
            ba,
            bb,
            Vec3::new(ax as f64, ay as f64, az as f64),
            Vec3::new(bx as f64, by as f64, bz as f64),
        );
        c.needs_feedback = true;
        Ok(ws.add_constraint(a, b, c))
    }

    fn sixdof(
        ws: &WorldSimulation,
        ba: usize,
        bb: usize,
        pa: Vec3,
        pb: Vec3,
    ) -> crate::bt::dynamics::typed_constraint::TypedConstraint {
        Generic6DofConstraint::new_typed(
            ba,
            bb,
            ws.dw.cw.obj(ba),
            ws.dw.cw.obj(bb),
            Transform::new(Mat3::IDENTITY, pa),
            Transform::new(Mat3::IDENTITY, pb),
            true,
        )
    }

    /// `add6DofConstraint(idA, idB, pivotA xyz, pivotB xyz, linear lower
    /// xyz, linear upper xyz, angular lower xyz, angular upper xyz)`
    /// (`p` holds the 18 floats); angular limits are normalised to
    /// [-pi, pi].
    pub fn add_6dof_constraint(&mut self, a: i32, b: i32, p: &[f32; 18]) -> Result<i32, JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if a == b {
            return Ok(-1);
        }
        let Some((ba, bb)) = Self::two_vehicles(ws, a, b) else {
            return Ok(-1);
        };
        let f = |i: usize| p[i] as f64;
        let mut c = Self::sixdof(
            ws,
            ba,
            bb,
            Vec3::new(f(0), f(1), f(2)),
            Vec3::new(f(3), f(4), f(5)),
        );
        if let ConstraintKind::Generic6Dof(g) = &mut c.kind {
            g.linear_limits.lower_limit = Vec3::new(f(6), f(7), f(8));
            g.linear_limits.upper_limit = Vec3::new(f(9), f(10), f(11));
            for k in 0..3 {
                g.angular_limits[k].lo_limit = bt_normalize_angle(f(12 + k));
            }
            for k in 0..3 {
                g.angular_limits[k].hi_limit = bt_normalize_angle(f(15 + k));
            }
        }
        c.needs_feedback = true;
        Ok(ws.add_constraint(a, b, c))
    }

    /// `addRopeConstraint(idA, idB, pivotA xyz, pivotB xyz, maxDistance)`:
    /// a 6-DOF constraint with linear limits ±maxDistance; both vehicles
    /// are made dynamic and active first.
    #[allow(clippy::too_many_arguments)]
    pub fn add_rope_constraint(
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
    ) -> Result<i32, JniError> {
        let scripts = &self.scripts;
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        if a == b {
            return Err(JniError::IllegalArgument("the same vehicle"));
        }
        if !ws.vehicles.contains_key(&a) {
            return Err(JniError::NullPointer("vehicle A not found"));
        }
        if !ws.vehicles.contains_key(&b) {
            return Err(JniError::NullPointer("vehicle B not found"));
        }
        for id in [a, b] {
            let mut v = ws.vehicles.remove(&id).unwrap();
            let s = &scripts[v.script];
            v.set_static(ws, false, s);
            v.set_active(ws, true);
            ws.vehicles.insert(id, v);
        }
        let (ba, bb) = Self::two_vehicles(ws, a, b).unwrap();
        let mut c = Self::sixdof(
            ws,
            ba,
            bb,
            Vec3::new(ax as f64, ay as f64, az as f64),
            Vec3::new(bx as f64, by as f64, bz as f64),
        );
        if let ConstraintKind::Generic6Dof(g) = &mut c.kind {
            let lo = (-d) as f64;
            let hi = d as f64;
            g.linear_limits.lower_limit = Vec3::new(lo, lo, lo);
            g.linear_limits.upper_limit = Vec3::new(hi, hi, hi);
        }
        c.needs_feedback = true;
        Ok(ws.add_constraint(a, b, c))
    }

    /// `setConstraintERP(constraintId, erp, axis)`.
    pub fn set_constraint_erp(&mut self, c: i32, erp: f32, axis: i32) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        ws.set_constraint_erp(c, erp, axis);
        Ok(())
    }

    /// `removeConstraint(constraintId)`.
    pub fn remove_constraint(&mut self, c: i32) -> Result<(), JniError> {
        let ws = world!(self, JniError::NullPointer(NULL_INSTANCE));
        ws.remove_constraint(c);
        Ok(())
    }

    /// `checkWheelCollision(vehicleId, ragdollId, bodyPart)`: whether a
    /// wheel of the vehicle is within 0.5 of the ragdoll body part.
    pub fn check_wheel_collision(
        &mut self,
        id: i32,
        ragdoll: i32,
        part: i32,
    ) -> Result<bool, JniError> {
        self.check_wheel_collision_ragdoll(id, ragdoll, part)
    }
}

#[allow(dead_code)]
fn _unused(_: Shape) -> f64 {
    LEVEL_SCALE
}
