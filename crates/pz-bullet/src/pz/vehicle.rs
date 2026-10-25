//! `PZVehicle`: a raycast vehicle with a compound chassis built from its
//! script.

use super::script::VehicleScript;
use super::world::{bullet_object, WorldSimulation, LEVEL_HEIGHT};
use crate::bt::collision_object::*;
use crate::bt::dynamics::raycast_vehicle::{RaycastVehicle, VehicleTuning};
use crate::bt::dynamics::rigid_body::RigidBodyConstructionInfo;
use crate::bt::dynamics::world::{ActionId, ActionInterface};
use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;

/// Per-wheel tire state (`+0xc4`, 8 bytes per wheel).
#[derive(Clone, Copy, Debug)]
pub struct Tire {
    /// `+0xc4`: inflation (only stored).
    pub inflation: f32,
    /// `+0xc8`: removed (`setTireRemoved`).
    pub removed: bool,
    /// `+0xc9`: tire present (`setVehicleParams` slot 0 > 0.1).
    pub present: bool,
}

/// `PZVehicle`.
pub struct PzVehicle {
    pub id: i32,
    /// `+0x04`: built without wheels.
    pub is_burnt: bool,
    /// `+0x05`: collided with something since the last `getVehiclePhysics`.
    pub is_collide: bool,
    /// The `btRaycastVehicle`: in the world's action list while the vehicle
    /// is not static, kept here otherwise.
    pub action: Option<ActionId>,
    pub parked: Option<Box<RaycastVehicle>>,
    /// `+0x10`: chassis body.
    pub body: ObjId,
    /// `+0x18`: tuning.
    pub tuning: VehicleTuning,
    /// `+0x48`: the chassis child shapes.
    pub child_shapes: Vec<ShapeId>,
    /// `+0x60`: the compound chassis shape.
    pub compound: ShapeId,
    /// `+0x68`: the script (index into the process-wide list).
    pub script: usize,
    /// `+0x70`: speed above which the linear velocity is multiplied.
    pub max_speed: f64,
    /// `+0x78`.
    pub velocity_multiplier: f64,
    /// `+0x80`: linear velocity at the previous substep.
    pub last_linear_velocity: Vec3,
    /// `+0xa0`: angular velocity at the previous substep.
    pub last_angular_velocity: Vec3,
    /// `+0xc0`.
    pub is_static: bool,
    /// `+0xc1`.
    pub is_active: bool,
    pub tires: [Tire; 8],
}

/// `btQuaternion::setEuler(yaw, pitch, roll)`.
pub fn quat_euler(yaw: f64, pitch: f64, roll: f64) -> Quat {
    let (sy, cy) = bt_sincos(yaw * 0.5);
    let (sp, cp) = bt_sincos(pitch * 0.5);
    let (sr, cr) = bt_sincos(roll * 0.5);
    Quat::new(
        fma(cr * sp, cy, (sr * cp) * sy),
        fma(cr * cp, sy, -((sr * sp) * cy)),
        fma(sr * cp, cy, -((cr * sp) * sy)),
        fma(cr * cp, cy, (sr * sp) * sy),
    )
}

fn deg(x: f64) -> f64 {
    (x * std::f64::consts::PI) / 180.0
}

