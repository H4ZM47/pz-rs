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
// Altered for pz-rs: Rust translation of btGeneric6DofConstraint,
// btRotationalLimitMotor and btTranslationalLimitMotor (Bullet 2.82, double
// precision).

//! `btGeneric6DofConstraint` (sequential-impulse `getInfo1/2` path, frame
//! offset on, as constructed by the game).

use crate::bt::collision_object::{CollisionObject, ObjId};
use crate::bt::linear_math::*;

use super::hinge::{neg3, set3};
use super::typed_constraint::*;

pub const BT_6DOF_FLAGS_CFM_NORM: i32 = 1;
pub const BT_6DOF_FLAGS_CFM_STOP: i32 = 2;
pub const BT_6DOF_FLAGS_ERP_STOP: i32 = 4;
pub const BT_6DOF_FLAGS_AXIS_SHIFT: i32 = 3;

/// `btRotationalLimitMotor`.
#[derive(Clone, Copy, Debug)]
pub struct RotationalLimitMotor {
    pub lo_limit: f64,
    pub hi_limit: f64,
    pub target_velocity: f64,
    pub max_motor_force: f64,
    pub max_limit_force: f64,
    pub damping: f64,
    pub limit_softness: f64,
    pub normal_cfm: f64,
    pub stop_erp: f64,
    pub stop_cfm: f64,
    pub bounce: f64,
    pub enable_motor: bool,
    pub current_limit_error: f64,
    pub current_position: f64,
    pub current_limit: i32,
    pub accumulated_impulse: f64,
}

impl Default for RotationalLimitMotor {
    fn default() -> Self {
        RotationalLimitMotor {
            accumulated_impulse: 0.0,
            target_velocity: 0.0,
            max_motor_force: flt(0.1),
            max_limit_force: flt(300.0),
            lo_limit: flt(1.0),
            hi_limit: flt(-1.0),
            normal_cfm: 0.0,
            stop_erp: flt(0.2),
            stop_cfm: 0.0,
            bounce: 0.0,
            damping: 1.0,
            limit_softness: 0.5,
            current_limit: 0,
            current_limit_error: 0.0,
            current_position: 0.0,
            enable_motor: false,
        }
    }
}

impl RotationalLimitMotor {
    pub fn is_limited(&self) -> bool {
        self.lo_limit <= self.hi_limit
    }
    pub fn need_apply_torques(&self) -> bool {
        !(self.current_limit == 0 && !self.enable_motor)
    }
    /// `testLimitValue`.
    pub fn test_limit_value(&mut self, test_value: f64) -> i32 {
        if self.lo_limit > self.hi_limit {
            self.current_limit = 0;
            return 0;
        }
        if test_value < self.lo_limit {
            self.current_limit = 1;
            self.current_limit_error = test_value - self.lo_limit;
            if self.current_limit_error > SIMD_PI {
                self.current_limit_error -= SIMD_2_PI;
            } else if self.current_limit_error < -SIMD_PI {
                self.current_limit_error += SIMD_2_PI;
            }
            return 1;
        } else if test_value > self.hi_limit {
            self.current_limit = 2;
            self.current_limit_error = test_value - self.hi_limit;
            if self.current_limit_error > SIMD_PI {
                self.current_limit_error -= SIMD_2_PI;
            } else if self.current_limit_error < -SIMD_PI {
                self.current_limit_error += SIMD_2_PI;
            }
            return 2;
        }
        self.current_limit = 0;
        0
    }
}

/// `btTranslationalLimitMotor`.
#[derive(Clone, Copy, Debug)]
pub struct TranslationalLimitMotor {
    pub lower_limit: Vec3,
    pub upper_limit: Vec3,
    pub accumulated_impulse: Vec3,
    pub limit_softness: f64,
    pub damping: f64,
    pub restitution: f64,
    pub normal_cfm: Vec3,
    pub stop_erp: Vec3,
    pub stop_cfm: Vec3,
    pub enable_motor: [bool; 3],
    pub target_velocity: Vec3,
    pub max_motor_force: Vec3,
    pub current_limit_error: Vec3,
    pub current_linear_diff: Vec3,
    pub current_limit: [i32; 3],
}

