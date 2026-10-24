// Copyright (c) 2005 Erwin Coumans http://continuousphysics.com/Bullet/
//
// Permission to use, copy, modify, distribute and sell this software
// and its documentation for any purpose is hereby granted without fee,
// provided that the above copyright notice appear in all copies.
// Erwin Coumans makes no representations about the suitability
// of this software for any purpose.
// It is provided "as is" without express or implied warranty.
//
// Bullet Continuous Collision Detection and Physics Library (btWheelInfo,
// btVehicleRaycaster, resolveSingleBilateral, btJacobianEntry)
// Copyright (c) 2003-2006 Erwin Coumans  http://continuousphysics.com/Bullet/
// zlib license, see LICENSE-ZLIB.
//
// Altered for pz-rs: Rust translation of btRaycastVehicle, btWheelInfo,
// btVehicleRaycaster, btWheelContactPoint, calcRollingFriction and
// resolveSingleBilateral from Bullet 2.82 (double precision).

//! `btRaycastVehicle`.
//!
//! The game's copy is stock 2.82 (`ROLLING_INFLUENCE_FIX` on). As in the
//! original, every wheel that hits something uses
//! `btActionInterface::getFixedBody()` as its ground object, never the body
//! the ray hit: the hit object only decides *whether* there is contact.
//!
//! The chassis lives in the collision-object arena; every method that
//! touches it takes the arena (`BodyArena`) and reads `self.chassis`.
//!
//! # `btWheelInfo` field offsets (arm64, double precision)
//!
//! | offset | field | | offset | field |
//! |---|---|---|---|---|
//! | 0x00 | `raycast_info.contact_normal_ws` | | 0x198 | `suspension_rest_length1` |
//! | 0x20 | `raycast_info.contact_point_ws` | | 0x1a0 | `max_suspension_travel_cm` |
//! | 0x40 | `raycast_info.suspension_length` | | 0x1a8 | `wheels_radius` |
//! | 0x48 | `raycast_info.hard_point_ws` | | 0x1b0 | `suspension_stiffness` |
//! | 0x68 | `raycast_info.wheel_direction_ws` | | 0x1b8 | `wheels_damping_compression` |
//! | 0x88 | `raycast_info.wheel_axle_ws` | | 0x1c0 | `wheels_damping_relaxation` |
//! | 0xa8 | `raycast_info.is_in_contact` | | 0x1c8 | `friction_slip` |
//! | 0xb0 | `raycast_info.ground_object` | | 0x1d0 | `steering` |
//! | 0xb8 | `world_transform` | | 0x1d8 | `rotation` |
//! | 0x138 | `chassis_connection_point_cs` | | 0x1e0 | `delta_rotation` |
//! | 0x158 | `wheel_direction_cs` | | 0x1e8 | `roll_influence` |
//! | 0x178 | `wheel_axle_cs` | | 0x1f0 | `max_suspension_force` |
//! | | | | 0x1f8 | `engine_force` |
//! | | | | 0x200 | `brake` |
//! | | | | 0x208 | `b_is_front_wheel` |
//! | | | | 0x218 | `clipped_inv_contact_dot_suspension` |
//! | | | | 0x220 | `suspension_relative_velocity` |
//! | | | | 0x228 | `wheels_suspension_force` |
//! | | | | 0x230 | `skid_info` |
//!
//! (The offsets the game glue uses: 0x40, 0x198, 0x1a8, 0x1b8, 0x1c0, 0x1c8,
//! 0x1d0, 0x1d8 and 0x230; the rest follow from the declaration order.)
//!
//! `btRaycastVehicle` fields the glue reads: `m_currentVehicleSpeedKmHour`
//! (0xb8) is [`RaycastVehicle::current_vehicle_speed_km_hour`], the wheel count
//! (0xdc, `m_wheelInfo.size()`) is `wheel_info.len()`.

use crate::bt::collision_object::{CollisionObject, ObjId};
use crate::bt::dynamics::rigid_body::RigidBodyConstructionInfo;
use crate::bt::dynamics::typed_constraint::BodyArena;
use crate::bt::linear_math::*;