impl PzVehicle {
    /// `PZVehicle(id, isBurnt, script)`.
    pub fn new(
        ws: &mut WorldSimulation,
        id: i32,
        is_burnt: bool,
        script_index: usize,
        script: &VehicleScript,
    ) -> PzVehicle {
        let shapes = &mut ws.dw.cw.shapes;
        let compound = shapes.add(Shape::new_compound(true));
        let mut child_shapes = Vec::new();
        for s in 0..script.num_shapes.max(0) as usize {
            let sh = &script.shapes[s];
            let mut t = Transform::IDENTITY;
            t.origin = sh.offset;
            let mut shape = None;
            if sh.kind == 1 {
                let he = Vec3::new(sh.extents.x * 0.5, sh.extents.y * 0.5, sh.extents.z * 0.5);
                shape = Some(Shape::new_box(&he));
                let q = quat_euler(deg(sh.rotate.y), deg(sh.rotate.x), deg(sh.rotate.z));
                t.basis.set_rotation(&q);
            }
            if sh.kind == 2 {
                shape = Some(Shape::new_sphere(sh.radius));
            }
            if sh.kind == 3 {
                shape = Some(Shape::new_convex_hull(&sh.points, sh.points.len() / 3, 3));
            }
            if let Some(shape) = shape {
                let id = shapes.add(shape);
                shapes.compound_add_child_shape(compound, &t, id);
                child_shapes.push(id);
            }
        }
        // localCreateRigidBody(mass)
        let mass = script.mass;
        let inertia = shapes.get(compound).calculate_local_inertia(mass);
        let ms = DefaultMotionState::new(Transform::IDENTITY);
        let mut info = RigidBodyConstructionInfo::new(mass, Some(ms), Some(compound), inertia);
        info.friction = 4.0;
        info.restitution = 0.2;
        let mut body = CollisionObject::new_rigid_body(&info);
        body.collision_flags |= CF_CUSTOM_MATERIAL_CALLBACK;
        body.user_index = ((id as i64) << 8) | bullet_object::VEHICLE;
        let body = ws.dw.cw.insert_object(body);
        let tuning = VehicleTuning::default();
        let mut v = PzVehicle {
            id,
            is_burnt,
            is_collide: false,
            action: None,
            parked: None,
            body,
            tuning,
            child_shapes,
            compound,
            script: script_index,
            max_speed: 100_000.0,
            velocity_multiplier: 1.0,
            last_linear_velocity: Vec3::ZERO,
            last_angular_velocity: Vec3::ZERO,
            is_static: false,
            is_active: false,
            tires: [Tire {
                inflation: 0.0,
                removed: false,
                present: false,
            }; 8],
        };
        v.reset(ws);
        let mut rv = RaycastVehicle::new(&v.tuning, body);
        rv.set_coordinate_system(0, 1, 2);
        let dir = Vec3::new(0.0, -1.0, 0.0);
        let axle = Vec3::new(-1.0, 0.0, 0.0);
        for w in 0..script.num_wheels.max(0) as usize {
            let sw = &script.wheels[w];
            let i = rv.add_wheel(
                &ws.dw.cw.objects,
                &sw.offset,
                &dir,
                &axle,
                script.suspension_rest_length,
                sw.radius,
                &v.tuning,
                sw.front,
            );
            let wi = &mut rv.wheel_info[i];
            wi.max_suspension_travel_cm = script.max_suspension_travel_cm;
            wi.wheels_damping_relaxation = script.suspension_damping;
            wi.wheels_damping_compression = script.suspension_compression;
            wi.suspension_stiffness = script.suspension_stiffness;
            wi.friction_slip = script.wheel_friction;
            wi.roll_influence = script.roll_influence;
            if !sw.front {
                rv.set_brake(10000.0, i);
            }
            v.tires[w] = Tire {
                inflation: 1.0,
                removed: false,
                present: true,
            };
        }
        v.parked = Some(Box::new(rv));
        v
    }

    /// The `btRaycastVehicle`, wherever it is.
    pub fn rv<'a>(&'a self, ws: &'a WorldSimulation) -> &'a RaycastVehicle {
        match &self.parked {
            Some(rv) => rv,
            None => ws
                .dw
                .action(self.action.expect("vehicle action"))
                .and_then(|a| a.as_any().downcast_ref::<RaycastVehicle>())
                .expect("raycast vehicle"),
        }
    }

    /// Runs `f` on the `btRaycastVehicle` and the world's objects.
    pub fn with_rv<R>(
        &mut self,
        ws: &mut WorldSimulation,
        f: impl FnOnce(&mut RaycastVehicle, &mut crate::bt::dispatch::ObjArena) -> R,
    ) -> R {
        if let Some(rv) = self.parked.as_deref_mut() {
            return f(rv, &mut ws.dw.cw.objects);
        }
        let id = self.action.expect("vehicle action");
        let i = ws
            .dw
            .actions
            .iter()
            .position(|(a, _)| *a == id)
            .expect("action");
        let (_, mut a) = std::mem::replace(&mut ws.dw.actions[i], (id, Box::new(Dummy)));
        let r = f(
            a.as_any_mut()
                .downcast_mut::<RaycastVehicle>()
                .expect("raycast vehicle"),
            &mut ws.dw.cw.objects,
        );
        ws.dw.actions[i] = (id, a);
        r
    }

