// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2006 Erwin Coumans  http://continuousphysics.com/Bullet/
//
// This software is provided 'as-is', without any express or implied warranty.
// In no event will the authors be held liable for any damages arising from the use of this software.
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it freely,
// subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Altered for pz-rs: Rust translation of btRigidBody (Bullet 2.82, double
// precision).

//! `btRigidBody` methods, as methods of [`CollisionObject`] (a rigid body is
//! a collision object whose `body` is `Some`).

use crate::bt::collision_object::*;
use crate::bt::linear_math::*;

use super::typed_constraint::ConstraintArena;

/// `gDeactivationTime` as set by the game (`WorldSimulation` sets 3.0).
pub const G_DEACTIVATION_TIME: f64 = 3.0;
/// `gDisableDeactivation`.
pub const G_DISABLE_DEACTIVATION: bool = false;

/// `btRigidBody::btRigidBodyConstructionInfo` (defaults of the 4-argument
/// constructor).
#[derive(Clone, Debug)]
pub struct RigidBodyConstructionInfo {
    pub mass: f64,
    pub motion_state: Option<DefaultMotionState>,
    pub start_world_transform: Transform,
    pub collision_shape: Option<ShapeId>,
    pub local_inertia: Vec3,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub friction: f64,
    pub rolling_friction: f64,
    pub restitution: f64,
    pub linear_sleeping_threshold: f64,
    pub angular_sleeping_threshold: f64,
    pub additional_damping: bool,
    pub additional_damping_factor: f64,
    pub additional_linear_damping_threshold_sqr: f64,
    pub additional_angular_damping_threshold_sqr: f64,
    pub additional_angular_damping_factor: f64,
}

impl RigidBodyConstructionInfo {
    /// `btRigidBodyConstructionInfo(mass, motionState, collisionShape, localInertia)`.
    pub fn new(
        mass: f64,
        motion_state: Option<DefaultMotionState>,
        collision_shape: Option<ShapeId>,
        local_inertia: Vec3,
    ) -> RigidBodyConstructionInfo {
        RigidBodyConstructionInfo {
            mass,
            motion_state,
            start_world_transform: Transform::IDENTITY,
            collision_shape,
            local_inertia,
            linear_damping: 0.0,
            angular_damping: 0.0,
            friction: 0.5,
            rolling_friction: 0.0,
            restitution: 0.0,
            linear_sleeping_threshold: 0.8,
            angular_sleeping_threshold: flt(1.0),
            additional_damping: false,
            additional_damping_factor: 0.005,
            additional_linear_damping_threshold_sqr: 0.01,
            additional_angular_damping_threshold_sqr: 0.01,
            additional_angular_damping_factor: 0.01,
        }
    }
}