/// `btVehicleRaycaster::btVehicleRaycasterResult`.
#[derive(Clone, Copy, Debug, Default)]
pub struct VehicleRaycasterResult {
    pub hit_point_in_world: Vec3,
    pub hit_normal_in_world: Vec3,
    pub dist_fraction: f64,
    /// The object the ray hit (the `void*` the C++ returns).
    pub hit_object: ObjId,
}

/// `btVehicleRaycaster`: `castRay` returns `None` for the null pointer.
pub trait VehicleRaycaster {
    fn cast_ray(&mut self, from: &Vec3, to: &Vec3) -> Option<VehicleRaycasterResult>;
}

/// `btRaycastVehicle::btVehicleTuning`.
#[derive(Clone, Copy, Debug)]
pub struct VehicleTuning {
    pub suspension_stiffness: f64,
    pub suspension_compression: f64,
    pub suspension_damping: f64,
    pub max_suspension_travel_cm: f64,
    pub friction_slip: f64,
    pub max_suspension_force: f64,
}

impl Default for VehicleTuning {
    /// `btVehicleTuning()`.
    fn default() -> Self {
        VehicleTuning {
            suspension_stiffness: 5.88,
            suspension_compression: 0.83,
            suspension_damping: 0.88,
            max_suspension_travel_cm: 500.0,
            friction_slip: 10.5,
            max_suspension_force: 6000.0,
        }
    }
}

/// `btWheelInfo::RaycastInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RaycastInfo {
    pub contact_normal_ws: Vec3,
    pub contact_point_ws: Vec3,
    pub suspension_length: f64,
    pub hard_point_ws: Vec3,
    pub wheel_direction_ws: Vec3,
    pub wheel_axle_ws: Vec3,
    pub is_in_contact: bool,
    /// `m_groundObject`: `true` when it points at the fixed body (the only
    /// value 2.82 ever stores besides null).
    pub ground_object: bool,
}

/// `btWheelInfoConstructionInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub struct WheelInfoConstructionInfo {
    pub chassis_connection_cs: Vec3,
    pub wheel_direction_cs: Vec3,
    pub wheel_axle_cs: Vec3,
    pub suspension_rest_length: f64,
    pub max_suspension_travel_cm: f64,
    pub wheel_radius: f64,
    pub suspension_stiffness: f64,
    pub wheels_damping_compression: f64,
    pub wheels_damping_relaxation: f64,
    pub friction_slip: f64,
    pub max_suspension_force: f64,
    pub b_is_front_wheel: bool,
}

/// `btWheelInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub struct WheelInfo {
    pub raycast_info: RaycastInfo,
    pub world_transform: Transform,
    pub chassis_connection_point_cs: Vec3,
    pub wheel_direction_cs: Vec3,
    pub wheel_axle_cs: Vec3,
    pub suspension_rest_length1: f64,
    pub max_suspension_travel_cm: f64,
    pub wheels_radius: f64,
    pub suspension_stiffness: f64,
    pub wheels_damping_compression: f64,
    pub wheels_damping_relaxation: f64,
    pub friction_slip: f64,
    pub steering: f64,
    pub rotation: f64,
    pub delta_rotation: f64,
    pub roll_influence: f64,
    pub max_suspension_force: f64,
    pub engine_force: f64,
    pub brake: f64,
    pub b_is_front_wheel: bool,
    pub clipped_inv_contact_dot_suspension: f64,
    pub suspension_relative_velocity: f64,
    pub wheels_suspension_force: f64,
    pub skid_info: f64,
}

impl WheelInfo {
    /// `btWheelInfo(ci)`. Members the C++ constructor leaves uninitialised
    /// (`m_raycastInfo`, `m_worldTransform`, the suspension outputs) are
    /// zero here; `addWheel` sets what is read before being written.
    pub fn new(ci: &WheelInfoConstructionInfo) -> WheelInfo {
        WheelInfo {
            suspension_rest_length1: ci.suspension_rest_length,
            max_suspension_travel_cm: ci.max_suspension_travel_cm,
            wheels_radius: ci.wheel_radius,
            suspension_stiffness: ci.suspension_stiffness,
            wheels_damping_compression: ci.wheels_damping_compression,
            wheels_damping_relaxation: ci.wheels_damping_relaxation,
            chassis_connection_point_cs: ci.chassis_connection_cs,
            wheel_direction_cs: ci.wheel_direction_cs,
            wheel_axle_cs: ci.wheel_axle_cs,
            friction_slip: ci.friction_slip,
            steering: 0.0,
            engine_force: 0.0,
            rotation: 0.0,
            delta_rotation: 0.0,
            brake: 0.0,
            roll_influence: 0.1,
            b_is_front_wheel: ci.b_is_front_wheel,
            max_suspension_force: ci.max_suspension_force,
            ..Default::default()
        }
    }
    #[inline(always)]
    pub fn get_suspension_rest_length(&self) -> f64 {
        self.suspension_rest_length1
    }
}