impl Default for TranslationalLimitMotor {
    fn default() -> Self {
        TranslationalLimitMotor {
            lower_limit: Vec3::ZERO,
            upper_limit: Vec3::ZERO,
            accumulated_impulse: Vec3::ZERO,
            normal_cfm: Vec3::ZERO,
            stop_erp: Vec3::splat(flt(0.2)),
            stop_cfm: Vec3::ZERO,
            limit_softness: flt(0.7),
            damping: 1.0,
            restitution: 0.5,
            enable_motor: [false; 3],
            target_velocity: Vec3::ZERO,
            max_motor_force: Vec3::ZERO,
            current_limit_error: Vec3::ZERO,
            current_linear_diff: Vec3::ZERO,
            current_limit: [0; 3],
        }
    }
}

impl TranslationalLimitMotor {
    pub fn is_limited(&self, i: usize) -> bool {
        self.upper_limit[i] >= self.lower_limit[i]
    }
    pub fn need_apply_force(&self, i: usize) -> bool {
        !(self.current_limit[i] == 0 && !self.enable_motor[i])
    }
    /// `testLimitValue`.
    pub fn test_limit_value(&mut self, i: usize, test_value: f64) -> i32 {
        let lo = self.lower_limit[i];
        let hi = self.upper_limit[i];
        if lo > hi {
            self.current_limit[i] = 0;
            self.current_limit_error[i] = 0.0;
            return 0;
        }
        if test_value < lo {
            self.current_limit[i] = 2;
            self.current_limit_error[i] = test_value - lo;
            return 2;
        } else if test_value > hi {
            self.current_limit[i] = 1;
            self.current_limit_error[i] = test_value - hi;
            return 1;
        }
        self.current_limit[i] = 0;
        self.current_limit_error[i] = 0.0;
        0
    }
}

/// `btGeneric6DofConstraint`.
#[derive(Clone, Debug)]
pub struct Generic6DofConstraint {
    pub frame_in_a: Transform,
    pub frame_in_b: Transform,
    pub linear_limits: TranslationalLimitMotor,
    pub angular_limits: [RotationalLimitMotor; 3],
    pub time_step: f64,
    pub calculated_transform_a: Transform,
    pub calculated_transform_b: Transform,
    pub calculated_axis_angle_diff: Vec3,
    pub calculated_axis: [Vec3; 3],
    pub calculated_linear_diff: Vec3,
    pub fact_a: f64,
    pub fact_b: f64,
    pub has_static_body: bool,
    pub anchor_pos: Vec3,
    pub use_linear_reference_frame_a: bool,
    pub use_offset_for_constraint_frame: bool,
    pub flags: i32,
    pub use_solve_constraint_obsolete: bool,
}

/// `btGetMatrixElem`.
#[inline(always)]
fn mat_elem(m: &Mat3, index: usize) -> f64 {
    m[index % 3][index / 3]
}

/// `matrixToEulerXYZ`.
pub fn matrix_to_euler_xyz(mat: &Mat3, xyz: &mut Vec3) -> bool {
    let fi = mat_elem(mat, 2);
    if fi < 1.0 {
        if fi > -1.0 {
            xyz[0] = bt_atan2(-mat_elem(mat, 5), mat_elem(mat, 8));
            xyz[1] = bt_asin(mat_elem(mat, 2));
            xyz[2] = bt_atan2(-mat_elem(mat, 1), mat_elem(mat, 0));
            true
        } else {
            xyz[0] = -bt_atan2(mat_elem(mat, 3), mat_elem(mat, 4));
            xyz[1] = -SIMD_HALF_PI;
            xyz[2] = 0.0;
            false
        }
    } else {
        xyz[0] = bt_atan2(mat_elem(mat, 3), mat_elem(mat, 4));
        xyz[1] = SIMD_HALF_PI;
        xyz[2] = 0.0;
        false
    }
}