thread_local! {
    static UNIQUE_ID: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

impl CollisionObject {
    /// `btRigidBody(constructionInfo)`: a new collision object with rigid-body
    /// data, set up by `setupRigidBody`.
    pub fn new_rigid_body(info: &RigidBodyConstructionInfo) -> CollisionObject {
        let mut o = CollisionObject::new();
        o.body = Some(Box::default());
        o.setup_rigid_body(info);
        o
    }

    /// `setupRigidBody`.
    pub fn setup_rigid_body(&mut self, info: &RigidBodyConstructionInfo) {
        self.internal_type = CO_RIGID_BODY;
        if self.body.is_none() {
            self.body = Some(Box::default());
        }
        {
            let rb = self.rb_mut();
            rb.linear_velocity.set_value(0.0, 0.0, 0.0);
            rb.angular_velocity.set_value(0.0, 0.0, 0.0);
            rb.angular_factor.set_value(1.0, 1.0, 1.0);
            rb.linear_factor.set_value(1.0, 1.0, 1.0);
            rb.gravity.set_value(0.0, 0.0, 0.0);
            rb.gravity_acceleration.set_value(0.0, 0.0, 0.0);
            rb.total_force.set_value(0.0, 0.0, 0.0);
            rb.total_torque.set_value(0.0, 0.0, 0.0);
        }
        self.set_damping(info.linear_damping, info.angular_damping);
        {
            let rb = self.rb_mut();
            rb.linear_sleeping_threshold = info.linear_sleeping_threshold;
            rb.angular_sleeping_threshold = info.angular_sleeping_threshold;
            rb.optional_motion_state = info.motion_state;
            rb.contact_solver_type = 0;
            rb.friction_solver_type = 0;
            rb.additional_damping = info.additional_damping;
            rb.additional_damping_factor = info.additional_damping_factor;
            rb.additional_linear_damping_threshold_sqr =
                info.additional_linear_damping_threshold_sqr;
            rb.additional_angular_damping_threshold_sqr =
                info.additional_angular_damping_threshold_sqr;
            rb.additional_angular_damping_factor = info.additional_angular_damping_factor;
        }
        if let Some(ms) = &self.rb().optional_motion_state {
            self.world_transform = ms.get_world_transform();
        } else {
            self.world_transform = info.start_world_transform;
        }
        self.interpolation_world_transform = self.world_transform;
        self.interpolation_linear_velocity.set_value(0.0, 0.0, 0.0);
        self.interpolation_angular_velocity.set_value(0.0, 0.0, 0.0);
        self.friction = info.friction;
        self.rolling_friction = info.rolling_friction;
        self.restitution = info.restitution;
        if let Some(s) = info.collision_shape {
            self.set_collision_shape(s);
        } else {
            self.update_revision += 1;
            self.collision_shape = None;
            self.root_collision_shape = None;
        }
        self.rb_mut().debug_body_id = UNIQUE_ID.with(|u| {
            let v = u.get();
            u.set(v.wrapping_add(1));
            v
        });
        self.set_mass_props(info.mass, &info.local_inertia);
        self.update_inertia_tensor();
        let rb = self.rb_mut();
        rb.rigidbody_flags = 0;
        rb.delta_linear_velocity.set_zero();
        rb.delta_angular_velocity.set_zero();
        rb.inv_mass = rb.inverse_mass * rb.linear_factor;
        rb.push_velocity.set_zero();
        rb.turn_velocity.set_zero();
    }

    /// `predictIntegratedTransform`.
    pub fn predict_integrated_transform(&self, time_step: f64) -> Transform {
        let rb = self.rb();
        transform_util::integrate_transform(
            &self.world_transform,
            &rb.linear_velocity,
            &rb.angular_velocity,
            time_step,
        )
    }

    /// `saveKinematicState`.
    pub fn save_kinematic_state(&mut self, time_step: f64) {
        if time_step != 0.0 {
            if let Some(ms) = &self.rb().optional_motion_state {
                self.world_transform = ms.get_world_transform();
            }
            let (lin, ang) = transform_util::calculate_velocity(
                &self.interpolation_world_transform,
                &self.world_transform,
                time_step,
            );
            let rb = self.rb_mut();
            rb.linear_velocity = lin;
            rb.angular_velocity = ang;
            self.interpolation_linear_velocity = lin;
            self.interpolation_angular_velocity = ang;
            self.interpolation_world_transform = self.world_transform;
        }
    }

    /// `setGravity`.
    pub fn set_gravity(&mut self, acceleration: &Vec3) {
        let rb = self.rb_mut();
        if rb.inverse_mass != 0.0 {
            rb.gravity = *acceleration * (1.0 / rb.inverse_mass);
        }
        rb.gravity_acceleration = *acceleration;
    }

    /// `setDamping`.
    pub fn set_damping(&mut self, lin_damping: f64, ang_damping: f64) {
        let rb = self.rb_mut();
        rb.linear_damping = bt_clamped(lin_damping, 0.0, 1.0);
        rb.angular_damping = bt_clamped(ang_damping, 0.0, 1.0);
    }

    /// `applyDamping`.
    pub fn apply_damping(&mut self, time_step: f64) {
        let rb = self.rb_mut();
        rb.linear_velocity *= bt_pow(1.0 - rb.linear_damping, time_step);
        rb.angular_velocity *= bt_pow(1.0 - rb.angular_damping, time_step);
        if rb.additional_damping {
            if rb.angular_velocity.length2() < rb.additional_angular_damping_threshold_sqr
                && rb.linear_velocity.length2() < rb.additional_linear_damping_threshold_sqr
            {
                rb.angular_velocity *= rb.additional_damping_factor;
                rb.linear_velocity *= rb.additional_damping_factor;
            }
            let speed = rb.linear_velocity.length();
            if speed < rb.linear_damping {
                let damp_vel = 0.005;
                if speed > damp_vel {
                    let dir = rb.linear_velocity.normalized();
                    rb.linear_velocity -= dir * damp_vel;
                } else {
                    rb.linear_velocity.set_value(0.0, 0.0, 0.0);
                }
            }
            let ang_speed = rb.angular_velocity.length();
            if ang_speed < rb.angular_damping {
                let ang_damp_vel = 0.005;
                if ang_speed > ang_damp_vel {
                    let dir = rb.angular_velocity.normalized();
                    rb.angular_velocity -= dir * ang_damp_vel;
                } else {
                    rb.angular_velocity.set_value(0.0, 0.0, 0.0);
                }
            }
        }
    }

    /// `applyGravity`.
    pub fn apply_gravity(&mut self) {
        if self.is_static_or_kinematic_object() {
            return;
        }
        let g = self.rb().gravity;
        self.apply_central_force(&g);
    }

    /// `proceedToTransform`.
    pub fn proceed_to_transform(&mut self, new_trans: &Transform) {
        self.set_center_of_mass_transform(new_trans);
    }

    /// `setMassProps`.
    pub fn set_mass_props(&mut self, mass: f64, inertia: &Vec3) {
        if mass == 0.0 {
            self.collision_flags |= CF_STATIC_OBJECT;
            self.rb_mut().inverse_mass = 0.0;
        } else {
            self.collision_flags &= !CF_STATIC_OBJECT;
            self.rb_mut().inverse_mass = 1.0 / mass;
        }
        let rb = self.rb_mut();
        rb.gravity = mass * rb.gravity_acceleration;
        rb.inv_inertia_local.set_value(
            if inertia.x != 0.0 {
                1.0 / inertia.x
            } else {
                0.0
            },
            if inertia.y != 0.0 {
                1.0 / inertia.y
            } else {
                0.0
            },
            if inertia.z != 0.0 {
                1.0 / inertia.z
            } else {
                0.0
            },
        );
        rb.inv_mass = rb.linear_factor * rb.inverse_mass;
    }

    /// `setLinearFactor`.
    pub fn set_linear_factor(&mut self, linear_factor: &Vec3) {
        let rb = self.rb_mut();
        rb.linear_factor = *linear_factor;
        rb.inv_mass = rb.linear_factor * rb.inverse_mass;
    }

    /// `updateInertiaTensor`.
    pub fn update_inertia_tensor(&mut self) {
        let b = self.world_transform.basis;
        let rb = self.body.as_deref_mut().expect("not a rigid body");
        rb.inv_inertia_tensor_world = b.scaled(&rb.inv_inertia_local) * b.transpose();
    }

    /// `computeGyroscopicForce`.
    pub fn compute_gyroscopic_force(&self, max_gyroscopic_force: f64) -> Vec3 {
        let rb = self.rb();
        let mut inertia_local = Vec3::ZERO;
        inertia_local[0] = 1.0 / rb.inv_inertia_local[0];
        inertia_local[1] = 1.0 / rb.inv_inertia_local[1];
        inertia_local[2] = 1.0 / rb.inv_inertia_local[2];
        let b = self.world_transform.basis;
        let inertia_tensor_world = b.scaled(&inertia_local) * b.transpose();
        let tmp = inertia_tensor_world * rb.angular_velocity;
        let mut gf = rb.angular_velocity.cross(&tmp);
        let l2 = gf.length2();
        if l2 > max_gyroscopic_force * max_gyroscopic_force {
            gf *= 1.0 / l2.sqrt() * max_gyroscopic_force;
        }
        gf
    }

    /// `integrateVelocities`.
    pub fn integrate_velocities(&mut self, step: f64) {
        if self.is_static_or_kinematic_object() {
            return;
        }
        let rb = self.rb_mut();
        rb.linear_velocity += rb.total_force * (rb.inverse_mass * step);
        rb.angular_velocity += rb.inv_inertia_tensor_world * rb.total_torque * step;
        let angvel = rb.angular_velocity.length();
        if angvel * step > SIMD_HALF_PI {
            rb.angular_velocity *= (SIMD_HALF_PI / step) / angvel;
        }
    }

    /// `getOrientation`.
    pub fn get_orientation(&self) -> Quat {
        self.world_transform.basis.get_rotation()
    }

    /// `setCenterOfMassTransform`.
    pub fn set_center_of_mass_transform(&mut self, xform: &Transform) {
        if self.is_kinematic_object() {
            self.interpolation_world_transform = self.world_transform;
        } else {
            self.interpolation_world_transform = *xform;
        }
        self.interpolation_linear_velocity = self.rb().linear_velocity;
        self.interpolation_angular_velocity = self.rb().angular_velocity;
        self.world_transform = *xform;
        self.update_inertia_tensor();
    }

    /// `applyCentralForce`.
    #[inline]
    pub fn apply_central_force(&mut self, force: &Vec3) {
        let rb = self.rb_mut();
        rb.total_force += *force * rb.linear_factor;
    }

    /// `applyTorque`.
    #[inline]
    pub fn apply_torque(&mut self, torque: &Vec3) {
        let rb = self.rb_mut();
        rb.total_torque += *torque * rb.angular_factor;
    }

    /// `applyForce(force, rel_pos)`.
    pub fn apply_force(&mut self, force: &Vec3, rel_pos: &Vec3) {
        self.apply_central_force(force);
        let lf = self.rb().linear_factor;
        self.apply_torque(&rel_pos.cross(&(*force * lf)));
    }

    /// `applyCentralImpulse`.
    #[inline]
    pub fn apply_central_impulse(&mut self, impulse: &Vec3) {
        let rb = self.rb_mut();
        rb.linear_velocity += *impulse * rb.linear_factor * rb.inverse_mass;
    }

    /// `applyTorqueImpulse`.
    #[inline]
    pub fn apply_torque_impulse(&mut self, torque: &Vec3) {
        let rb = self.rb_mut();
        rb.angular_velocity += rb.inv_inertia_tensor_world * *torque * rb.angular_factor;
    }

    /// `applyImpulse(impulse, rel_pos)`.
    pub fn apply_impulse(&mut self, impulse: &Vec3, rel_pos: &Vec3) {
        if self.rb().inverse_mass != 0.0 {
            self.apply_central_impulse(impulse);
            // `if (m_angularFactor)` tests the vector's address: always true.
            let lf = self.rb().linear_factor;
            self.apply_torque_impulse(&rel_pos.cross(&(*impulse * lf)));
        }
    }

    /// `clearForces`.
    pub fn clear_forces(&mut self) {
        let rb = self.rb_mut();
        rb.total_force.set_value(0.0, 0.0, 0.0);
        rb.total_torque.set_value(0.0, 0.0, 0.0);
    }

    /// `setLinearVelocity`.
    pub fn set_linear_velocity(&mut self, v: &Vec3) {
        self.update_revision += 1;
        self.rb_mut().linear_velocity = *v;
    }

    /// `setAngularVelocity`.
    pub fn set_angular_velocity(&mut self, v: &Vec3) {
        self.update_revision += 1;
        self.rb_mut().angular_velocity = *v;
    }

    /// `getVelocityInLocalPoint`.
    #[inline]
    pub fn get_velocity_in_local_point(&self, rel_pos: &Vec3) -> Vec3 {
        let rb = self.rb();
        rb.linear_velocity + rb.angular_velocity.cross(rel_pos)
    }

    /// `computeImpulseDenominator`.
    pub fn compute_impulse_denominator(&self, pos: &Vec3, normal: &Vec3) -> f64 {
        let rb = self.rb();
        let r0 = *pos - self.world_transform.origin;
        let c0 = r0.cross(normal);
        let vec = (c0 * rb.inv_inertia_tensor_world).cross(&r0);
        rb.inverse_mass + normal.dot(&vec)
    }

    /// `computeAngularImpulseDenominator`.
    pub fn compute_angular_impulse_denominator(&self, axis: &Vec3) -> f64 {
        let vec = *axis * self.rb().inv_inertia_tensor_world;
        axis.dot(&vec)
    }

    /// `updateDeactivation`.
    pub fn update_deactivation(&mut self, time_step: f64) {
        let st = self.activation_state1;
        if st == ISLAND_SLEEPING || st == DISABLE_DEACTIVATION {
            return;
        }
        let rb = self.rb();
        if rb.linear_velocity.length2()
            < rb.linear_sleeping_threshold * rb.linear_sleeping_threshold
            && rb.angular_velocity.length2()
                < rb.angular_sleeping_threshold * rb.angular_sleeping_threshold
        {
            self.deactivation_time += time_step;
        } else {
            self.deactivation_time = 0.0;
            self.set_activation_state(0);
        }
    }

    /// `wantsSleeping`.
    pub fn wants_sleeping(&self) -> bool {
        let st = self.activation_state1;
        if st == DISABLE_DEACTIVATION {
            return false;
        }
        if G_DISABLE_DEACTIVATION || G_DEACTIVATION_TIME == 0.0 {
            return false;
        }
        if st == ISLAND_SLEEPING || st == WANTS_DEACTIVATION {
            return true;
        }
        self.deactivation_time > G_DEACTIVATION_TIME
    }

    /// `getInvMass` (0 for non-rigid objects, like `upcast` callers expect).
    #[inline]
    pub fn get_inv_mass(&self) -> f64 {
        self.rb().inverse_mass
    }

    /// `addConstraintRef`.
    pub fn add_constraint_ref(&mut self, c: ConstraintId) {
        let rb = self.rb_mut();
        if !rb.constraint_refs.contains(&c) {
            rb.constraint_refs.push(c);
        }
        self.check_collide_with = true;
    }

    /// `removeConstraintRef` (`btAlignedObjectArray::remove`: swaps the last
    /// element into the first match and pops).
    pub fn remove_constraint_ref(&mut self, c: ConstraintId) {
        let rb = self.rb_mut();
        if let Some(i) = rb.constraint_refs.iter().position(|&x| x == c) {
            let last = rb.constraint_refs.len() - 1;
            rb.constraint_refs.swap(i, last);
            rb.constraint_refs.pop();
        }
        self.check_collide_with = !rb.constraint_refs.is_empty();
    }
}

/// `btRigidBody::checkCollideWithOverride(co)` for rigid body `this`:
/// false when an enabled constraint referenced by `this` links it to `other`.
pub fn check_collide_with_override(
    objects: &[CollisionObject],
    constraints: &ConstraintArena,
    this: ObjId,
    other: ObjId,
) -> bool {
    check_collide_with_override_by(|id| &objects[id], constraints, this, other)
}

/// Same as [`check_collide_with_override`] with an object accessor.
pub fn check_collide_with_override_by<'a, F: Fn(ObjId) -> &'a CollisionObject>(
    obj: F,
    constraints: &ConstraintArena,
    this: ObjId,
    other: ObjId,
) -> bool {
    if !obj(other).is_rigid_body() {
        return true;
    }
    for &cid in &obj(this).rb().constraint_refs {
        let c = constraints.get(cid);
        if c.is_enabled && (c.rb_a == Some(other) || c.rb_b == Some(other)) {
            return false;
        }
    }
    true
}

/// `btCollisionObject::checkCollideWith(co)` dispatching to the rigid-body override.
pub fn check_collide_with<'a, F: Fn(ObjId) -> &'a CollisionObject>(
    obj: F,
    constraints: &ConstraintArena,
    this: ObjId,
    other: ObjId,
) -> bool {
    let o = obj(this);
    if o.check_collide_with {
        if o.is_rigid_body() {
            return check_collide_with_override_by(obj, constraints, this, other);
        }
        return true;
    }
    true
}