thread_local! {
    /// `btActionInterface::getFixedBody()::s_fixed`, created on first use
    /// like the function-local static.
    static FIXED_BODY: std::cell::OnceCell<CollisionObject> = const { std::cell::OnceCell::new() };
}

/// `btActionInterface::getFixedBody()`: `btRigidBody(0, 0, 0)` followed by
/// `setMassProps(0, (0,0,0))`.
pub fn fixed_body() -> CollisionObject {
    FIXED_BODY.with(|c| {
        c.get_or_init(|| {
            let mut b = CollisionObject::new_rigid_body(&RigidBodyConstructionInfo::new(
                0.0,
                None,
                None,
                Vec3::ZERO,
            ));
            b.set_mass_props(0.0, &Vec3::ZERO);
            b
        })
        .clone()
    })
}

/// `btJacobianEntry(world2A, world2B, rel_pos1, rel_pos2, jointAxis,
/// inertiaInvA, massInvA, inertiaInvB, massInvB).getDiagonal()`.
#[allow(clippy::too_many_arguments)]
fn jacobian_diagonal(
    world2a: &Mat3,
    world2b: &Mat3,
    rel_pos1: &Vec3,
    rel_pos2: &Vec3,
    joint_axis: &Vec3,
    inertia_inv_a: &Vec3,
    mass_inv_a: f64,
    inertia_inv_b: &Vec3,
    mass_inv_b: f64,
) -> f64 {
    let a_j = *world2a * rel_pos1.cross(joint_axis);
    let b_j = *world2b * rel_pos2.cross(&-*joint_axis);
    let m0 = *inertia_inv_a * a_j;
    let m1 = *inertia_inv_b * b_j;
    mass_inv_a + m0.dot(&a_j) + mass_inv_b + m1.dot(&b_j)
}

/// `resolveSingleBilateral(body1, pos1, body2, pos2, distance, normal,
/// impulse, timeStep)`: returns the impulse.
pub fn resolve_single_bilateral(
    body1: &CollisionObject,
    pos1: &Vec3,
    body2: &CollisionObject,
    pos2: &Vec3,
    normal: &Vec3,
) -> f64 {
    let normal_len_sqr = normal.length2();
    if normal_len_sqr > 1.1 {
        return 0.0;
    }
    let rel_pos1 = *pos1 - body1.world_transform.origin;
    let rel_pos2 = *pos2 - body2.world_transform.origin;
    let vel1 = body1.get_velocity_in_local_point(&rel_pos1);
    let vel2 = body2.get_velocity_in_local_point(&rel_pos2);
    let vel = vel1 - vel2;
    let jac_diag_ab = jacobian_diagonal(
        &body1.world_transform.basis.transpose(),
        &body2.world_transform.basis.transpose(),
        &rel_pos1,
        &rel_pos2,
        normal,
        &body1.rb().inv_inertia_local,
        body1.rb().inverse_mass,
        &body2.rb().inv_inertia_local,
        body2.rb().inverse_mass,
    );
    let jac_diag_ab_inv = 1.0 / jac_diag_ab;
    // (jac.getRelativeVelocity(...) is computed and discarded in the original.)
    let rel_vel = normal.dot(&vel);
    let contact_damping = 0.2;
    -contact_damping * rel_vel * jac_diag_ab_inv
}