impl Generic6DofConstraint {
    /// `btGeneric6DofConstraint(rbA, rbB, frameInA, frameInB,
    /// useLinearReferenceFrameA)`; the constructor's `calculateTransforms()`
    /// needs both bodies.
    pub fn new_typed(
        rb_a: ObjId,
        rb_b: ObjId,
        body_a: &CollisionObject,
        body_b: &CollisionObject,
        frame_in_a: Transform,
        frame_in_b: Transform,
        use_linear_reference_frame_a: bool,
    ) -> TypedConstraint {
        let mut c = Generic6DofConstraint {
            frame_in_a,
            frame_in_b,
            linear_limits: TranslationalLimitMotor::default(),
            angular_limits: [RotationalLimitMotor::default(); 3],
            time_step: 0.0,
            calculated_transform_a: Transform::default(),
            calculated_transform_b: Transform::default(),
            calculated_axis_angle_diff: Vec3::ZERO,
            calculated_axis: [Vec3::ZERO; 3],
            calculated_linear_diff: Vec3::ZERO,
            fact_a: 0.0,
            fact_b: 0.0,
            has_static_body: false,
            anchor_pos: Vec3::ZERO,
            use_linear_reference_frame_a,
            use_offset_for_constraint_frame: true,
            flags: 0,
            use_solve_constraint_obsolete: false,
        };
        c.calculate_transforms(
            &body_a.world_transform,
            &body_b.world_transform,
            body_a.rb().inverse_mass,
            body_b.rb().inverse_mass,
        );
        TypedConstraint::new(
            D6_CONSTRAINT_TYPE,
            Some(rb_a),
            Some(rb_b),
            ConstraintKind::Generic6Dof(Box::new(c)),
        )
    }

    /// `calculateAngleInfo`.
    pub fn calculate_angle_info(&mut self) {
        let relative_frame =
            self.calculated_transform_a.basis.inverse() * self.calculated_transform_b.basis;
        matrix_to_euler_xyz(&relative_frame, &mut self.calculated_axis_angle_diff);
        let axis0 = self.calculated_transform_b.basis.get_column(0);
        let axis2 = self.calculated_transform_a.basis.get_column(2);
        self.calculated_axis[1] = axis2.cross(&axis0);
        self.calculated_axis[0] = self.calculated_axis[1].cross(&axis2);
        self.calculated_axis[2] = axis0.cross(&self.calculated_axis[1]);
        self.calculated_axis[0].normalize();
        self.calculated_axis[1].normalize();
        self.calculated_axis[2].normalize();
    }

    /// `calculateLinearInfo`.
    pub fn calculate_linear_info(&mut self) {
        self.calculated_linear_diff =
            self.calculated_transform_b.origin - self.calculated_transform_a.origin;
        self.calculated_linear_diff =
            self.calculated_transform_a.basis.inverse() * self.calculated_linear_diff;
        for i in 0..3 {
            self.linear_limits.current_linear_diff[i] = self.calculated_linear_diff[i];
            self.linear_limits
                .test_limit_value(i, self.calculated_linear_diff[i]);
        }
    }

    /// `calculateTransforms(transA, transB)` (inverse masses of both bodies
    /// for the frame-offset factors).
    pub fn calculate_transforms(
        &mut self,
        trans_a: &Transform,
        trans_b: &Transform,
        mi_a: f64,
        mi_b: f64,
    ) {
        self.calculated_transform_a = *trans_a * self.frame_in_a;
        self.calculated_transform_b = *trans_b * self.frame_in_b;
        self.calculate_linear_info();
        self.calculate_angle_info();
        if self.use_offset_for_constraint_frame {
            self.has_static_body = (mi_a < SIMD_EPSILON) || (mi_b < SIMD_EPSILON);
            let mi_s = mi_a + mi_b;
            self.fact_a = if mi_s > 0.0 { mi_b / mi_s } else { flt(0.5) };
            self.fact_b = 1.0 - self.fact_a;
        }
    }

    /// `testAngularLimitMotor`.
    pub fn test_angular_limit_motor(&mut self, axis_index: usize) -> bool {
        let mut angle = self.calculated_axis_angle_diff[axis_index];
        let m = &mut self.angular_limits[axis_index];
        angle = adjust_angle_to_limits(angle, m.lo_limit, m.hi_limit);
        m.current_position = angle;
        m.test_limit_value(angle);
        m.need_apply_torques()
    }

    /// `getInfo1`.
    pub fn get_info1(
        &mut self,
        info: &mut ConstraintInfo1,
        trans_a: &Transform,
        trans_b: &Transform,
        mi_a: f64,
        mi_b: f64,
    ) {
        if self.use_solve_constraint_obsolete {
            info.num_constraint_rows = 0;
            info.nub = 0;
        } else {
            self.calculate_transforms(trans_a, trans_b, mi_a, mi_b);
            info.num_constraint_rows = 0;
            info.nub = 6;
            for i in 0..3 {
                if self.linear_limits.need_apply_force(i) {
                    info.num_constraint_rows += 1;
                    info.nub -= 1;
                }
            }
            for i in 0..3 {
                if self.test_angular_limit_motor(i) {
                    info.num_constraint_rows += 1;
                    info.nub -= 1;
                }
            }
        }
    }