    fn add_action(&mut self, ws: &mut WorldSimulation) {
        let rv = self.parked.take().expect("parked vehicle");
        self.action = Some(ws.dw.add_action(rv));
    }
    fn remove_action(&mut self, ws: &mut WorldSimulation) {
        let a = ws
            .dw
            .remove_action(self.action.take().expect("vehicle action"))
            .expect("action");
        let rv: Box<dyn std::any::Any> = a;
        self.parked = Some(rv.downcast::<RaycastVehicle>().expect("raycast vehicle"));
    }

    /// `PZVehicle::reset`.
    pub fn reset(&mut self, ws: &mut WorldSimulation) {
        let o = ws.dw.cw.obj_mut(self.body);
        o.set_center_of_mass_transform(&Transform::IDENTITY);
        o.set_linear_velocity(&Vec3::ZERO);
        o.set_angular_velocity(&Vec3::ZERO);
        if let Some(bp) = ws.dw.cw.obj(self.body).broadphase_handle {
            let cw = &mut ws.dw.cw;
            cw.broadphase
                .paircache
                .clean_proxy_from_pairs(bp, Some(&mut cw.dispatcher));
        }
        if self.parked.is_some() || self.action.is_some() {
            self.with_rv(ws, |rv, objs| {
                rv.reset_suspension();
                for i in 0..rv.wheel_info.len() {
                    rv.update_wheel_transform(objs, i, true);
                }
            });
        }
    }

    /// `PZVehicle::addToWorld(transform)`.
    pub fn add_to_world(mut self, ws: &mut WorldSimulation, t: &Transform) -> PzVehicle {
        let o = ws.dw.cw.obj_mut(self.body);
        o.update_revision += 1;
        o.world_transform = *t;
        let mask = if ws.is_server { 13 } else { 15 };
        ws.dw.add_rigid_body_filtered(self.body, 2, mask);
        self.add_action(ws);
        self
    }

    /// `PZVehicle::removeFromWorld` (and the destructor).
    pub fn remove_from_world(mut self, ws: &mut WorldSimulation) {
        ws.dw.cw.obj_mut(self.body).user_index = 0;
        if self.action.is_some() {
            self.remove_action(ws);
        }
        ws.dw.remove_rigid_body(self.body);
        ws.dw.cw.delete_object(self.body);
        ws.dw.cw.shapes.remove(self.compound);
        for s in self.child_shapes.drain(..) {
            ws.dw.cw.shapes.remove(s);
        }
    }