/// `btWheelContactPoint` + `calcRollingFriction`.
fn calc_rolling_friction(
    body0: &CollisionObject,
    body1: &CollisionObject,
    friction_pos_world: &Vec3,
    friction_direction_world: &Vec3,
    max_impulse: f64,
) -> f64 {
    let denom0 = body0.compute_impulse_denominator(friction_pos_world, friction_direction_world);
    let denom1 = body1.compute_impulse_denominator(friction_pos_world, friction_direction_world);
    let relaxation = 1.0;
    let jac_diag_ab_inv = relaxation / (denom0 + denom1);

    let rel_pos1 = *friction_pos_world - body0.world_transform.origin;
    let rel_pos2 = *friction_pos_world - body1.world_transform.origin;
    let vel1 = body0.get_velocity_in_local_point(&rel_pos1);
    let vel2 = body1.get_velocity_in_local_point(&rel_pos2);
    let vel = vel1 - vel2;
    let vrel = friction_direction_world.dot(&vel);
    let mut j1 = -vrel * jac_diag_ab_inv;
    bt_set_min(&mut j1, max_impulse);
    bt_set_max(&mut j1, -max_impulse);
    j1
}

/// `ROLLING_INFLUENCE_FIX` (defined in 2.82 and in the game's build).
pub const ROLLING_INFLUENCE_FIX: bool = true;

/// `sideFrictionStiffness2`.
pub const SIDE_FRICTION_STIFFNESS2: f64 = 1.0;

/// `btRaycastVehicle`.
#[derive(Clone, Debug)]
pub struct RaycastVehicle {
    pub forward_ws: Vec<Vec3>,
    pub axle: Vec<Vec3>,
    pub forward_impulse: Vec<f64>,
    pub side_impulse: Vec<f64>,
    pub user_constraint_type: i32,
    pub user_constraint_id: i32,
    pub tau: f64,
    pub damping: f64,
    pub pitch_control: f64,
    pub steering_value: f64,
    pub current_vehicle_speed_km_hour: f64,
    /// `m_chassisBody`.
    pub chassis: ObjId,
    pub index_right_axis: usize,
    pub index_up_axis: usize,
    pub index_forward_axis: usize,
    pub wheel_info: Vec<WheelInfo>,
}

impl RaycastVehicle {
    /// `btRaycastVehicle(tuning, chassis, raycaster)` (the raycaster is passed
    /// to [`RaycastVehicle::update_vehicle`] instead of being stored).
    pub fn new(_tuning: &VehicleTuning, chassis: ObjId) -> RaycastVehicle {
        RaycastVehicle {
            forward_ws: Vec::new(),
            axle: Vec::new(),
            forward_impulse: Vec::new(),
            side_impulse: Vec::new(),
            user_constraint_type: -1,
            user_constraint_id: -1,
            tau: 0.0,
            damping: 0.0,
            pitch_control: 0.0,
            steering_value: 0.0,
            current_vehicle_speed_km_hour: 0.0,
            chassis,
            index_right_axis: 0,
            index_up_axis: 2,
            index_forward_axis: 1,
            wheel_info: Vec::new(),
        }
    }

    /// `setCoordinateSystem(right, up, forward)`.
    pub fn set_coordinate_system(
        &mut self,
        right_index: usize,
        up_index: usize,
        forward_index: usize,
    ) {
        self.index_right_axis = right_index;
        self.index_up_axis = up_index;
        self.index_forward_axis = forward_index;
    }

    #[inline(always)]
    pub fn get_num_wheels(&self) -> usize {
        self.wheel_info.len()
    }

    /// `getChassisWorldTransform()`: the chassis' center-of-mass transform.
    #[inline(always)]
    pub fn get_chassis_world_transform<A: BodyArena + ?Sized>(&self, objects: &A) -> Transform {
        objects.obj(self.chassis).world_transform
    }