    /// `getInfo2`.
    pub fn get_info2(
        &mut self,
        info: &mut ConstraintInfo2,
        body_a: &CollisionObject,
        body_b: &CollisionObject,
    ) {
        let ctx = BodyState {
            trans_a: body_a.world_transform,
            trans_b: body_b.world_transform,
            lin_vel_a: body_a.rb().linear_velocity,
            lin_vel_b: body_b.rb().linear_velocity,
            ang_vel_a: body_a.rb().angular_velocity,
            ang_vel_b: body_b.rb().angular_velocity,
        };
        if self.use_offset_for_constraint_frame {
            let row = self.set_angular_limits(info, 0, &ctx);
            self.set_linear_limits(info, row, &ctx);
        } else {
            let row = self.set_linear_limits(info, 0, &ctx);
            self.set_angular_limits(info, row, &ctx);
        }
    }

    fn set_linear_limits(
        &mut self,
        info: &mut ConstraintInfo2,
        mut row: usize,
        ctx: &BodyState,
    ) -> usize {
        let mut limot = RotationalLimitMotor::default();
        for i in 0..3 {
            if self.linear_limits.need_apply_force(i) {
                limot.bounce = 0.0;
                limot.current_limit = self.linear_limits.current_limit[i];
                limot.current_position = self.linear_limits.current_linear_diff[i];
                limot.current_limit_error = self.linear_limits.current_limit_error[i];
                limot.damping = self.linear_limits.damping;
                limot.enable_motor = self.linear_limits.enable_motor[i];
                limot.hi_limit = self.linear_limits.upper_limit[i];
                limot.limit_softness = self.linear_limits.limit_softness;
                limot.lo_limit = self.linear_limits.lower_limit[i];
                limot.max_limit_force = 0.0;
                limot.max_motor_force = self.linear_limits.max_motor_force[i];
                limot.target_velocity = self.linear_limits.target_velocity[i];
                let axis = self.calculated_transform_a.basis.get_column(i);
                let flags = self.flags >> (i as i32 * BT_6DOF_FLAGS_AXIS_SHIFT);
                limot.normal_cfm = if flags & BT_6DOF_FLAGS_CFM_NORM != 0 {
                    self.linear_limits.normal_cfm[i]
                } else {
                    info.rows[0].cfm
                };
                limot.stop_cfm = if flags & BT_6DOF_FLAGS_CFM_STOP != 0 {
                    self.linear_limits.stop_cfm[i]
                } else {
                    info.rows[0].cfm
                };
                limot.stop_erp = if flags & BT_6DOF_FLAGS_ERP_STOP != 0 {
                    self.linear_limits.stop_erp[i]
                } else {
                    info.erp
                };
                if self.use_offset_for_constraint_frame {
                    let indx1 = (i + 1) % 3;
                    let indx2 = (i + 2) % 3;
                    let mut rot_allowed = true;
                    if self.angular_limits[indx1].current_limit != 0
                        && self.angular_limits[indx2].current_limit != 0
                    {
                        rot_allowed = false;
                    }
                    row += self.get_limit_motor_info2(
                        &limot,
                        ctx,
                        info,
                        row,
                        &axis,
                        false,
                        rot_allowed,
                    );
                } else {
                    row += self.get_limit_motor_info2(&limot, ctx, info, row, &axis, false, false);
                }
            }
        }
        row
    }

    fn set_angular_limits(
        &mut self,
        info: &mut ConstraintInfo2,
        row_offset: usize,
        ctx: &BodyState,
    ) -> usize {
        let mut row = row_offset;
        for i in 0..3 {
            if self.angular_limits[i].need_apply_torques() {
                let axis = self.calculated_axis[i];
                let flags = self.flags >> ((i as i32 + 3) * BT_6DOF_FLAGS_AXIS_SHIFT);
                if flags & BT_6DOF_FLAGS_CFM_NORM == 0 {
                    self.angular_limits[i].normal_cfm = info.rows[0].cfm;
                }
                if flags & BT_6DOF_FLAGS_CFM_STOP == 0 {
                    self.angular_limits[i].stop_cfm = info.rows[0].cfm;
                }
                if flags & BT_6DOF_FLAGS_ERP_STOP == 0 {
                    self.angular_limits[i].stop_erp = info.erp;
                }
                let limot = self.angular_limits[i];
                row += self.get_limit_motor_info2(&limot, ctx, info, row, &axis, true, false);
            }
        }
        row
    }

