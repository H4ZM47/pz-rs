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
// Altered for pz-rs: Rust translation of btSolverBody, btSolverConstraint and
// btContactSolverInfo (Bullet 2.82, double precision).

//! `btSolverBody`, `btSolverConstraint` and `btContactSolverInfo`.

use crate::bt::collision_object::{ConstraintId, ObjId};
use crate::bt::linear_math::*;
use crate::bt::manifold::ManifoldId;

pub const SOLVER_RANDMIZE_ORDER: i32 = 1;
pub const SOLVER_FRICTION_SEPARATE: i32 = 2;
pub const SOLVER_USE_WARMSTARTING: i32 = 4;
pub const SOLVER_USE_2_FRICTION_DIRECTIONS: i32 = 16;
pub const SOLVER_ENABLE_FRICTION_DIRECTION_CACHING: i32 = 32;
pub const SOLVER_DISABLE_VELOCITY_DEPENDENT_FRICTION_DIRECTION: i32 = 64;
pub const SOLVER_CACHE_FRIENDLY: i32 = 128;
pub const SOLVER_SIMD: i32 = 256;
pub const SOLVER_INTERLEAVE_CONTACT_AND_FRICTION_CONSTRAINTS: i32 = 512;
pub const SOLVER_ALLOW_ZERO_LENGTH_FRICTION_DIRECTIONS: i32 = 1024;

/// `btContactSolverInfo` (`btContactSolverInfoData`).
#[derive(Clone, Copy, Debug)]
pub struct ContactSolverInfo {
    pub tau: f64,
    pub damping: f64,
    pub friction: f64,
    pub time_step: f64,
    pub restitution: f64,
    pub num_iterations: i32,
    pub max_error_reduction: f64,
    pub sor: f64,
    pub erp: f64,
    pub erp2: f64,
    pub global_cfm: f64,
    pub split_impulse: i32,
    pub split_impulse_penetration_threshold: f64,
    pub split_impulse_turn_erp: f64,
    pub linear_slop: f64,
    pub warmstarting_factor: f64,
    pub solver_mode: i32,
    pub resting_contact_restitution_threshold: i32,
    pub minimum_solver_batch_size: i32,
    pub max_gyroscopic_force: f64,
    pub single_axis_rolling_friction_threshold: f64,
}

impl Default for ContactSolverInfo {
    fn default() -> Self {
        ContactSolverInfo {
            tau: 0.6,
            damping: 1.0,
            friction: 0.3,
            time_step: flt(1.0 / 60.0),
            restitution: 0.0,
            max_error_reduction: 20.0,
            num_iterations: 10,
            erp: 0.2,
            erp2: 0.8,
            global_cfm: 0.0,
            sor: 1.0,
            split_impulse: 1,
            split_impulse_penetration_threshold: flt(-0.04),
            split_impulse_turn_erp: flt(0.1),
            linear_slop: 0.0,
            warmstarting_factor: 0.85,
            solver_mode: SOLVER_USE_WARMSTARTING | SOLVER_SIMD,
            resting_contact_restitution_threshold: 2,
            minimum_solver_batch_size: 128,
            max_gyroscopic_force: flt(100.0),
            single_axis_rolling_friction_threshold: flt(1e30),
        }
    }
}

/// `btSolverBody`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SolverBody {
    pub world_transform: Transform,
    pub delta_linear_velocity: Vec3,
    pub delta_angular_velocity: Vec3,
    pub angular_factor: Vec3,
    pub linear_factor: Vec3,
    pub inv_mass: Vec3,
    pub push_velocity: Vec3,
    pub turn_velocity: Vec3,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub external_force_impulse: Vec3,
    pub external_torque_impulse: Vec3,
    /// `m_originalBody`.
    pub original_body: Option<ObjId>,
}

impl SolverBody {
    /// `getVelocityInLocalPointNoDelta`.
    #[inline]
    pub fn get_velocity_in_local_point_no_delta(&self, rel_pos: &Vec3) -> Vec3 {
        if self.original_body.is_some() {
            self.linear_velocity
                + self.external_force_impulse
                + (self.angular_velocity + self.external_torque_impulse).cross(rel_pos)
        } else {
            Vec3::ZERO
        }
    }
    /// `internalApplyImpulse` (also `applyImpulse`).
    #[inline]
    pub fn internal_apply_impulse(
        &mut self,
        linear_component: &Vec3,
        angular_component: &Vec3,
        impulse_magnitude: f64,
    ) {
        if self.original_body.is_some() {
            self.delta_linear_velocity +=
                *linear_component * impulse_magnitude * self.linear_factor;
            self.delta_angular_velocity +=
                *angular_component * (impulse_magnitude * self.angular_factor);
        }
    }
    /// `internalApplyPushImpulse`.
    #[inline]
    pub fn internal_apply_push_impulse(
        &mut self,
        linear_component: &Vec3,
        angular_component: &Vec3,
        impulse_magnitude: f64,
    ) {
        if self.original_body.is_some() {
            self.push_velocity += *linear_component * impulse_magnitude * self.linear_factor;
            self.turn_velocity += *angular_component * (impulse_magnitude * self.angular_factor);
        }
    }
    /// `writebackVelocity`.
    pub fn writeback_velocity(&mut self) {
        if self.original_body.is_some() {
            self.linear_velocity += self.delta_linear_velocity;
            self.angular_velocity += self.delta_angular_velocity;
        }
    }
    /// `writebackVelocityAndTransform`.
    pub fn writeback_velocity_and_transform(
        &mut self,
        time_step: f64,
        split_impulse_turn_erp: f64,
    ) {
        if self.original_body.is_some() {
            self.linear_velocity += self.delta_linear_velocity;
            self.angular_velocity += self.delta_angular_velocity;
            let p = self.push_velocity;
            let t = self.turn_velocity;
            if p[0] != 0.0
                || p[1] != 0.0
                || p[2] != 0.0
                || t[0] != 0.0
                || t[1] != 0.0
                || t[2] != 0.0
            {
                let new_transform = transform_util::integrate_transform(
                    &self.world_transform,
                    &self.push_velocity,
                    &(self.turn_velocity * split_impulse_turn_erp),
                    time_step,
                );
                self.world_transform = new_transform;
            }
        }
    }
}

/// What a solver row came from (`m_originalContactPoint`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum RowOrigin {
    #[default]
    None,
    /// A contact point: manifold and point index.
    Contact(ManifoldId, usize),
    /// A typed constraint.
    Constraint(ConstraintId),
}

/// `btSolverConstraint`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SolverConstraint {
    pub relpos1_cross_normal: Vec3,
    pub contact_normal1: Vec3,
    pub relpos2_cross_normal: Vec3,
    pub contact_normal2: Vec3,
    pub angular_component_a: Vec3,
    pub angular_component_b: Vec3,
    pub applied_push_impulse: f64,
    pub applied_impulse: f64,
    pub friction: f64,
    pub jac_diag_ab_inv: f64,
    pub rhs: f64,
    pub cfm: f64,
    pub lower_limit: f64,
    pub upper_limit: f64,
    pub rhs_penetration: f64,
    pub original_contact_point: RowOrigin,
    pub override_num_solver_iterations: i32,
    pub friction_index: i32,
    pub solver_body_id_a: i32,
    pub solver_body_id_b: i32,
}