    /// `addWheel`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_wheel<A: BodyArena + ?Sized>(
        &mut self,
        objects: &A,
        connection_point_cs: &Vec3,
        wheel_direction_cs0: &Vec3,
        wheel_axle_cs: &Vec3,
        suspension_rest_length: f64,
        wheel_radius: f64,
        tuning: &VehicleTuning,
        is_front_wheel: bool,
    ) -> usize {
        let ci = WheelInfoConstructionInfo {
            chassis_connection_cs: *connection_point_cs,
            wheel_direction_cs: *wheel_direction_cs0,
            wheel_axle_cs: *wheel_axle_cs,
            suspension_rest_length,
            wheel_radius,
            suspension_stiffness: tuning.suspension_stiffness,
            wheels_damping_compression: tuning.suspension_compression,
            wheels_damping_relaxation: tuning.suspension_damping,
            friction_slip: tuning.friction_slip,
            b_is_front_wheel: is_front_wheel,
            max_suspension_travel_cm: tuning.max_suspension_travel_cm,
            max_suspension_force: tuning.max_suspension_force,
        };
        self.wheel_info.push(WheelInfo::new(&ci));
        let idx = self.wheel_info.len() - 1;
        let chassis_trans = self.get_chassis_world_transform(objects);
        Self::update_wheel_transforms_ws_with(&chassis_trans, &mut self.wheel_info[idx]);
        self.update_wheel_transform(objects, idx, false);
        idx
    }

    /// `updateWheelTransformsWS(wheel, interpolatedTransform)`. The game never
    /// gives the chassis a motion state, so `interpolatedTransform` changes
    /// nothing (it is honoured anyway).
    pub fn update_wheel_transforms_ws<A: BodyArena + ?Sized>(
        &mut self,
        objects: &A,
        wheel_index: usize,
        interpolated_transform: bool,
    ) {
        let chassis = objects.obj(self.chassis);
        let mut chassis_trans = chassis.world_transform;
        if interpolated_transform {
            if let Some(ms) = chassis.rb().optional_motion_state.as_ref() {
                chassis_trans = ms.get_world_transform();
            }
        }
        Self::update_wheel_transforms_ws_with(&chassis_trans, &mut self.wheel_info[wheel_index]);
    }

    fn update_wheel_transforms_ws_with(chassis_trans: &Transform, wheel: &mut WheelInfo) {
        wheel.raycast_info.is_in_contact = false;
        wheel.raycast_info.hard_point_ws = chassis_trans.apply(&wheel.chassis_connection_point_cs);
        wheel.raycast_info.wheel_direction_ws = chassis_trans.basis * wheel.wheel_direction_cs;
        wheel.raycast_info.wheel_axle_ws = chassis_trans.basis * wheel.wheel_axle_cs;
    }

    /// `updateWheelTransform(wheelIndex, interpolatedTransform)`.
    pub fn update_wheel_transform<A: BodyArena + ?Sized>(
        &mut self,
        objects: &A,
        wheel_index: usize,
        interpolated_transform: bool,
    ) {
        self.update_wheel_transforms_ws(objects, wheel_index, interpolated_transform);
        let wheel = &mut self.wheel_info[wheel_index];
        let up = -wheel.raycast_info.wheel_direction_ws;
        let right = wheel.raycast_info.wheel_axle_ws;
        let mut fwd = up.cross(&right);
        fwd = *fwd.normalize();

        let steering = wheel.steering;
        let steering_orn = Quat::from_axis_angle(&up, steering);
        let steering_mat = Mat3::from_quat(&steering_orn);

        let rotating_orn = Quat::from_axis_angle(&right, -wheel.rotation);
        let rotating_mat = Mat3::from_quat(&rotating_orn);

        let basis2 = Mat3::new(
            right.x, fwd.x, up.x, right.y, fwd.y, up.y, right.z, fwd.z, up.z,
        );

        wheel.world_transform.basis = steering_mat * rotating_mat * basis2;
        wheel.world_transform.origin = wheel.raycast_info.hard_point_ws
            + wheel.raycast_info.wheel_direction_ws * wheel.raycast_info.suspension_length;
    }

    /// `resetSuspension`.
    pub fn reset_suspension(&mut self) {
        for wheel in &mut self.wheel_info {
            wheel.raycast_info.suspension_length = wheel.get_suspension_rest_length();
            wheel.suspension_relative_velocity = 0.0;
            wheel.raycast_info.contact_normal_ws = -wheel.raycast_info.wheel_direction_ws;
            wheel.clipped_inv_contact_dot_suspension = 1.0;
        }
    }

    /// `rayCast(wheel)`: returns `depth`.
    pub fn ray_cast<A: BodyArena + ?Sized>(
        &mut self,
        objects: &A,
        wheel_index: usize,
        raycaster: &mut dyn VehicleRaycaster,
    ) -> f64 {
        self.update_wheel_transforms_ws(objects, wheel_index, false);
        let chassis = objects.obj(self.chassis);
        let wheel = &mut self.wheel_info[wheel_index];

        let mut depth = -1.0;
        let raylen = wheel.get_suspension_rest_length() + wheel.wheels_radius;
        let rayvector = wheel.raycast_info.wheel_direction_ws * raylen;
        let source = wheel.raycast_info.hard_point_ws;
        wheel.raycast_info.contact_point_ws = source + rayvector;
        let target = wheel.raycast_info.contact_point_ws;

        let hit = raycaster.cast_ray(&source, &target);
        wheel.raycast_info.ground_object = false;

        if let Some(ray_results) = hit {
            let param = ray_results.dist_fraction;
            depth = raylen * ray_results.dist_fraction;
            wheel.raycast_info.contact_normal_ws = ray_results.hit_normal_in_world;
            wheel.raycast_info.is_in_contact = true;
            wheel.raycast_info.ground_object = true;

            let hit_distance = param * raylen;
            wheel.raycast_info.suspension_length = hit_distance - wheel.wheels_radius;

            let min_suspension_length = fma(
                -wheel.max_suspension_travel_cm,
                0.01,
                wheel.get_suspension_rest_length(),
            );
            let max_suspension_length = fma(
                wheel.max_suspension_travel_cm,
                0.01,
                wheel.get_suspension_rest_length(),
            );
            if wheel.raycast_info.suspension_length < min_suspension_length {
                wheel.raycast_info.suspension_length = min_suspension_length;
            }
            if wheel.raycast_info.suspension_length > max_suspension_length {
                wheel.raycast_info.suspension_length = max_suspension_length;
            }

            wheel.raycast_info.contact_point_ws = ray_results.hit_point_in_world;

            let denominator = wheel
                .raycast_info
                .contact_normal_ws
                .dot(&wheel.raycast_info.wheel_direction_ws);
            let relpos = wheel.raycast_info.contact_point_ws - chassis.world_transform.origin;
            let chassis_velocity_at_contact_point = chassis.get_velocity_in_local_point(&relpos);
            let proj_vel = wheel
                .raycast_info
                .contact_normal_ws
                .dot(&chassis_velocity_at_contact_point);

            if denominator >= -0.1 {
                wheel.suspension_relative_velocity = 0.0;
                wheel.clipped_inv_contact_dot_suspension = 1.0 / 0.1;
            } else {
                let inv = -1.0 / denominator;
                wheel.suspension_relative_velocity = proj_vel * inv;
                wheel.clipped_inv_contact_dot_suspension = inv;
            }
        } else {
            wheel.raycast_info.suspension_length = wheel.get_suspension_rest_length();
            wheel.suspension_relative_velocity = 0.0;
            wheel.raycast_info.contact_normal_ws = -wheel.raycast_info.wheel_direction_ws;
            wheel.clipped_inv_contact_dot_suspension = 1.0;
        }
        depth
    }

    /// `updateAction(collisionWorld, step)` = `updateVehicle(step)`.
    pub fn update_vehicle<A: BodyArena + ?Sized>(
        &mut self,
        objects: &mut A,
        step: f64,
        raycaster: &mut dyn VehicleRaycaster,
    ) {
        for i in 0..self.get_num_wheels() {
            self.update_wheel_transform(objects, i, false);
        }

        {
            let chassis = objects.obj(self.chassis);
            self.current_vehicle_speed_km_hour = 3.6 * chassis.rb().linear_velocity.length();
            let chassis_trans = chassis.world_transform;
            let b = &chassis_trans.basis;
            let fa = self.index_forward_axis;
            let forward_w = Vec3::new(b[0][fa], b[1][fa], b[2][fa]);
            if forward_w.dot(&chassis.rb().linear_velocity) < 0.0 {
                self.current_vehicle_speed_km_hour *= -1.0;
            }
        }

        for i in 0..self.wheel_info.len() {
            let _depth = self.ray_cast(objects, i, raycaster);
        }

        self.update_suspension(objects, step);

        for i in 0..self.wheel_info.len() {
            let wheel = &self.wheel_info[i];
            let mut suspension_force = wheel.wheels_suspension_force;
            if suspension_force > wheel.max_suspension_force {
                suspension_force = wheel.max_suspension_force;
            }
            let impulse = wheel.raycast_info.contact_normal_ws * suspension_force * step;
            let chassis = objects.obj_mut(self.chassis);
            let relpos = wheel.raycast_info.contact_point_ws - chassis.world_transform.origin;
            chassis.apply_impulse(&impulse, &relpos);
        }

        self.update_friction(objects, step);

        let fa = self.index_forward_axis;
        for i in 0..self.wheel_info.len() {
            let chassis = objects.obj(self.chassis);
            let wheel = &mut self.wheel_info[i];
            let relpos = wheel.raycast_info.hard_point_ws - chassis.world_transform.origin;
            let vel = chassis.get_velocity_in_local_point(&relpos);

            if wheel.raycast_info.is_in_contact {
                let b = &chassis.world_transform.basis;
                let mut fwd = Vec3::new(b[0][fa], b[1][fa], b[2][fa]);
                let proj = fwd.dot(&wheel.raycast_info.contact_normal_ws);
                fwd -= wheel.raycast_info.contact_normal_ws * proj;
                let proj2 = fwd.dot(&vel);
                wheel.delta_rotation = (proj2 * step) / wheel.wheels_radius;
                wheel.rotation += wheel.delta_rotation;
            } else {
                wheel.rotation += wheel.delta_rotation;
            }
            wheel.delta_rotation *= 0.99;
        }
    }

    /// `setSteeringValue`.
    pub fn set_steering_value(&mut self, steering: f64, wheel: usize) {
        self.wheel_info[wheel].steering = steering;
    }
    /// `getSteeringValue`.
    pub fn get_steering_value(&self, wheel: usize) -> f64 {
        self.wheel_info[wheel].steering
    }
    /// `applyEngineForce`.
    pub fn apply_engine_force(&mut self, force: f64, wheel: usize) {
        self.wheel_info[wheel].engine_force = force;
    }
    /// `setBrake`.
    pub fn set_brake(&mut self, brake: f64, wheel: usize) {
        self.wheel_info[wheel].brake = brake;
    }
    /// `getWheelTransformWS`.
    pub fn get_wheel_transform_ws(&self, wheel: usize) -> &Transform {
        &self.wheel_info[wheel].world_transform
    }

    /// `updateSuspension(deltaTime)`.
    pub fn update_suspension<A: BodyArena + ?Sized>(&mut self, objects: &A, _delta_time: f64) {
        let chassis_mass = 1.0 / objects.obj(self.chassis).rb().inverse_mass;
        for wheel_info in &mut self.wheel_info {
            if wheel_info.raycast_info.is_in_contact {
                let susp_length = wheel_info.get_suspension_rest_length();
                let current_length = wheel_info.raycast_info.suspension_length;
                let length_diff = susp_length - current_length;
                let mut force = wheel_info.suspension_stiffness
                    * length_diff
                    * wheel_info.clipped_inv_contact_dot_suspension;

                let projected_rel_vel = wheel_info.suspension_relative_velocity;
                let susp_damping = if projected_rel_vel < 0.0 {
                    wheel_info.wheels_damping_compression
                } else {
                    wheel_info.wheels_damping_relaxation
                };
                // force -= susp_damping * projected_rel_vel
                force = fma(-susp_damping, projected_rel_vel, force);

                wheel_info.wheels_suspension_force = force * chassis_mass;
                if wheel_info.wheels_suspension_force < 0.0 {
                    wheel_info.wheels_suspension_force = 0.0;
                }
            } else {
                wheel_info.wheels_suspension_force = 0.0;
            }
        }
    }

    /// `updateFriction(timeStep)`.
    pub fn update_friction<A: BodyArena + ?Sized>(&mut self, objects: &mut A, time_step: f64) {
        let num_wheel = self.get_num_wheels();
        if num_wheel == 0 {
            return;
        }
        // btAlignedObjectArray::resize keeps existing entries.
        self.forward_ws.resize(num_wheel, Vec3::ZERO);
        self.axle.resize(num_wheel, Vec3::ZERO);
        self.forward_impulse.resize(num_wheel, 0.0);
        self.side_impulse.resize(num_wheel, 0.0);

        let ground = fixed_body();

        let mut _num_wheels_on_ground = 0;
        for i in 0..num_wheel {
            if self.wheel_info[i].raycast_info.ground_object {
                _num_wheels_on_ground += 1;
            }
            self.side_impulse[i] = 0.0;
            self.forward_impulse[i] = 0.0;
        }

        let ra = self.index_right_axis;
        for i in 0..num_wheel {
            let wheel_info = &self.wheel_info[i];
            if wheel_info.raycast_info.ground_object {
                let wheel_basis0 = wheel_info.world_transform.basis;
                self.axle[i] = Vec3::new(
                    wheel_basis0[0][ra],
                    wheel_basis0[1][ra],
                    wheel_basis0[2][ra],
                );

                let surf_normal_ws = wheel_info.raycast_info.contact_normal_ws;
                let proj = self.axle[i].dot(&surf_normal_ws);
                self.axle[i] -= surf_normal_ws * proj;
                self.axle[i] = *self.axle[i].normalize();

                self.forward_ws[i] = surf_normal_ws.cross(&self.axle[i]);
                self.forward_ws[i].normalize();

                let chassis = objects.obj(self.chassis);
                self.side_impulse[i] = resolve_single_bilateral(
                    chassis,
                    &wheel_info.raycast_info.contact_point_ws,
                    &ground,
                    &wheel_info.raycast_info.contact_point_ws,
                    &self.axle[i],
                );
                self.side_impulse[i] *= SIDE_FRICTION_STIFFNESS2;
            }
        }

        let side_factor = 1.0;
        let fwd_factor = 0.5;
        let mut sliding = false;
        for wheel in 0..num_wheel {
            let wheel_info = self.wheel_info[wheel];
            let mut rolling_friction = 0.0;
            if wheel_info.raycast_info.ground_object {
                if wheel_info.engine_force != 0.0 {
                    rolling_friction = wheel_info.engine_force * time_step;
                } else {
                    let default_rolling_friction_impulse = 0.0;
                    let max_impulse = if wheel_info.brake != 0.0 {
                        wheel_info.brake
                    } else {
                        default_rolling_friction_impulse
                    };
                    let chassis = objects.obj(self.chassis);
                    rolling_friction = calc_rolling_friction(
                        chassis,
                        &ground,
                        &wheel_info.raycast_info.contact_point_ws,
                        &self.forward_ws[wheel],
                        max_impulse,
                    );
                }
            }

            self.forward_impulse[wheel] = 0.0;
            self.wheel_info[wheel].skid_info = 1.0;

            if wheel_info.raycast_info.ground_object {
                self.wheel_info[wheel].skid_info = 1.0;
                let maximp =
                    wheel_info.wheels_suspension_force * time_step * wheel_info.friction_slip;
                let maximp_side = maximp;
                let maximp_squared = maximp * maximp_side;

                self.forward_impulse[wheel] = rolling_friction;

                let x = self.forward_impulse[wheel] * fwd_factor;
                let y = self.side_impulse[wheel] * side_factor;
                let impulse_squared = fma(x, x, y * y);

                if impulse_squared > maximp_squared {
                    sliding = true;
                    let factor = maximp / impulse_squared.sqrt();
                    self.wheel_info[wheel].skid_info *= factor;
                }
            }
        }

        if sliding {
            for wheel in 0..num_wheel {
                if self.side_impulse[wheel] != 0.0 && self.wheel_info[wheel].skid_info < 1.0 {
                    self.forward_impulse[wheel] *= self.wheel_info[wheel].skid_info;
                    self.side_impulse[wheel] *= self.wheel_info[wheel].skid_info;
                }
            }
        }

        let ua = self.index_up_axis;
        for wheel in 0..num_wheel {
            let wheel_info = self.wheel_info[wheel];
            let chassis = objects.obj_mut(self.chassis);
            let mut rel_pos =
                wheel_info.raycast_info.contact_point_ws - chassis.world_transform.origin;

            if self.forward_impulse[wheel] != 0.0 {
                chassis.apply_impulse(
                    &(self.forward_ws[wheel] * self.forward_impulse[wheel]),
                    &rel_pos,
                );
            }
            if self.side_impulse[wheel] != 0.0 {
                // groundObject is the fixed body: its applyImpulse does nothing.
                let side_imp = self.axle[wheel] * self.side_impulse[wheel];
                if ROLLING_INFLUENCE_FIX {
                    let v_chassis_world_up = chassis.world_transform.basis.get_column(ua);
                    rel_pos -= v_chassis_world_up
                        * (v_chassis_world_up.dot(&rel_pos) * (1.0 - wheel_info.roll_influence));
                } else {
                    rel_pos[ua] *= wheel_info.roll_influence;
                }
                chassis.apply_impulse(&side_imp, &rel_pos);
            }
        }
    }
}