    /// `PZVehicle::hasChunksAllAround`: every valid chunk of the 3×3 around
    /// the vehicle is in a chunk map with the vehicle's level built (or
    /// outside the chunk's levels).
    pub fn has_chunks_all_around(&self, ws: &WorldSimulation) -> bool {
        let o = ws.dw.cw.obj(self.body).world_transform.origin;
        let cx = ((o.x + ws.offset_x as f64) * 0.125).floor() as i32;
        let cy = ((o.z + ws.offset_y as f64) * 0.125).floor() as i32;
        let level = (o.y / LEVEL_HEIGHT).floor() as i32;
        for (dx, dy) in [
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (0, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ] {
            let (x, y) = (cx + dx, cy + dy);
            if !ws.is_valid_chunk(x, y) {
                continue;
            }
            let Some(c) = ws.get_chunk_for_any_player(x, y) else {
                return false;
            };
            let c = ws.chunk(c);
            if level < c.min_level || c.max_level < level {
                continue;
            }
            match c.level(level) {
                Some(l) if l.dirty && l.has_bodies => {}
                _ => return false,
            }
        }
        true
    }

    /// `PZVehicle::setStatic(isStatic)`.
    pub fn set_static(
        &mut self,
        ws: &mut WorldSimulation,
        is_static: bool,
        script: &VehicleScript,
    ) {
        if self.is_static == is_static {
            return;
        }
        if !self.is_static {
            self.remove_action(ws);
        }
        ws.dw.remove_rigid_body(self.body);
        if is_static {
            let inertia = ws
                .dw
                .cw
                .shapes
                .get(self.compound)
                .calculate_local_inertia(0.0);
            let around = self.has_chunks_all_around(ws);
            let o = ws.dw.cw.obj_mut(self.body);
            o.set_mass_props(0.0, &inertia);
            o.collision_flags |= CF_KINEMATIC_OBJECT;
            o.set_activation_state(if around {
                DISABLE_DEACTIVATION
            } else {
                DISABLE_SIMULATION
            });
            let wt = o.world_transform;
            if let Some(ms) = o.rb_mut().optional_motion_state.as_mut() {
                ms.set_world_transform(&wt);
            }
        } else {
            let inertia = ws
                .dw
                .cw
                .shapes
                .get(self.compound)
                .calculate_local_inertia(script.mass);
            let around = self.has_chunks_all_around(ws);
            let o = ws.dw.cw.obj_mut(self.body);
            o.set_mass_props(script.mass, &inertia);
            o.collision_flags &= !CF_KINEMATIC_OBJECT;
            if around {
                o.force_activation_state(ACTIVE_TAG);
            } else {
                o.set_activation_state(DISABLE_SIMULATION);
            }
        }
        let o = ws.dw.cw.obj_mut(self.body);
        o.set_angular_velocity(&Vec3::ZERO);
        o.set_linear_velocity(&Vec3::ZERO);
        let rb = o.rb_mut();
        rb.total_torque = Vec3::ZERO;
        rb.total_force = Vec3::ZERO;
        self.is_static = is_static;
        let mask = if ws.is_server { 13 } else { 15 };
        ws.dw.add_rigid_body_filtered(self.body, 2, mask);
        if !is_static {
            self.add_action(ws);
        }
    }

    /// `PZVehicle::setActive(active)`.
    pub fn set_active(&mut self, ws: &mut WorldSimulation, active: bool) {
        if !active {
            self.is_active = false;
            ws.dw
                .cw
                .obj_mut(self.body)
                .force_activation_state(ACTIVE_TAG);
            return;
        }
        if self.is_static {
            return;
        }
        self.is_active = true;
        ws.dw
            .cw
            .obj_mut(self.body)
            .force_activation_state(DISABLE_DEACTIVATION);
    }

    /// `PZVehicle::control(engineForce, brakingForce, steering)`.
    pub fn control(
        &mut self,
        ws: &mut WorldSimulation,
        engine: f32,
        brake: f32,
        steering: f32,
        script: &VehicleScript,
    ) {
        if self.is_burnt {
            return;
        }
        let stopping = if engine == 0.0 {
            script.stopping_movement_force
        } else {
            0.0
        };
        let max_speed = self.max_speed;
        let tires = self.tires;
        let fronts: Vec<bool> = script.wheels.iter().map(|w| w.front).collect();
        self.with_rv(ws, |rv, _| {
            let r = rv.current_vehicle_speed_km_hour.abs() / max_speed;
            let c = if r > 1.0 { 1.0 } else { r };
            let speed = if r < 0.0 { 0.0 } else { c };
            for i in 0..rv.wheel_info.len() {
                let t = tires[i];
                let extra = if t.inflation < 1.0 {
                    speed * ((1.0 - t.inflation as f64) * 300.0)
                } else {
                    0.0
                };
                let mut b = stopping + 10.0;
                if fronts[i] {
                    rv.set_steering_value(steering as f64, i);
                    rv.apply_engine_force(0.0, i);
                    if t.present {
                        b = stopping + extra;
                    }
                } else if !t.present {
                    rv.apply_engine_force(0.0, i);
                } else {
                    rv.apply_engine_force(engine as f64, i);
                    b = stopping + brake as f64 + extra;
                }
                rv.set_brake(b, i);
            }
        });
    }
}

/// Placeholder while an action is borrowed out of the world.
struct Dummy;
impl ActionInterface for Dummy {
    fn update_action(
        &mut self,
        _world: &mut crate::bt::dynamics::world::DiscreteDynamicsWorld,
        _dt: f64,
    ) {
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