    /// `get_limit_motor_info2`.
    #[allow(clippy::too_many_arguments)]
    fn get_limit_motor_info2(
        &self,
        limot: &RotationalLimitMotor,
        ctx: &BodyState,
        info: &mut ConstraintInfo2,
        row: usize,
        ax1: &Vec3,
        rotational: bool,
        rot_allowed: bool,
    ) -> usize {
        let mut powered = limot.enable_motor;
        let limit = limot.current_limit;
        if !(powered || limit != 0) {
            return 0;
        }
        let fps = info.fps;
        let r = &mut info.rows[row];
        if rotational {
            set3(&mut r.relpos1_cross_normal, ax1);
            set3(&mut r.relpos2_cross_normal, &neg3(ax1));
        } else {
            set3(&mut r.contact_normal1, ax1);
            set3(&mut r.contact_normal2, &neg3(ax1));
        }
        if !rotational {
            if self.use_offset_for_constraint_frame {
                let mut rel_b = self.calculated_transform_b.origin - ctx.trans_b.origin;
                let proj_b = *ax1 * rel_b.dot(ax1);
                let ortho_b = rel_b - proj_b;
                let mut rel_a = self.calculated_transform_a.origin - ctx.trans_a.origin;
                let proj_a = *ax1 * rel_a.dot(ax1);
                let ortho_a = rel_a - proj_a;
                let desired_offs = limot.current_position - limot.current_limit_error;
                let total_dist = proj_a + *ax1 * desired_offs - proj_b;
                rel_a = ortho_a + total_dist * self.fact_a;
                rel_b = ortho_b - total_dist * self.fact_b;
                let mut tmp_a = rel_a.cross(ax1);
                let mut tmp_b = rel_b.cross(ax1);
                if self.has_static_body && !rot_allowed {
                    tmp_a *= self.fact_a;
                    tmp_b *= self.fact_b;
                }
                set3(&mut r.relpos1_cross_normal, &tmp_a);
                set3(&mut r.relpos2_cross_normal, &neg3(&tmp_b));
            } else {
                let c = self.calculated_transform_b.origin - ctx.trans_a.origin;
                let ltd = c.cross(ax1);
                set3(&mut r.relpos1_cross_normal, &ltd);
                let c = self.calculated_transform_b.origin - ctx.trans_b.origin;
                let ltd = -c.cross(ax1);
                set3(&mut r.relpos2_cross_normal, &ltd);
            }
        }
        if limit != 0 && limot.lo_limit == limot.hi_limit {
            powered = false;
        }
        r.rhs = 0.0;
        if powered {
            r.cfm = limot.normal_cfm;
            if limit == 0 {
                let tag_vel = if rotational {
                    limot.target_velocity
                } else {
                    -limot.target_velocity
                };
                let mot_fact = get_motor_factor(
                    limot.current_position,
                    limot.lo_limit,
                    limot.hi_limit,
                    tag_vel,
                    fps * limot.stop_erp,
                );
                r.rhs = fma(mot_fact, limot.target_velocity, r.rhs);
                r.lower_limit = -limot.max_motor_force;
                r.upper_limit = limot.max_motor_force;
            }
        }
        if limit != 0 {
            let k = fps * limot.stop_erp;
            if !rotational {
                r.rhs = fma(k, limot.current_limit_error, r.rhs);
            } else {
                r.rhs = fma(-k, limot.current_limit_error, r.rhs);
            }
            r.cfm = limot.stop_cfm;
            if limot.lo_limit == limot.hi_limit {
                r.lower_limit = -SIMD_INFINITY;
                r.upper_limit = SIMD_INFINITY;
            } else {
                if limit == 1 {
                    r.lower_limit = 0.0;
                    r.upper_limit = SIMD_INFINITY;
                } else {
                    r.lower_limit = -SIMD_INFINITY;
                    r.upper_limit = 0.0;
                }
                if limot.bounce > 0.0 {
                    let mut vel;
                    if rotational {
                        vel = ctx.ang_vel_a.dot(ax1);
                        vel -= ctx.ang_vel_b.dot(ax1);
                    } else {
                        vel = ctx.lin_vel_a.dot(ax1);
                        vel -= ctx.lin_vel_b.dot(ax1);
                    }
                    if limit == 1 {
                        if vel < 0.0 {
                            let newc = -limot.bounce * vel;
                            if newc > r.rhs {
                                r.rhs = newc;
                            }
                        }
                    } else if vel > 0.0 {
                        let newc = -limot.bounce * vel;
                        if newc < r.rhs {
                            r.rhs = newc;
                        }
                    }
                }
            }
        }
        1
    }

