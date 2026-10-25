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
// Altered for pz-rs: Rust translation of btSequentialImpulseConstraintSolver
// (Bullet 2.82, double precision, scalar code paths).

//! `btSequentialImpulseConstraintSolver`.
//!
//! [`SequentialImpulseConstraintSolver::solve_group`] takes the object arena,
//! the island's bodies, the manifold arena with the island's manifold ids and
//! the constraint arena with the island's constraint ids, like
//! `solveGroup(bodies, numBodies, manifoldPtr, numManifolds, constraints,
//! numConstraints, infoGlobal, ...)`.

use crate::bt::collision_object::*;
use crate::bt::linear_math::*;
use crate::bt::manifold::{ManifoldId, ManifoldPoint, PersistentManifold};

use super::solver_body::*;
use super::typed_constraint::*;

/// Manifold arena access for the solver.
pub trait ManifoldArena {
    fn manifold(&self, id: ManifoldId) -> &PersistentManifold;
    fn manifold_mut(&mut self, id: ManifoldId) -> &mut PersistentManifold;
}
impl ManifoldArena for [PersistentManifold] {
    fn manifold(&self, id: ManifoldId) -> &PersistentManifold {
        &self[id]
    }
    fn manifold_mut(&mut self, id: ManifoldId) -> &mut PersistentManifold {
        &mut self[id]
    }
}
impl ManifoldArena for Vec<PersistentManifold> {
    fn manifold(&self, id: ManifoldId) -> &PersistentManifold {
        &self[id]
    }
    fn manifold_mut(&mut self, id: ManifoldId) -> &mut PersistentManifold {
        &mut self[id]
    }
}
impl ManifoldArena for Vec<Option<PersistentManifold>> {
    fn manifold(&self, id: ManifoldId) -> &PersistentManifold {
        self[id].as_ref().expect("dangling manifold id")
    }
    fn manifold_mut(&mut self, id: ManifoldId) -> &mut PersistentManifold {
        self[id].as_mut().expect("dangling manifold id")
    }
}

/// `btSequentialImpulseConstraintSolver`.
#[derive(Clone, Debug, Default)]
pub struct SequentialImpulseConstraintSolver {
    pub tmp_solver_body_pool: Vec<SolverBody>,
    pub tmp_solver_contact_constraint_pool: Vec<SolverConstraint>,
    pub tmp_solver_non_contact_constraint_pool: Vec<SolverConstraint>,
    pub tmp_solver_contact_friction_constraint_pool: Vec<SolverConstraint>,
    pub tmp_solver_contact_rolling_friction_constraint_pool: Vec<SolverConstraint>,
    pub order_tmp_constraint_pool: Vec<i32>,
    pub order_non_contact_constraint_pool: Vec<i32>,
    pub order_friction_constraint_pool: Vec<i32>,
    pub tmp_constraint_sizes_pool: Vec<ConstraintInfo1>,
    pub max_override_num_solver_iterations: i32,
    pub fixed_body_id: i32,
    /// `m_btSeed2` (`unsigned long`, 64-bit).
    pub bt_seed2: u64,
}

/// The three row resolvers of Bullet. Body indices may be equal: both
/// halves then act on the same solver body in turn, like the C++ aliases.
#[derive(Clone, Copy)]
enum RowKind {
    /// `resolveSingleConstraintRowGeneric`.
    Generic,
    /// `resolveSingleConstraintRowLowerLimit`.
    LowerLimit,
    /// `resolveSplitPenetrationImpulseCacheFriendly`.
    Split,
}

#[inline]
fn resolve_row(pool: &mut [SolverBody], c: &mut SolverConstraint, kind: RowKind) {
    let ia = c.solver_body_id_a as usize;
    let ib = c.solver_body_id_b as usize;
    match kind {
        RowKind::Generic | RowKind::LowerLimit => {
            let mut delta_impulse = fma(-c.applied_impulse, c.cfm, c.rhs);
            let dv1 = c.contact_normal1.dot(&pool[ia].delta_linear_velocity)
                + c.relpos1_cross_normal.dot(&pool[ia].delta_angular_velocity);
            let dv2 = c.contact_normal2.dot(&pool[ib].delta_linear_velocity)
                + c.relpos2_cross_normal.dot(&pool[ib].delta_angular_velocity);
            delta_impulse = fma(-dv1, c.jac_diag_ab_inv, delta_impulse);
            delta_impulse = fma(-dv2, c.jac_diag_ab_inv, delta_impulse);
            let sum = c.applied_impulse + delta_impulse;
            if sum < c.lower_limit {
                delta_impulse = c.lower_limit - c.applied_impulse;
                c.applied_impulse = c.lower_limit;
            } else if matches!(kind, RowKind::Generic) && sum > c.upper_limit {
                delta_impulse = c.upper_limit - c.applied_impulse;
                c.applied_impulse = c.upper_limit;
            } else {
                c.applied_impulse = sum;
            }
            let lin_a = c.contact_normal1 * pool[ia].inv_mass;
            pool[ia].internal_apply_impulse(&lin_a, &c.angular_component_a, delta_impulse);
            let lin_b = c.contact_normal2 * pool[ib].inv_mass;
            pool[ib].internal_apply_impulse(&lin_b, &c.angular_component_b, delta_impulse);
        }
        RowKind::Split => {
            if c.rhs_penetration != 0.0 {
                let mut delta_impulse = fma(-c.applied_push_impulse, c.cfm, c.rhs_penetration);
                let dv1 = c.contact_normal1.dot(&pool[ia].push_velocity)
                    + c.relpos1_cross_normal.dot(&pool[ia].turn_velocity);
                let dv2 = c.contact_normal2.dot(&pool[ib].push_velocity)
                    + c.relpos2_cross_normal.dot(&pool[ib].turn_velocity);
                delta_impulse = fma(-dv1, c.jac_diag_ab_inv, delta_impulse);
                delta_impulse = fma(-dv2, c.jac_diag_ab_inv, delta_impulse);
                let sum = c.applied_push_impulse + delta_impulse;
                if sum < c.lower_limit {
                    delta_impulse = c.lower_limit - c.applied_push_impulse;
                    c.applied_push_impulse = c.lower_limit;
                } else {
                    c.applied_push_impulse = sum;
                }
                let lin_a = c.contact_normal1 * pool[ia].inv_mass;
                pool[ia].internal_apply_push_impulse(&lin_a, &c.angular_component_a, delta_impulse);
                let lin_b = c.contact_normal2 * pool[ib].inv_mass;
                pool[ib].internal_apply_push_impulse(&lin_b, &c.angular_component_b, delta_impulse);
            }
        }
    }
}

impl SequentialImpulseConstraintSolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// `reset`.
    pub fn reset(&mut self) {
        self.bt_seed2 = 0;
    }

    /// `btRand2`.
    pub fn bt_rand2(&mut self) -> u64 {
        self.bt_seed2 = (1664525u64
            .wrapping_mul(self.bt_seed2)
            .wrapping_add(1013904223))
            & 0xffff_ffff;
        self.bt_seed2
    }

    /// `btRandInt2`.
    pub fn bt_rand_int2(&mut self, n: i32) -> i32 {
        let un = n as i64 as u64;
        let mut r = self.bt_rand2();
        if un <= 0x0001_0000 {
            r ^= r >> 16;
            if un <= 0x0000_0100 {
                r ^= r >> 8;
                if un <= 0x0000_0010 {
                    r ^= r >> 4;
                    if un <= 0x0000_0004 {
                        r ^= r >> 2;
                        if un <= 0x0000_0002 {
                            r ^= r >> 1;
                        }
                    }
                }
            }
        }
        (r % un) as i32
    }

    /// `initSolverBody`.
    fn init_solver_body(
        solver_body: &mut SolverBody,
        obj: Option<(ObjId, &CollisionObject)>,
        time_step: f64,
    ) {
        solver_body.delta_linear_velocity.set_value(0.0, 0.0, 0.0);
        solver_body.delta_angular_velocity.set_value(0.0, 0.0, 0.0);
        solver_body.push_velocity.set_value(0.0, 0.0, 0.0);
        solver_body.turn_velocity.set_value(0.0, 0.0, 0.0);
        match obj.filter(|(_, o)| o.is_rigid_body()) {
            Some((id, o)) => {
                let rb = o.rb();
                solver_body.world_transform = o.world_transform;
                solver_body.inv_mass =
                    Vec3::new(rb.inverse_mass, rb.inverse_mass, rb.inverse_mass) * rb.linear_factor;
                solver_body.original_body = Some(id);
                solver_body.angular_factor = rb.angular_factor;
                solver_body.linear_factor = rb.linear_factor;
                solver_body.linear_velocity = rb.linear_velocity;
                solver_body.angular_velocity = rb.angular_velocity;
                solver_body.external_force_impulse = rb.total_force * rb.inverse_mass * time_step;
                solver_body.external_torque_impulse =
                    rb.total_torque * rb.inv_inertia_tensor_world * time_step;
            }
            None => {
                solver_body.world_transform.set_identity();
                solver_body.inv_mass = Vec3::new(0.0, 0.0, 0.0);
                solver_body.original_body = None;
                solver_body.angular_factor.set_value(1.0, 1.0, 1.0);
                solver_body.linear_factor.set_value(1.0, 1.0, 1.0);
                solver_body.linear_velocity.set_value(0.0, 0.0, 0.0);
                solver_body.angular_velocity.set_value(0.0, 0.0, 0.0);
                solver_body.external_force_impulse.set_value(0.0, 0.0, 0.0);
                solver_body.external_torque_impulse.set_value(0.0, 0.0, 0.0);
            }
        }
    }

    /// `getOrInitSolverBody(body, timeStep)`; `None` is the constraints'
    /// fixed body.
    pub fn get_or_init_solver_body<B: BodyArena + ?Sized>(
        &mut self,
        objects: &mut B,
        body: Option<ObjId>,
        time_step: f64,
    ) -> i32 {
        let companion = match body {
            Some(id) => objects.obj(id).companion_id,
            None => fixed_body().companion_id,
        };
        if companion >= 0 {
            return companion;
        }
        let o = body_ref(objects, body);
        let dynamic = o.is_rigid_body() && (o.rb().inverse_mass != 0.0 || o.is_kinematic_object());
        if dynamic {
            let id = body.expect("fixed body is static");
            let solver_body_id = self.tmp_solver_body_pool.len() as i32;
            let mut sb = SolverBody::default();
            Self::init_solver_body(&mut sb, Some((id, objects.obj(id))), time_step);
            self.tmp_solver_body_pool.push(sb);
            objects.obj_mut(id).companion_id = solver_body_id;
            solver_body_id
        } else {
            if self.fixed_body_id < 0 {
                self.fixed_body_id = self.tmp_solver_body_pool.len() as i32;
                let mut sb = SolverBody::default();
                Self::init_solver_body(&mut sb, None, time_step);
                self.tmp_solver_body_pool.push(sb);
            }
            self.fixed_body_id
        }
    }

    /// `applyAnisotropicFriction`.
    fn apply_anisotropic_friction(
        col_obj: &CollisionObject,
        friction_direction: &mut Vec3,
        friction_mode: i32,
    ) {
        if col_obj.has_anisotropic_friction(friction_mode) {
            let mut loc_lateral = *friction_direction * col_obj.world_transform.basis;
            loc_lateral.mul_assign_vec(&col_obj.anisotropic_friction);
            *friction_direction = col_obj.world_transform.basis * loc_lateral;
        }
    }

    /// `setupFrictionConstraint`.
    #[allow(clippy::too_many_arguments)]
    fn setup_friction_constraint<B: BodyArena + ?Sized>(
        &self,
        objects: &B,
        sc: &mut SolverConstraint,
        normal_axis: &Vec3,
        solver_body_id_a: i32,
        solver_body_id_b: i32,
        cp: &ManifoldPoint,
        rel_pos1: &Vec3,
        rel_pos2: &Vec3,
        relaxation: f64,
        desired_velocity: f64,
        cfm_slip: f64,
    ) {
        let body_a = &self.tmp_solver_body_pool[solver_body_id_a as usize];
        let body_b = &self.tmp_solver_body_pool[solver_body_id_b as usize];
        let body0 = body_a.original_body.map(|id| objects.obj(id));
        let body1 = body_b.original_body.map(|id| objects.obj(id));
        sc.solver_body_id_a = solver_body_id_a;
        sc.solver_body_id_b = solver_body_id_b;
        sc.friction = cp.combined_friction;
        sc.original_contact_point = RowOrigin::None;
        sc.applied_impulse = 0.0;
        sc.applied_push_impulse = 0.0;
        if let Some(b0) = body0 {
            sc.contact_normal1 = *normal_axis;
            let ftorque_axis1 = rel_pos1.cross(&sc.contact_normal1);
            sc.relpos1_cross_normal = ftorque_axis1;
            sc.angular_component_a =
                b0.rb().inv_inertia_tensor_world * ftorque_axis1 * b0.rb().angular_factor;
        } else {
            sc.contact_normal1.set_zero();
            sc.relpos1_cross_normal.set_zero();
            sc.angular_component_a.set_zero();
        }
        if let Some(b1) = body1 {
            sc.contact_normal2 = -*normal_axis;
            let ftorque_axis1 = rel_pos2.cross(&sc.contact_normal2);
            sc.relpos2_cross_normal = ftorque_axis1;
            sc.angular_component_b =
                b1.rb().inv_inertia_tensor_world * ftorque_axis1 * b1.rb().angular_factor;
        } else {
            sc.contact_normal2.set_zero();
            sc.relpos2_cross_normal.set_zero();
            sc.angular_component_b.set_zero();
        }
        {
            let mut denom0 = 0.0;
            let mut denom1 = 0.0;
            if let Some(b0) = body0 {
                let vec = sc.angular_component_a.cross(rel_pos1);
                denom0 = b0.rb().inverse_mass + normal_axis.dot(&vec);
            }
            if let Some(b1) = body1 {
                let vec = (-sc.angular_component_b).cross(rel_pos2);
                denom1 = b1.rb().inverse_mass + normal_axis.dot(&vec);
            }
            sc.jac_diag_ab_inv = relaxation / (denom0 + denom1);
        }
        {
            let v1 = if body0.is_some() {
                body_a.linear_velocity + body_a.external_force_impulse
            } else {
                Vec3::ZERO
            };
            let w1 = if body0.is_some() {
                body_a.angular_velocity
            } else {
                Vec3::ZERO
            };
            let v2 = if body1.is_some() {
                body_b.linear_velocity + body_b.external_force_impulse
            } else {
                Vec3::ZERO
            };
            let w2 = if body1.is_some() {
                body_b.angular_velocity
            } else {
                Vec3::ZERO
            };
            let vel1_dotn = sc.contact_normal1.dot(&v1) + sc.relpos1_cross_normal.dot(&w1);
            let vel2_dotn = sc.contact_normal2.dot(&v2) + sc.relpos2_cross_normal.dot(&w2);
            let rel_vel = vel1_dotn + vel2_dotn;
            let velocity_error = desired_velocity - rel_vel;
            let velocity_impulse = velocity_error * sc.jac_diag_ab_inv;
            sc.rhs = velocity_impulse;
            sc.cfm = cfm_slip;
            sc.lower_limit = -sc.friction;
            sc.upper_limit = sc.friction;
        }
    }

    /// `addFrictionConstraint`.
    #[allow(clippy::too_many_arguments)]
    fn add_friction_constraint<B: BodyArena + ?Sized>(
        &mut self,
        objects: &B,
        normal_axis: &Vec3,
        solver_body_id_a: i32,
        solver_body_id_b: i32,
        friction_index: i32,
        cp: &ManifoldPoint,
        rel_pos1: &Vec3,
        rel_pos2: &Vec3,
        relaxation: f64,
        desired_velocity: f64,
        cfm_slip: f64,
    ) {
        let mut sc = SolverConstraint {
            friction_index,
            ..Default::default()
        };
        self.setup_friction_constraint(
            objects,
            &mut sc,
            normal_axis,
            solver_body_id_a,
            solver_body_id_b,
            cp,
            rel_pos1,
            rel_pos2,
            relaxation,
            desired_velocity,
            cfm_slip,
        );
        self.tmp_solver_contact_friction_constraint_pool.push(sc);
    }

    /// `setupRollingFrictionConstraint`.
    #[allow(clippy::too_many_arguments)]
    fn setup_rolling_friction_constraint<B: BodyArena + ?Sized>(
        &self,
        objects: &B,
        sc: &mut SolverConstraint,
        normal_axis1: &Vec3,
        solver_body_id_a: i32,
        solver_body_id_b: i32,
        cp: &ManifoldPoint,
        desired_velocity: f64,
        cfm_slip: f64,
    ) {
        let normal_axis = Vec3::new(0.0, 0.0, 0.0);
        sc.contact_normal1 = normal_axis;
        sc.contact_normal2 = -normal_axis;
        let body_a = &self.tmp_solver_body_pool[solver_body_id_a as usize];
        let body_b = &self.tmp_solver_body_pool[solver_body_id_b as usize];
        let body0 = body_a.original_body.map(|id| objects.obj(id));
        let body1 = body_b.original_body.map(|id| objects.obj(id));
        sc.solver_body_id_a = solver_body_id_a;
        sc.solver_body_id_b = solver_body_id_b;
        sc.friction = cp.combined_rolling_friction;
        sc.original_contact_point = RowOrigin::None;
        sc.applied_impulse = 0.0;
        sc.applied_push_impulse = 0.0;
        {
            let ftorque_axis1 = -*normal_axis1;
            sc.relpos1_cross_normal = ftorque_axis1;
            sc.angular_component_a = match body0 {
                Some(b0) => {
                    b0.rb().inv_inertia_tensor_world * ftorque_axis1 * b0.rb().angular_factor
                }
                None => Vec3::ZERO,
            };
        }
        {
            let ftorque_axis1 = *normal_axis1;
            sc.relpos2_cross_normal = ftorque_axis1;
            sc.angular_component_b = match body1 {
                Some(b1) => {
                    b1.rb().inv_inertia_tensor_world * ftorque_axis1 * b1.rb().angular_factor
                }
                None => Vec3::ZERO,
            };
        }
        {
            let i_mja_a = match body0 {
                Some(b0) => b0.rb().inv_inertia_tensor_world * sc.relpos1_cross_normal,
                None => Vec3::ZERO,
            };
            let i_mja_b = match body1 {
                Some(b1) => b1.rb().inv_inertia_tensor_world * sc.relpos2_cross_normal,
                None => Vec3::ZERO,
            };
            let mut sum = 0.0;
            sum += i_mja_a.dot(&sc.relpos1_cross_normal);
            sum += i_mja_b.dot(&sc.relpos2_cross_normal);
            sc.jac_diag_ab_inv = 1.0 / sum;
        }
        {
            let v1 = if body0.is_some() {
                body_a.linear_velocity + body_a.external_force_impulse
            } else {
                Vec3::ZERO
            };
            let w1 = if body0.is_some() {
                body_a.angular_velocity
            } else {
                Vec3::ZERO
            };
            let v2 = if body1.is_some() {
                body_b.linear_velocity + body_b.external_force_impulse
            } else {
                Vec3::ZERO
            };
            let w2 = if body1.is_some() {
                body_b.angular_velocity
            } else {
                Vec3::ZERO
            };
            let vel1_dotn = sc.contact_normal1.dot(&v1) + sc.relpos1_cross_normal.dot(&w1);
            let vel2_dotn = sc.contact_normal2.dot(&v2) + sc.relpos2_cross_normal.dot(&w2);
            let rel_vel = vel1_dotn + vel2_dotn;
            let velocity_error = desired_velocity - rel_vel;
            sc.rhs = velocity_error * sc.jac_diag_ab_inv;
            sc.cfm = cfm_slip;
            sc.lower_limit = -sc.friction;
            sc.upper_limit = sc.friction;
        }
    }

    /// `addRollingFrictionConstraint`.
    #[allow(clippy::too_many_arguments)]
    fn add_rolling_friction_constraint<B: BodyArena + ?Sized>(
        &mut self,
        objects: &B,
        normal_axis: &Vec3,
        solver_body_id_a: i32,
        solver_body_id_b: i32,
        friction_index: i32,
        cp: &ManifoldPoint,
    ) {
        let mut sc = SolverConstraint {
            friction_index,
            ..Default::default()
        };
        self.setup_rolling_friction_constraint(
            objects,
            &mut sc,
            normal_axis,
            solver_body_id_a,
            solver_body_id_b,
            cp,
            0.0,
            0.0,
        );
        self.tmp_solver_contact_rolling_friction_constraint_pool
            .push(sc);
    }

    /// `setupContactConstraint`; returns `relaxation`.
    #[allow(clippy::too_many_arguments)]
    fn setup_contact_constraint<B: BodyArena + ?Sized>(
        &mut self,
        objects: &B,
        sc: &mut SolverConstraint,
        solver_body_id_a: i32,
        solver_body_id_b: i32,
        cp: &ManifoldPoint,
        info: &ContactSolverInfo,
        rel_pos1: &Vec3,
        rel_pos2: &Vec3,
    ) -> f64 {
        let rb0_id = self.tmp_solver_body_pool[solver_body_id_a as usize].original_body;
        let rb1_id = self.tmp_solver_body_pool[solver_body_id_b as usize].original_body;
        let rb0 = rb0_id.map(|id| objects.obj(id));
        let rb1 = rb1_id.map(|id| objects.obj(id));
        let relaxation = 1.0;
        let torque_axis0 = rel_pos1.cross(&cp.normal_world_on_b);
        sc.angular_component_a = match rb0 {
            Some(b) => b.rb().inv_inertia_tensor_world * torque_axis0 * b.rb().angular_factor,
            None => Vec3::ZERO,
        };
        let torque_axis1 = rel_pos2.cross(&cp.normal_world_on_b);
        sc.angular_component_b = match rb1 {
            Some(b) => b.rb().inv_inertia_tensor_world * -torque_axis1 * b.rb().angular_factor,
            None => Vec3::ZERO,
        };
        {
            let mut denom0 = 0.0;
            let mut denom1 = 0.0;
            if let Some(b) = rb0 {
                let vec = sc.angular_component_a.cross(rel_pos1);
                denom0 = b.rb().inverse_mass + cp.normal_world_on_b.dot(&vec);
            }
            if let Some(b) = rb1 {
                let vec = (-sc.angular_component_b).cross(rel_pos2);
                denom1 = b.rb().inverse_mass + cp.normal_world_on_b.dot(&vec);
            }
            sc.jac_diag_ab_inv = relaxation / (denom0 + denom1);
        }
        if rb0.is_some() {
            sc.contact_normal1 = cp.normal_world_on_b;
            sc.relpos1_cross_normal = torque_axis0;
        } else {
            sc.contact_normal1.set_zero();
            sc.relpos1_cross_normal.set_zero();
        }
        if rb1.is_some() {
            sc.contact_normal2 = -cp.normal_world_on_b;
            sc.relpos2_cross_normal = -torque_axis1;
        } else {
            sc.contact_normal2.set_zero();
            sc.relpos2_cross_normal.set_zero();
        }
        let mut restitution;
        let penetration = cp.get_distance() + info.linear_slop;
        {
            let vel1 = match rb0 {
                Some(b) => b.get_velocity_in_local_point(rel_pos1),
                None => Vec3::ZERO,
            };
            let vel2 = match rb1 {
                Some(b) => b.get_velocity_in_local_point(rel_pos2),
                None => Vec3::ZERO,
            };
            let vel = vel1 - vel2;
            let rel_vel = cp.normal_world_on_b.dot(&vel);
            sc.friction = cp.combined_friction;
            restitution = cp.combined_restitution * -rel_vel;
            if restitution <= 0.0 {
                restitution = 0.0;
            }
        }
        let lf0 = rb0.map(|b| b.rb().linear_factor);
        let lf1 = rb1.map(|b| b.rb().linear_factor);
        if info.solver_mode & SOLVER_USE_WARMSTARTING != 0 {
            sc.applied_impulse = cp.applied_impulse * info.warmstarting_factor;
            if let Some(lf) = lf0 {
                let body_a = &mut self.tmp_solver_body_pool[solver_body_id_a as usize];
                let lin = sc.contact_normal1 * body_a.inv_mass * lf;
                body_a.internal_apply_impulse(&lin, &sc.angular_component_a, sc.applied_impulse);
            }
            if let Some(lf) = lf1 {
                let body_b = &mut self.tmp_solver_body_pool[solver_body_id_b as usize];
                let lin = -sc.contact_normal2 * body_b.inv_mass * lf;
                body_b.internal_apply_impulse(
                    &lin,
                    &(-sc.angular_component_b),
                    -sc.applied_impulse,
                );
            }
        } else {
            sc.applied_impulse = 0.0;
        }
        sc.applied_push_impulse = 0.0;
        {
            let body_a = &self.tmp_solver_body_pool[solver_body_id_a as usize];
            let body_b = &self.tmp_solver_body_pool[solver_body_id_b as usize];
            let ext_fa = if body_a.original_body.is_some() {
                body_a.external_force_impulse
            } else {
                Vec3::ZERO
            };
            let ext_ta = if body_a.original_body.is_some() {
                body_a.external_torque_impulse
            } else {
                Vec3::ZERO
            };
            let ext_fb = if body_b.original_body.is_some() {
                body_b.external_force_impulse
            } else {
                Vec3::ZERO
            };
            let ext_tb = if body_b.original_body.is_some() {
                body_b.external_torque_impulse
            } else {
                Vec3::ZERO
            };
            let vel1_dotn = sc.contact_normal1.dot(&(body_a.linear_velocity + ext_fa))
                + sc.relpos1_cross_normal
                    .dot(&(body_a.angular_velocity + ext_ta));
            let vel2_dotn = sc.contact_normal2.dot(&(body_b.linear_velocity + ext_fb))
                + sc.relpos2_cross_normal
                    .dot(&(body_b.angular_velocity + ext_tb));
            let rel_vel = vel1_dotn + vel2_dotn;
            let positional_error;
            let mut velocity_error = restitution - rel_vel;
            let mut erp = info.erp2;
            if info.split_impulse == 0 || penetration > info.split_impulse_penetration_threshold {
                erp = info.erp;
            }
            if penetration > 0.0 {
                positional_error = 0.0;
                velocity_error -= penetration / info.time_step;
            } else {
                positional_error = -penetration * erp / info.time_step;
            }
            let penetration_impulse = positional_error * sc.jac_diag_ab_inv;
            let velocity_impulse = velocity_error * sc.jac_diag_ab_inv;
            if info.split_impulse == 0 || penetration > info.split_impulse_penetration_threshold {
                sc.rhs = penetration_impulse + velocity_impulse;
                sc.rhs_penetration = 0.0;
            } else {
                sc.rhs = velocity_impulse;
                sc.rhs_penetration = penetration_impulse;
            }
            sc.cfm = 0.0;
            sc.lower_limit = 0.0;
            sc.upper_limit = flt(1e10);
        }
        relaxation
    }

    /// `setFrictionConstraintImpulse`.
    fn set_friction_constraint_impulse<B: BodyArena + ?Sized>(
        &mut self,
        objects: &B,
        sc: &SolverConstraint,
        solver_body_id_a: i32,
        solver_body_id_b: i32,
        cp: &ManifoldPoint,
        info: &ContactSolverInfo,
    ) {
        let rb0 = self.tmp_solver_body_pool[solver_body_id_a as usize]
            .original_body
            .map(|id| objects.obj(id).rb());
        let rb1 = self.tmp_solver_body_pool[solver_body_id_b as usize]
            .original_body
            .map(|id| objects.obj(id).rb());
        let (im0, lf0) = rb0
            .map(|r| (r.inverse_mass, r.linear_factor))
            .unwrap_or_default();
        let (im1, lf1) = rb1
            .map(|r| (r.inverse_mass, r.linear_factor))
            .unwrap_or_default();
        {
            let fi = sc.friction_index as usize;
            if info.solver_mode & SOLVER_USE_WARMSTARTING != 0 {
                let fc = &mut self.tmp_solver_contact_friction_constraint_pool[fi];
                fc.applied_impulse = cp.applied_impulse_lateral1 * info.warmstarting_factor;
                let fc = *fc;
                if rb0.is_some() {
                    self.tmp_solver_body_pool[solver_body_id_a as usize].internal_apply_impulse(
                        &(fc.contact_normal1 * im0 * lf0),
                        &fc.angular_component_a,
                        fc.applied_impulse,
                    );
                }
                if rb1.is_some() {
                    self.tmp_solver_body_pool[solver_body_id_b as usize].internal_apply_impulse(
                        &(-fc.contact_normal2 * im1 * lf1),
                        &(-fc.angular_component_b),
                        -fc.applied_impulse,
                    );
                }
            } else {
                self.tmp_solver_contact_friction_constraint_pool[fi].applied_impulse = 0.0;
            }
        }
        if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0 {
            let fi = sc.friction_index as usize + 1;
            if info.solver_mode & SOLVER_USE_WARMSTARTING != 0 {
                let fc = &mut self.tmp_solver_contact_friction_constraint_pool[fi];
                fc.applied_impulse = cp.applied_impulse_lateral2 * info.warmstarting_factor;
                let fc = *fc;
                if rb0.is_some() {
                    self.tmp_solver_body_pool[solver_body_id_a as usize].internal_apply_impulse(
                        &(fc.contact_normal1 * im0),
                        &fc.angular_component_a,
                        fc.applied_impulse,
                    );
                }
                if rb1.is_some() {
                    self.tmp_solver_body_pool[solver_body_id_b as usize].internal_apply_impulse(
                        &(-fc.contact_normal2 * im1),
                        &(-fc.angular_component_b),
                        -fc.applied_impulse,
                    );
                }
            } else {
                self.tmp_solver_contact_friction_constraint_pool[fi].applied_impulse = 0.0;
            }
        }
    }

    /// `convertContact`.
    fn convert_contact<B: BodyArena + ?Sized, M: ManifoldArena + ?Sized>(
        &mut self,
        objects: &mut B,
        manifolds: &mut M,
        manifold_id: ManifoldId,
        info: &ContactSolverInfo,
    ) {
        let (col0, col1, num, cpt) = {
            let m = manifolds.manifold(manifold_id);
            (
                m.body0,
                m.body1,
                m.get_num_contacts(),
                m.contact_processing_threshold,
            )
        };
        let solver_body_id_a = self.get_or_init_solver_body(objects, Some(col0), info.time_step);
        let solver_body_id_b = self.get_or_init_solver_body(objects, Some(col1), info.time_step);
        {
            let a = &self.tmp_solver_body_pool[solver_body_id_a as usize];
            let b = &self.tmp_solver_body_pool[solver_body_id_b as usize];
            if a.inv_mass.is_zero() && b.inv_mass.is_zero() {
                return;
            }
        }
        let objects: &B = objects;
        let o0 = objects.obj(col0);
        let o1 = objects.obj(col1);
        let mut rolling_friction = 1;
        for j in 0..num as usize {
            let mut cp = *manifolds.manifold(manifold_id).get_contact_point(j);
            if cp.get_distance() <= cpt {
                let friction_index = self.tmp_solver_contact_constraint_pool.len() as i32;
                let mut sc = SolverConstraint {
                    solver_body_id_a,
                    solver_body_id_b,
                    original_contact_point: RowOrigin::Contact(manifold_id, j),
                    ..Default::default()
                };
                let pos1 = cp.position_world_on_a;
                let pos2 = cp.position_world_on_b;
                let rel_pos1 = pos1 - o0.world_transform.origin;
                let rel_pos2 = pos2 - o1.world_transform.origin;
                let vel1 = self.tmp_solver_body_pool[solver_body_id_a as usize]
                    .get_velocity_in_local_point_no_delta(&rel_pos1);
                let vel2 = self.tmp_solver_body_pool[solver_body_id_b as usize]
                    .get_velocity_in_local_point_no_delta(&rel_pos2);
                let vel = vel1 - vel2;
                let rel_vel = cp.normal_world_on_b.dot(&vel);
                let relaxation = self.setup_contact_constraint(
                    objects,
                    &mut sc,
                    solver_body_id_a,
                    solver_body_id_b,
                    &cp,
                    info,
                    &rel_pos1,
                    &rel_pos2,
                );
                sc.friction_index = self.tmp_solver_contact_friction_constraint_pool.len() as i32;
                // The contact row lives in the pool from here on (Bullet
                // expands the pool before filling it).
                self.tmp_solver_contact_constraint_pool.push(sc);

                let ang_vel_a = if o0.is_rigid_body() {
                    o0.rb().angular_velocity
                } else {
                    Vec3::ZERO
                };
                let ang_vel_b = if o1.is_rigid_body() {
                    o1.rb().angular_velocity
                } else {
                    Vec3::ZERO
                };
                let mut rel_ang_vel = ang_vel_b - ang_vel_a;
                if cp.combined_rolling_friction > 0.0 && rolling_friction > 0 {
                    rolling_friction -= 1;
                    if rel_ang_vel.length() > info.single_axis_rolling_friction_threshold {
                        rel_ang_vel.normalize();
                        Self::apply_anisotropic_friction(
                            o0,
                            &mut rel_ang_vel,
                            CF_ANISOTROPIC_ROLLING_FRICTION,
                        );
                        Self::apply_anisotropic_friction(
                            o1,
                            &mut rel_ang_vel,
                            CF_ANISOTROPIC_ROLLING_FRICTION,
                        );
                        if rel_ang_vel.length() > 0.001 {
                            self.add_rolling_friction_constraint(
                                objects,
                                &rel_ang_vel,
                                solver_body_id_a,
                                solver_body_id_b,
                                friction_index,
                                &cp,
                            );
                        }
                    } else {
                        let n = cp.normal_world_on_b;
                        self.add_rolling_friction_constraint(
                            objects,
                            &n,
                            solver_body_id_a,
                            solver_body_id_b,
                            friction_index,
                            &cp,
                        );
                        let mut axis0 = Vec3::ZERO;
                        let mut axis1 = Vec3::ZERO;
                        plane_space1(&cp.normal_world_on_b, &mut axis0, &mut axis1);
                        Self::apply_anisotropic_friction(
                            o0,
                            &mut axis0,
                            CF_ANISOTROPIC_ROLLING_FRICTION,
                        );
                        Self::apply_anisotropic_friction(
                            o1,
                            &mut axis0,
                            CF_ANISOTROPIC_ROLLING_FRICTION,
                        );
                        Self::apply_anisotropic_friction(
                            o0,
                            &mut axis1,
                            CF_ANISOTROPIC_ROLLING_FRICTION,
                        );
                        Self::apply_anisotropic_friction(
                            o1,
                            &mut axis1,
                            CF_ANISOTROPIC_ROLLING_FRICTION,
                        );
                        if axis0.length() > 0.001 {
                            self.add_rolling_friction_constraint(
                                objects,
                                &axis0,
                                solver_body_id_a,
                                solver_body_id_b,
                                friction_index,
                                &cp,
                            );
                        }
                        if axis1.length() > 0.001 {
                            self.add_rolling_friction_constraint(
                                objects,
                                &axis1,
                                solver_body_id_a,
                                solver_body_id_b,
                                friction_index,
                                &cp,
                            );
                        }
                    }
                }

                if info.solver_mode & SOLVER_ENABLE_FRICTION_DIRECTION_CACHING == 0
                    || !cp.lateral_friction_initialized
                {
                    cp.lateral_friction_dir1 = vel - cp.normal_world_on_b * rel_vel;
                    let lat_rel_vel = cp.lateral_friction_dir1.length2();
                    if info.solver_mode & SOLVER_DISABLE_VELOCITY_DEPENDENT_FRICTION_DIRECTION == 0
                        && lat_rel_vel > SIMD_EPSILON
                    {
                        cp.lateral_friction_dir1 *= 1.0 / lat_rel_vel.sqrt();
                        Self::apply_anisotropic_friction(
                            o0,
                            &mut cp.lateral_friction_dir1,
                            CF_ANISOTROPIC_FRICTION,
                        );
                        Self::apply_anisotropic_friction(
                            o1,
                            &mut cp.lateral_friction_dir1,
                            CF_ANISOTROPIC_FRICTION,
                        );
                        let d = cp.lateral_friction_dir1;
                        self.add_friction_constraint(
                            objects,
                            &d,
                            solver_body_id_a,
                            solver_body_id_b,
                            friction_index,
                            &cp,
                            &rel_pos1,
                            &rel_pos2,
                            relaxation,
                            0.0,
                            0.0,
                        );
                        if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0 {
                            cp.lateral_friction_dir2 =
                                cp.lateral_friction_dir1.cross(&cp.normal_world_on_b);
                            cp.lateral_friction_dir2.normalize();
                            Self::apply_anisotropic_friction(
                                o0,
                                &mut cp.lateral_friction_dir2,
                                CF_ANISOTROPIC_FRICTION,
                            );
                            Self::apply_anisotropic_friction(
                                o1,
                                &mut cp.lateral_friction_dir2,
                                CF_ANISOTROPIC_FRICTION,
                            );
                            let d = cp.lateral_friction_dir2;
                            self.add_friction_constraint(
                                objects,
                                &d,
                                solver_body_id_a,
                                solver_body_id_b,
                                friction_index,
                                &cp,
                                &rel_pos1,
                                &rel_pos2,
                                relaxation,
                                0.0,
                                0.0,
                            );
                        }
                    } else {
                        let n = cp.normal_world_on_b;
                        plane_space1(
                            &n,
                            &mut cp.lateral_friction_dir1,
                            &mut cp.lateral_friction_dir2,
                        );
                        Self::apply_anisotropic_friction(
                            o0,
                            &mut cp.lateral_friction_dir1,
                            CF_ANISOTROPIC_FRICTION,
                        );
                        Self::apply_anisotropic_friction(
                            o1,
                            &mut cp.lateral_friction_dir1,
                            CF_ANISOTROPIC_FRICTION,
                        );
                        let d = cp.lateral_friction_dir1;
                        self.add_friction_constraint(
                            objects,
                            &d,
                            solver_body_id_a,
                            solver_body_id_b,
                            friction_index,
                            &cp,
                            &rel_pos1,
                            &rel_pos2,
                            relaxation,
                            0.0,
                            0.0,
                        );
                        if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0 {
                            Self::apply_anisotropic_friction(
                                o0,
                                &mut cp.lateral_friction_dir2,
                                CF_ANISOTROPIC_FRICTION,
                            );
                            Self::apply_anisotropic_friction(
                                o1,
                                &mut cp.lateral_friction_dir2,
                                CF_ANISOTROPIC_FRICTION,
                            );
                            let d = cp.lateral_friction_dir2;
                            self.add_friction_constraint(
                                objects,
                                &d,
                                solver_body_id_a,
                                solver_body_id_b,
                                friction_index,
                                &cp,
                                &rel_pos1,
                                &rel_pos2,
                                relaxation,
                                0.0,
                                0.0,
                            );
                        }
                        if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0
                            && info.solver_mode
                                & SOLVER_DISABLE_VELOCITY_DEPENDENT_FRICTION_DIRECTION
                                != 0
                        {
                            cp.lateral_friction_initialized = true;
                        }
                    }
                } else {
                    let d = cp.lateral_friction_dir1;
                    self.add_friction_constraint(
                        objects,
                        &d,
                        solver_body_id_a,
                        solver_body_id_b,
                        friction_index,
                        &cp,
                        &rel_pos1,
                        &rel_pos2,
                        relaxation,
                        cp.contact_motion1,
                        cp.contact_cfm1,
                    );
                    if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0 {
                        let d = cp.lateral_friction_dir2;
                        self.add_friction_constraint(
                            objects,
                            &d,
                            solver_body_id_a,
                            solver_body_id_b,
                            friction_index,
                            &cp,
                            &rel_pos1,
                            &rel_pos2,
                            relaxation,
                            cp.contact_motion2,
                            cp.contact_cfm2,
                        );
                    }
                }
                let sc = self.tmp_solver_contact_constraint_pool[friction_index as usize];
                self.set_friction_constraint_impulse(
                    objects,
                    &sc,
                    solver_body_id_a,
                    solver_body_id_b,
                    &cp,
                    info,
                );
                *manifolds.manifold_mut(manifold_id).get_contact_point_mut(j) = cp;
            }
        }
    }

    /// `solveGroupCacheFriendlySetup`.
    #[allow(clippy::too_many_arguments)]
    pub fn solve_group_cache_friendly_setup<B: BodyArena + ?Sized, M: ManifoldArena + ?Sized>(
        &mut self,
        objects: &mut B,
        bodies: &[ObjId],
        manifolds: &mut M,
        manifold_ids: &[ManifoldId],
        constraints: &mut ConstraintArena,
        constraint_ids: &[ConstraintId],
        info: &ContactSolverInfo,
    ) {
        self.fixed_body_id = -1;
        self.max_override_num_solver_iterations = 0;
        for &b in bodies {
            objects.obj_mut(b).companion_id = -1;
        }
        self.tmp_solver_body_pool.clear();
        for &b in bodies {
            let body_id = self.get_or_init_solver_body(objects, Some(b), info.time_step);
            let o = objects.obj(b);
            if o.is_rigid_body()
                && o.rb().inverse_mass != 0.0
                && o.rb().rigidbody_flags & BT_ENABLE_GYROPSCOPIC_FORCE != 0
            {
                let gyro_force = o.compute_gyroscopic_force(info.max_gyroscopic_force);
                let t = gyro_force * o.rb().inv_inertia_tensor_world * info.time_step;
                self.tmp_solver_body_pool[body_id as usize].external_torque_impulse -= t;
            }
        }
        for &cid in constraint_ids {
            let c = constraints.get_mut(cid);
            c.build_jacobian();
            c.applied_impulse = 0.0;
        }
        let mut total_num_rows = 0;
        self.tmp_constraint_sizes_pool.clear();
        self.tmp_constraint_sizes_pool
            .resize(constraint_ids.len(), ConstraintInfo1::default());
        for (i, &cid) in constraint_ids.iter().enumerate() {
            let c = constraints.get_mut(cid);
            if let Some(fb) = c.joint_feedback.as_mut() {
                fb.applied_force_body_a.set_zero();
                fb.applied_torque_body_a.set_zero();
                fb.applied_force_body_b.set_zero();
                fb.applied_torque_body_b.set_zero();
            }
            let mut info1 = ConstraintInfo1::default();
            if c.is_enabled {
                c.get_info1(&*objects, &mut info1);
            } else {
                info1.num_constraint_rows = 0;
                info1.nub = 0;
            }
            self.tmp_constraint_sizes_pool[i] = info1;
            total_num_rows += info1.num_constraint_rows;
        }
        self.tmp_solver_non_contact_constraint_pool.clear();
        self.tmp_solver_non_contact_constraint_pool
            .resize(total_num_rows as usize, SolverConstraint::default());
        let mut current_row = 0usize;
        for (i, &cid) in constraint_ids.iter().enumerate() {
            let info1 = self.tmp_constraint_sizes_pool[i];
            let nrows = info1.num_constraint_rows as usize;
            if nrows != 0 {
                let (rb_a, rb_b) = {
                    let c = constraints.get(cid);
                    (c.rb_a, c.rb_b)
                };
                let solver_body_id_a = self.get_or_init_solver_body(objects, rb_a, info.time_step);
                let solver_body_id_b = self.get_or_init_solver_body(objects, rb_b, info.time_step);
                let c = constraints.get_mut(cid);
                let override_num = if c.override_num_solver_iterations > 0 {
                    c.override_num_solver_iterations
                } else {
                    info.num_iterations
                };
                if override_num > self.max_override_num_solver_iterations {
                    self.max_override_num_solver_iterations = override_num;
                }
                for j in 0..nrows {
                    let r = &mut self.tmp_solver_non_contact_constraint_pool[current_row + j];
                    *r = SolverConstraint::default();
                    r.lower_limit = -SIMD_INFINITY;
                    r.upper_limit = SIMD_INFINITY;
                    r.applied_impulse = 0.0;
                    r.applied_push_impulse = 0.0;
                    r.solver_body_id_a = solver_body_id_a;
                    r.solver_body_id_b = solver_body_id_b;
                    r.override_num_solver_iterations = override_num;
                }
                for sid in [solver_body_id_a, solver_body_id_b] {
                    let sb = &mut self.tmp_solver_body_pool[sid as usize];
                    sb.delta_linear_velocity.set_value(0.0, 0.0, 0.0);
                    sb.delta_angular_velocity.set_value(0.0, 0.0, 0.0);
                    sb.push_velocity.set_value(0.0, 0.0, 0.0);
                    sb.turn_velocity.set_value(0.0, 0.0, 0.0);
                }
                let rows = &mut self.tmp_solver_non_contact_constraint_pool
                    [current_row..current_row + nrows];
                rows[0].cfm = info.global_cfm;
                let mut info2 = ConstraintInfo2 {
                    fps: 1.0 / info.time_step,
                    erp: info.erp,
                    rows,
                    damping: info.damping,
                    num_iterations: info.num_iterations,
                };
                c.get_info2(&*objects, &mut info2);
                let damping = info2.damping;
                let bit = c.breaking_impulse_threshold;
                let oa = body_ref(&*objects, rb_a);
                let ob = body_ref(&*objects, rb_b);
                let (ra, rb) = (oa.rb(), ob.rb());
                let ba = self.tmp_solver_body_pool[solver_body_id_a as usize];
                let bb = self.tmp_solver_body_pool[solver_body_id_b as usize];
                for j in 0..nrows {
                    let sc = &mut self.tmp_solver_non_contact_constraint_pool[current_row + j];
                    if sc.upper_limit >= bit {
                        sc.upper_limit = bit;
                    }
                    if sc.lower_limit <= -bit {
                        sc.lower_limit = -bit;
                    }
                    sc.original_contact_point = RowOrigin::Constraint(cid);
                    sc.angular_component_a =
                        ra.inv_inertia_tensor_world * sc.relpos1_cross_normal * ra.angular_factor;
                    sc.angular_component_b =
                        rb.inv_inertia_tensor_world * sc.relpos2_cross_normal * rb.angular_factor;
                    {
                        let i_mjl_a = sc.contact_normal1 * ra.inverse_mass;
                        let i_mja_a = ra.inv_inertia_tensor_world * sc.relpos1_cross_normal;
                        let i_mjl_b = sc.contact_normal2 * rb.inverse_mass;
                        let i_mja_b = rb.inv_inertia_tensor_world * sc.relpos2_cross_normal;
                        let mut sum = i_mjl_a.dot(&sc.contact_normal1);
                        sum += i_mja_a.dot(&sc.relpos1_cross_normal);
                        sum += i_mjl_b.dot(&sc.contact_normal2);
                        sum += i_mja_b.dot(&sc.relpos2_cross_normal);
                        let fsum = sum.abs();
                        sc.jac_diag_ab_inv = if fsum > SIMD_EPSILON { 1.0 / sum } else { 0.0 };
                    }
                    {
                        let ext_fa = if ba.original_body.is_some() {
                            ba.external_force_impulse
                        } else {
                            Vec3::ZERO
                        };
                        let ext_ta = if ba.original_body.is_some() {
                            ba.external_torque_impulse
                        } else {
                            Vec3::ZERO
                        };
                        let ext_fb = if bb.original_body.is_some() {
                            bb.external_force_impulse
                        } else {
                            Vec3::ZERO
                        };
                        let ext_tb = if bb.original_body.is_some() {
                            bb.external_torque_impulse
                        } else {
                            Vec3::ZERO
                        };
                        let vel1_dotn = sc.contact_normal1.dot(&(ra.linear_velocity + ext_fa))
                            + sc.relpos1_cross_normal.dot(&(ra.angular_velocity + ext_ta));
                        let vel2_dotn = sc.contact_normal2.dot(&(rb.linear_velocity + ext_fb))
                            + sc.relpos2_cross_normal.dot(&(rb.angular_velocity + ext_tb));
                        let rel_vel = vel1_dotn + vel2_dotn;
                        let restitution = 0.0;
                        let positional_error = sc.rhs;
                        let velocity_error = fma(-rel_vel, damping, restitution);
                        let penetration_impulse = positional_error * sc.jac_diag_ab_inv;
                        let velocity_impulse = velocity_error * sc.jac_diag_ab_inv;
                        sc.rhs = penetration_impulse + velocity_impulse;
                        sc.applied_impulse = 0.0;
                    }
                }
            }
            current_row += nrows;
        }
        for &mid in manifold_ids {
            self.convert_contact(objects, manifolds, mid, info);
        }
        let num_non_contact = self.tmp_solver_non_contact_constraint_pool.len();
        let num_constraint = self.tmp_solver_contact_constraint_pool.len();
        let num_friction = self.tmp_solver_contact_friction_constraint_pool.len();
        self.order_non_contact_constraint_pool = (0..num_non_contact as i32).collect();
        let n = if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0 {
            num_constraint * 2
        } else {
            num_constraint
        };
        self.order_tmp_constraint_pool = vec![0; n];
        for (i, v) in self
            .order_tmp_constraint_pool
            .iter_mut()
            .enumerate()
            .take(num_constraint)
        {
            *v = i as i32;
        }
        self.order_friction_constraint_pool = (0..num_friction as i32).collect();
    }

    fn solve_row(&mut self, pool: Pool, idx: usize, kind: RowKind) {
        let c = match pool {
            Pool::NonContact => &mut self.tmp_solver_non_contact_constraint_pool[idx],
            Pool::Contact => &mut self.tmp_solver_contact_constraint_pool[idx],
            Pool::Friction => &mut self.tmp_solver_contact_friction_constraint_pool[idx],
            Pool::Rolling => &mut self.tmp_solver_contact_rolling_friction_constraint_pool[idx],
        };
        resolve_row(&mut self.tmp_solver_body_pool, c, kind);
    }

    /// `solveSingleIteration`.
    pub fn solve_single_iteration<B: BodyArena + ?Sized>(
        &mut self,
        iteration: i32,
        objects: &mut B,
        constraints: &ConstraintArena,
        constraint_ids: &[ConstraintId],
        info: &ContactSolverInfo,
    ) {
        let num_non_contact = self.tmp_solver_non_contact_constraint_pool.len();
        let num_constraint = self.tmp_solver_contact_constraint_pool.len();
        let num_friction = self.tmp_solver_contact_friction_constraint_pool.len();
        if info.solver_mode & SOLVER_RANDMIZE_ORDER != 0 {
            for j in 0..num_non_contact {
                let tmp = self.order_non_contact_constraint_pool[j];
                let swapi = self.bt_rand_int2(j as i32 + 1) as usize;
                self.order_non_contact_constraint_pool[j] =
                    self.order_non_contact_constraint_pool[swapi];
                self.order_non_contact_constraint_pool[swapi] = tmp;
            }
            if iteration < info.num_iterations {
                for j in 0..num_constraint {
                    let tmp = self.order_tmp_constraint_pool[j];
                    let swapi = self.bt_rand_int2(j as i32 + 1) as usize;
                    self.order_tmp_constraint_pool[j] = self.order_tmp_constraint_pool[swapi];
                    self.order_tmp_constraint_pool[swapi] = tmp;
                }
                for j in 0..num_friction {
                    let tmp = self.order_friction_constraint_pool[j];
                    let swapi = self.bt_rand_int2(j as i32 + 1) as usize;
                    self.order_friction_constraint_pool[j] =
                        self.order_friction_constraint_pool[swapi];
                    self.order_friction_constraint_pool[swapi] = tmp;
                }
            }
        }
        // SOLVER_SIMD and scalar paths are the same in double precision,
        // except for the interleaved contact/friction option.
        for j in 0..num_non_contact {
            let idx = self.order_non_contact_constraint_pool[j] as usize;
            if iteration
                < self.tmp_solver_non_contact_constraint_pool[idx].override_num_solver_iterations
            {
                self.solve_row(Pool::NonContact, idx, RowKind::Generic);
            }
        }
        if iteration < info.num_iterations {
            for &cid in constraint_ids {
                let c = constraints.get(cid);
                if c.is_enabled {
                    // solveConstraintObsolete is a no-op for these constraints,
                    // but the solver bodies are still looked up.
                    let (a, b) = (c.rb_a, c.rb_b);
                    self.get_or_init_solver_body(objects, a, info.time_step);
                    self.get_or_init_solver_body(objects, b, info.time_step);
                }
            }
            let simd = info.solver_mode & SOLVER_SIMD != 0;
            if simd && info.solver_mode & SOLVER_INTERLEAVE_CONTACT_AND_FRICTION_CONSTRAINTS != 0 {
                let multiplier = if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0 {
                    2
                } else {
                    1
                };
                for c in 0..num_constraint {
                    let idx = self.order_tmp_constraint_pool[c] as usize;
                    self.solve_row(Pool::Contact, idx, RowKind::LowerLimit);
                    let total_impulse =
                        self.tmp_solver_contact_constraint_pool[idx].applied_impulse;
                    let fidx = self.order_friction_constraint_pool[c * multiplier] as usize;
                    if total_impulse > 0.0 {
                        let f = &mut self.tmp_solver_contact_friction_constraint_pool[fidx];
                        f.lower_limit = -(f.friction * total_impulse);
                        f.upper_limit = f.friction * total_impulse;
                        self.solve_row(Pool::Friction, fidx, RowKind::Generic);
                    }
                    if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0 {
                        let fidx = self.order_friction_constraint_pool[c * multiplier + 1] as usize;
                        if total_impulse > 0.0 {
                            let f = &mut self.tmp_solver_contact_friction_constraint_pool[fidx];
                            f.lower_limit = -(f.friction * total_impulse);
                            f.upper_limit = f.friction * total_impulse;
                            self.solve_row(Pool::Friction, fidx, RowKind::Generic);
                        }
                    }
                }
            } else {
                for j in 0..num_constraint {
                    let idx = self.order_tmp_constraint_pool[j] as usize;
                    self.solve_row(Pool::Contact, idx, RowKind::LowerLimit);
                }
                for j in 0..num_friction {
                    let idx = self.order_friction_constraint_pool[j] as usize;
                    let fi = self.tmp_solver_contact_friction_constraint_pool[idx].friction_index
                        as usize;
                    let total_impulse = self.tmp_solver_contact_constraint_pool[fi].applied_impulse;
                    if total_impulse > 0.0 {
                        let f = &mut self.tmp_solver_contact_friction_constraint_pool[idx];
                        f.lower_limit = -(f.friction * total_impulse);
                        f.upper_limit = f.friction * total_impulse;
                        self.solve_row(Pool::Friction, idx, RowKind::Generic);
                    }
                }
                let num_rolling = self
                    .tmp_solver_contact_rolling_friction_constraint_pool
                    .len();
                for j in 0..num_rolling {
                    let fi = self.tmp_solver_contact_rolling_friction_constraint_pool[j]
                        .friction_index as usize;
                    let total_impulse = self.tmp_solver_contact_constraint_pool[fi].applied_impulse;
                    if total_impulse > 0.0 {
                        let r = &mut self.tmp_solver_contact_rolling_friction_constraint_pool[j];
                        let mut mag = r.friction * total_impulse;
                        if mag > r.friction {
                            mag = r.friction;
                        }
                        r.lower_limit = -mag;
                        r.upper_limit = mag;
                        self.solve_row(Pool::Rolling, j, RowKind::Generic);
                    }
                }
            }
        }
    }

    /// `solveGroupCacheFriendlySplitImpulseIterations`.
    pub fn solve_group_cache_friendly_split_impulse_iterations(
        &mut self,
        info: &ContactSolverInfo,
    ) {
        if info.split_impulse != 0 {
            for _ in 0..info.num_iterations {
                let n = self.tmp_solver_contact_constraint_pool.len();
                for j in 0..n {
                    let idx = self.order_tmp_constraint_pool[j] as usize;
                    self.solve_row(Pool::Contact, idx, RowKind::Split);
                }
            }
        }
    }

    /// `solveGroupCacheFriendlyIterations`.
    pub fn solve_group_cache_friendly_iterations<B: BodyArena + ?Sized>(
        &mut self,
        objects: &mut B,
        constraints: &ConstraintArena,
        constraint_ids: &[ConstraintId],
        info: &ContactSolverInfo,
    ) {
        self.solve_group_cache_friendly_split_impulse_iterations(info);
        let max_iterations = if self.max_override_num_solver_iterations > info.num_iterations {
            self.max_override_num_solver_iterations
        } else {
            info.num_iterations
        };
        for iteration in 0..max_iterations {
            self.solve_single_iteration(iteration, objects, constraints, constraint_ids, info);
        }
    }

    /// `solveGroupCacheFriendlyFinish`.
    pub fn solve_group_cache_friendly_finish<B: BodyArena + ?Sized, M: ManifoldArena + ?Sized>(
        &mut self,
        objects: &mut B,
        manifolds: &mut M,
        constraints: &mut ConstraintArena,
        info: &ContactSolverInfo,
    ) {
        if info.solver_mode & SOLVER_USE_WARMSTARTING != 0 {
            for sm in &self.tmp_solver_contact_constraint_pool {
                if let RowOrigin::Contact(mid, j) = sm.original_contact_point {
                    let lat1 = self.tmp_solver_contact_friction_constraint_pool
                        [sm.friction_index as usize]
                        .applied_impulse;
                    let lat2 = if info.solver_mode & SOLVER_USE_2_FRICTION_DIRECTIONS != 0 {
                        Some(
                            self.tmp_solver_contact_friction_constraint_pool
                                [sm.friction_index as usize + 1]
                                .applied_impulse,
                        )
                    } else {
                        None
                    };
                    let pt = manifolds.manifold_mut(mid).get_contact_point_mut(j);
                    pt.applied_impulse = sm.applied_impulse;
                    pt.applied_impulse_lateral1 = lat1;
                    if let Some(l2) = lat2 {
                        pt.applied_impulse_lateral2 = l2;
                    }
                }
            }
        }
        for sc in &self.tmp_solver_non_contact_constraint_pool {
            let RowOrigin::Constraint(cid) = sc.original_contact_point else {
                continue;
            };
            let (rb_a, rb_b) = {
                let c = constraints.get(cid);
                (c.rb_a, c.rb_b)
            };
            let (lfa, afa) = {
                let r = body_ref(&*objects, rb_a).rb();
                (r.linear_factor, r.angular_factor)
            };
            let (lfb, afb) = {
                let r = body_ref(&*objects, rb_b).rb();
                (r.linear_factor, r.angular_factor)
            };
            let c = constraints.get_mut(cid);
            if let Some(fb) = c.joint_feedback.as_mut() {
                fb.applied_force_body_a +=
                    sc.contact_normal1 * sc.applied_impulse * lfa / info.time_step;
                fb.applied_force_body_b +=
                    sc.contact_normal2 * sc.applied_impulse * lfb / info.time_step;
                fb.applied_torque_body_a +=
                    sc.relpos1_cross_normal * afa * sc.applied_impulse / info.time_step;
                fb.applied_torque_body_b +=
                    sc.relpos2_cross_normal * afb * sc.applied_impulse / info.time_step;
            }
            c.applied_impulse = sc.applied_impulse;
            if sc.applied_impulse.abs() >= c.breaking_impulse_threshold {
                c.is_enabled = false;
            }
        }
        for i in 0..self.tmp_solver_body_pool.len() {
            let sb = &mut self.tmp_solver_body_pool[i];
            if let Some(id) = sb.original_body {
                if info.split_impulse != 0 {
                    sb.writeback_velocity_and_transform(
                        info.time_step,
                        info.split_impulse_turn_erp,
                    );
                } else {
                    sb.writeback_velocity();
                }
                let lin = sb.linear_velocity + sb.external_force_impulse;
                let ang = sb.angular_velocity + sb.external_torque_impulse;
                let wt = sb.world_transform;
                let o = objects.obj_mut(id);
                o.set_linear_velocity(&lin);
                o.set_angular_velocity(&ang);
                if info.split_impulse != 0 {
                    o.set_world_transform(wt);
                }
                o.companion_id = -1;
            }
        }
        self.tmp_solver_contact_constraint_pool.clear();
        self.tmp_solver_non_contact_constraint_pool.clear();
        self.tmp_solver_contact_friction_constraint_pool.clear();
        self.tmp_solver_contact_rolling_friction_constraint_pool
            .clear();
        self.tmp_solver_body_pool.clear();
    }

    /// `solveGroup`.
    #[allow(clippy::too_many_arguments)]
    pub fn solve_group<B: BodyArena + ?Sized, M: ManifoldArena + ?Sized>(
        &mut self,
        objects: &mut B,
        bodies: &[ObjId],
        manifolds: &mut M,
        manifold_ids: &[ManifoldId],
        constraints: &mut ConstraintArena,
        constraint_ids: &[ConstraintId],
        info: &ContactSolverInfo,
    ) {
        self.solve_group_cache_friendly_setup(
            objects,
            bodies,
            manifolds,
            manifold_ids,
            constraints,
            constraint_ids,
            info,
        );
        self.solve_group_cache_friendly_iterations(objects, constraints, constraint_ids, info);
        self.solve_group_cache_friendly_finish(objects, manifolds, constraints, info);
    }
}

#[derive(Clone, Copy)]
enum Pool {
    NonContact,
    Contact,
    Friction,
    Rolling,
}