    /// `setLinearLowerLimit`.
    pub fn set_linear_lower_limit(&mut self, v: Vec3) {
        self.linear_limits.lower_limit = v;
    }
    /// `setLinearUpperLimit`.
    pub fn set_linear_upper_limit(&mut self, v: Vec3) {
        self.linear_limits.upper_limit = v;
    }
    /// `setAngularLowerLimit` (normalized).
    pub fn set_angular_lower_limit(&mut self, v: &Vec3) {
        for i in 0..3 {
            self.angular_limits[i].lo_limit = bt_normalize_angle(v[i]);
        }
    }
    /// `setAngularUpperLimit` (normalized).
    pub fn set_angular_upper_limit(&mut self, v: &Vec3) {
        for i in 0..3 {
            self.angular_limits[i].hi_limit = bt_normalize_angle(v[i]);
        }
    }

    /// `setParam`.
    pub fn set_param(&mut self, num: i32, value: f64, axis: i32) {
        if (0..3).contains(&axis) {
            let a = axis as usize;
            match num {
                BT_CONSTRAINT_STOP_ERP => {
                    self.linear_limits.stop_erp[a] = value;
                    self.flags |= BT_6DOF_FLAGS_ERP_STOP << (axis * BT_6DOF_FLAGS_AXIS_SHIFT);
                }
                BT_CONSTRAINT_STOP_CFM => {
                    self.linear_limits.stop_cfm[a] = value;
                    self.flags |= BT_6DOF_FLAGS_CFM_STOP << (axis * BT_6DOF_FLAGS_AXIS_SHIFT);
                }
                BT_CONSTRAINT_CFM => {
                    self.linear_limits.normal_cfm[a] = value;
                    self.flags |= BT_6DOF_FLAGS_CFM_NORM << (axis * BT_6DOF_FLAGS_AXIS_SHIFT);
                }
                _ => {}
            }
        } else if (3..6).contains(&axis) {
            let a = (axis - 3) as usize;
            match num {
                BT_CONSTRAINT_STOP_ERP => {
                    self.angular_limits[a].stop_erp = value;
                    self.flags |= BT_6DOF_FLAGS_ERP_STOP << (axis * BT_6DOF_FLAGS_AXIS_SHIFT);
                }
                BT_CONSTRAINT_STOP_CFM => {
                    self.angular_limits[a].stop_cfm = value;
                    self.flags |= BT_6DOF_FLAGS_CFM_STOP << (axis * BT_6DOF_FLAGS_AXIS_SHIFT);
                }
                BT_CONSTRAINT_CFM => {
                    self.angular_limits[a].normal_cfm = value;
                    self.flags |= BT_6DOF_FLAGS_CFM_NORM << (axis * BT_6DOF_FLAGS_AXIS_SHIFT);
                }
                _ => {}
            }
        }
    }

    /// `getParam`.
    pub fn get_param(&self, num: i32, axis: i32) -> f64 {
        if (0..3).contains(&axis) {
            let a = axis as usize;
            match num {
                BT_CONSTRAINT_STOP_ERP => return self.linear_limits.stop_erp[a],
                BT_CONSTRAINT_STOP_CFM => return self.linear_limits.stop_cfm[a],
                BT_CONSTRAINT_CFM => return self.linear_limits.normal_cfm[a],
                _ => {}
            }
        } else if (3..6).contains(&axis) {
            let a = (axis - 3) as usize;
            match num {
                BT_CONSTRAINT_STOP_ERP => return self.angular_limits[a].stop_erp,
                BT_CONSTRAINT_STOP_CFM => return self.angular_limits[a].stop_cfm,
                BT_CONSTRAINT_CFM => return self.angular_limits[a].normal_cfm,
                _ => {}
            }
        }
        0.0
    }
}

struct BodyState {
    trans_a: Transform,
    trans_b: Transform,
    lin_vel_a: Vec3,
    lin_vel_b: Vec3,
    ang_vel_a: Vec3,
    ang_vel_b: Vec3,
}
