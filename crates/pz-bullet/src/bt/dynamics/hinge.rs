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
// Altered for pz-rs: Rust translation of btHingeConstraint (Bullet 2.82,
// double precision, `_BT_USE_CENTER_LIMIT_`).

//! `btHingeConstraint`. Only the frame-offset solver path exists
//! (`m_useOffsetForConstraintFrame` defaults to true and the game never
//! changes it).

use crate::bt::collision_object::{CollisionObject, ObjId};
use crate::bt::linear_math::*;

use super::typed_constraint::*;

pub const BT_HINGE_FLAGS_CFM_STOP: i32 = 1;
pub const BT_HINGE_FLAGS_ERP_STOP: i32 = 2;
pub const BT_HINGE_FLAGS_CFM_NORM: i32 = 4;

/// `btHingeConstraint`.
#[derive(Clone, Debug)]
pub struct HingeConstraint {
    pub rb_a_frame: Transform,
    pub rb_b_frame: Transform,
    pub motor_target_velocity: f64,
    pub max_motor_impulse: f64,
    pub limit: AngularLimit,
    pub k_hinge: f64,
    pub acc_limit_impulse: f64,
    pub hinge_angle: f64,
    pub reference_sign: f64,
    pub angular_only: bool,
    pub enable_angular_motor: bool,
    pub use_solve_constraint_obsolete: bool,
    pub use_offset_for_constraint_frame: bool,
    pub use_reference_frame_a: bool,
    pub acc_motor_impulse: f64,
    pub flags: i32,
    pub normal_cfm: f64,
    pub stop_cfm: f64,
    pub stop_erp: f64,
}

impl HingeConstraint {
    /// The member initialisation shared by the constructors.
    fn blank(use_reference_frame_a: bool) -> HingeConstraint {
        HingeConstraint {
            rb_a_frame: Transform::default(),
            rb_b_frame: Transform::default(),
            motor_target_velocity: 0.0,
            max_motor_impulse: 0.0,
            limit: AngularLimit::default(),
            k_hinge: 0.0,
            acc_limit_impulse: 0.0,
            hinge_angle: 0.0,
            reference_sign: 0.0,
            angular_only: false,
            enable_angular_motor: false,
            use_solve_constraint_obsolete: false,
            use_offset_for_constraint_frame: true,
            use_reference_frame_a,
            acc_motor_impulse: 0.0,
            flags: 0,
            normal_cfm: 0.0,
            stop_cfm: 0.0,
            stop_erp: 0.0,
        }
    }

    /// `btHingeConstraint(rbA, rbB, rbAFrame, rbBFrame, useReferenceFrameA)`.
    pub fn new_typed_frames(
        rb_a: ObjId,
        rb_b: ObjId,
        rb_a_frame: Transform,
        rb_b_frame: Transform,
        use_reference_frame_a: bool,
    ) -> TypedConstraint {
        let mut h = HingeConstraint::blank(use_reference_frame_a);
        h.rb_a_frame = rb_a_frame;
        h.rb_b_frame = rb_b_frame;
        h.reference_sign = if use_reference_frame_a {
            flt(-1.0)
        } else {
            flt(1.0)
        };
        TypedConstraint::new(
            HINGE_CONSTRAINT_TYPE,
            Some(rb_a),
            Some(rb_b),
            ConstraintKind::Hinge(Box::new(h)),
        )
    }

    /// `btHingeConstraint(rbA, rbB, pivotInA, pivotInB, axisInA, axisInB,
    /// useReferenceFrameA)`. `rb_a_transform` is rbA's world transform.
    #[allow(clippy::too_many_arguments)]
    pub fn new_typed(
        rb_a: ObjId,
        rb_b: ObjId,
        rb_a_transform: &Transform,
        pivot_in_a: Vec3,
        pivot_in_b: Vec3,
        axis_in_a: Vec3,
        axis_in_b: Vec3,
        use_reference_frame_a: bool,
    ) -> TypedConstraint {
        let mut h = HingeConstraint::blank(use_reference_frame_a);
        h.rb_a_frame.origin = pivot_in_a;
        let b = &rb_a_transform.basis;
        let mut rb_axis_a1 = b.get_column(0);
        let rb_axis_a2;
        let projection = axis_in_a.dot(&rb_axis_a1);
        if projection >= 1.0 - SIMD_EPSILON {
            rb_axis_a1 = -b.get_column(2);
            rb_axis_a2 = b.get_column(1);
        } else if projection <= -1.0 + SIMD_EPSILON {
            rb_axis_a1 = b.get_column(2);
            rb_axis_a2 = b.get_column(1);
        } else {
            rb_axis_a2 = axis_in_a.cross(&rb_axis_a1);
            rb_axis_a1 = rb_axis_a2.cross(&axis_in_a);
        }
        h.rb_a_frame.basis.set_value(
            rb_axis_a1.x,
            rb_axis_a2.x,
            axis_in_a.x,
            rb_axis_a1.y,
            rb_axis_a2.y,
            axis_in_a.y,
            rb_axis_a1.z,
            rb_axis_a2.z,
            axis_in_a.z,
        );
        let rotation_arc = shortest_arc_quat(&axis_in_a, &axis_in_b);
        let rb_axis_b1 = quat_rotate(&rotation_arc, &rb_axis_a1);
        let rb_axis_b2 = axis_in_b.cross(&rb_axis_b1);
        h.rb_b_frame.origin = pivot_in_b;
        h.rb_b_frame.basis.set_value(
            rb_axis_b1.x,
            rb_axis_b2.x,
            axis_in_b.x,
            rb_axis_b1.y,
            rb_axis_b2.y,
            axis_in_b.y,
            rb_axis_b1.z,
            rb_axis_b2.z,
            axis_in_b.z,
        );
        h.reference_sign = if use_reference_frame_a {
            flt(-1.0)
        } else {
            flt(1.0)
        };
        TypedConstraint::new(
            HINGE_CONSTRAINT_TYPE,
            Some(rb_a),
            Some(rb_b),
            ConstraintKind::Hinge(Box::new(h)),
        )
    }

    /// `setLimit`.
    pub fn set_limit(
        &mut self,
        low: f64,
        high: f64,
        softness: f64,
        bias_factor: f64,
        relaxation_factor: f64,
    ) {
        self.limit
            .set(low, high, softness, bias_factor, relaxation_factor);
    }

    /// `enableAngularMotor`.
    pub fn enable_angular_motor(
        &mut self,
        enable: bool,
        target_velocity: f64,
        max_motor_impulse: f64,
    ) {
        self.enable_angular_motor = enable;
        self.motor_target_velocity = target_velocity;
        self.max_motor_impulse = max_motor_impulse;
    }

    /// `getHingeAngle(transA, transB)`.
    pub fn get_hinge_angle(&self, trans_a: &Transform, trans_b: &Transform) -> f64 {
        let ref_axis0 = trans_a.basis * self.rb_a_frame.basis.get_column(0);
        let ref_axis1 = trans_a.basis * self.rb_a_frame.basis.get_column(1);
        let swing_axis = trans_b.basis * self.rb_b_frame.basis.get_column(1);
        let angle = bt_atan2(swing_axis.dot(&ref_axis0), swing_axis.dot(&ref_axis1));
        self.reference_sign * angle
    }

    /// `testLimit`.
    pub fn test_limit(&mut self, trans_a: &Transform, trans_b: &Transform) {
        self.hinge_angle = self.get_hinge_angle(trans_a, trans_b);
        self.limit.test(self.hinge_angle);
    }

    /// `getInfo1`.
    pub fn get_info1(
        &mut self,
        info: &mut ConstraintInfo1,
        trans_a: &Transform,
        trans_b: &Transform,
    ) {
        if self.use_solve_constraint_obsolete {
            info.num_constraint_rows = 0;
            info.nub = 0;
        } else {
            info.num_constraint_rows = 5;
            info.nub = 1;
            self.test_limit(trans_a, trans_b);
            if self.limit.is_limit() || self.enable_angular_motor {
                info.num_constraint_rows += 1;
                info.nub -= 1;
            }
        }
    }

    /// `getInfo2` (`getInfo2InternalUsingFrameOffset`).
    pub fn get_info2(
        &mut self,
        info: &mut ConstraintInfo2,
        body_a: &CollisionObject,
        body_b: &CollisionObject,
    ) {
        assert!(
            self.use_offset_for_constraint_frame,
            "btHingeConstraint::getInfo2Internal (no frame offset) is not ported"
        );
        let trans_a = body_a.world_transform;
        let trans_b = body_b.world_transform;
        let ang_vel_a = body_a.rb().angular_velocity;
        let ang_vel_b = body_b.rb().angular_velocity;
        let tr_a = trans_a * self.rb_a_frame;
        let tr_b = trans_b * self.rb_b_frame;
        let ofs = tr_b.origin - tr_a.origin;
        let mi_a = body_a.rb().inverse_mass;
        let mi_b = body_b.rb().inverse_mass;
        let has_static_body = (mi_a < SIMD_EPSILON) || (mi_b < SIMD_EPSILON);
        let mi_s = mi_a + mi_b;
        let fact_a = if mi_s > 0.0 { mi_b / mi_s } else { flt(0.5) };
        let fact_b = 1.0 - fact_a;
        let ax1_a = tr_a.basis.get_column(2);
        let ax1_b = tr_b.basis.get_column(2);
        let mut ax1 = ax1_a * fact_a + ax1_b * fact_b;
        ax1.normalize();
        let body_a_trans = trans_a;
        let body_b_trans = trans_b;

        let mut rel_b = tr_b.origin - body_b_trans.origin;
        let proj_b = ax1 * rel_b.dot(&ax1);
        let ortho_b = rel_b - proj_b;
        let mut rel_a = tr_a.origin - body_a_trans.origin;
        let proj_a = ax1 * rel_a.dot(&ax1);
        let ortho_a = rel_a - proj_a;
        let total_dist = proj_a - proj_b;
        rel_a = ortho_a + total_dist * fact_a;
        rel_b = ortho_b - total_dist * fact_b;
        let mut p = ortho_b * fact_a + ortho_a * fact_b;
        let len2 = p.length2();
        if len2 > SIMD_EPSILON {
            p.div_assign_scalar(len2.sqrt());
        } else {
            p = tr_a.basis.get_column(1);
        }
        let q = ax1.cross(&p);
        let solve_limit = self.limit.is_limit();
        let r = &mut *info.rows;

        let tmp_a = rel_a.cross(&p);
        let tmp_b = rel_b.cross(&p);
        set3(&mut r[0].relpos1_cross_normal, &tmp_a);
        set3(&mut r[0].relpos2_cross_normal, &neg3(&tmp_b));
        let mut tmp_a = rel_a.cross(&q);
        let mut tmp_b = rel_b.cross(&q);
        if has_static_body && solve_limit {
            tmp_b *= fact_b;
            tmp_a *= fact_a;
        }
        set3(&mut r[1].relpos1_cross_normal, &tmp_a);
        set3(&mut r[1].relpos2_cross_normal, &neg3(&tmp_b));
        let mut tmp_a = rel_a.cross(&ax1);
        let mut tmp_b = rel_b.cross(&ax1);
        if has_static_body {
            tmp_b *= fact_b;
            tmp_a *= fact_a;
        }
        set3(&mut r[2].relpos1_cross_normal, &tmp_a);
        set3(&mut r[2].relpos2_cross_normal, &neg3(&tmp_b));

        let mut k = info.fps * info.erp;
        if !self.angular_only {
            set3(&mut r[0].contact_normal1, &p);
            set3(&mut r[1].contact_normal1, &q);
            set3(&mut r[2].contact_normal1, &ax1);
            set3(&mut r[0].contact_normal2, &neg3(&p));
            set3(&mut r[1].contact_normal2, &neg3(&q));
            set3(&mut r[2].contact_normal2, &neg3(&ax1));
            r[0].rhs = k * p.dot(&ofs);
            r[1].rhs = k * q.dot(&ofs);
            r[2].rhs = k * ax1.dot(&ofs);
        }
        set3(&mut r[3].relpos1_cross_normal, &p);
        set3(&mut r[4].relpos1_cross_normal, &q);
        set3(&mut r[3].relpos2_cross_normal, &neg3(&p));
        set3(&mut r[4].relpos2_cross_normal, &neg3(&q));
        k = info.fps * info.erp;
        let u = ax1_a.cross(&ax1_b);
        r[3].rhs = k * u.dot(&p);
        r[4].rhs = k * u.dot(&q);

        let mut nrow = 4;
        let mut limit_err = 0.0;
        let mut limit = 0;
        if solve_limit {
            limit_err = self.limit.correction * self.reference_sign;
            limit = if limit_err > 0.0 { 1 } else { 2 };
        }
        let mut powered = 0;
        if self.enable_angular_motor {
            powered = 1;
        }
        if limit != 0 || powered != 0 {
            nrow += 1;
            let row = &mut r[nrow];
            set3(&mut row.relpos1_cross_normal, &ax1);
            set3(&mut row.relpos2_cross_normal, &neg3(&ax1));
            let lostop = self.limit.get_low();
            let histop = self.limit.get_high();
            if limit != 0 && lostop == histop {
                powered = 0;
            }
            row.rhs = 0.0;
            let curr_erp = if self.flags & BT_HINGE_FLAGS_ERP_STOP != 0 {
                self.stop_erp
            } else {
                info.erp
            };
            if powered != 0 {
                if self.flags & BT_HINGE_FLAGS_CFM_NORM != 0 {
                    row.cfm = self.normal_cfm;
                }
                let mot_fact = get_motor_factor(
                    self.hinge_angle,
                    lostop,
                    histop,
                    self.motor_target_velocity,
                    info.fps * curr_erp,
                );
                row.rhs = fma(
                    mot_fact * self.motor_target_velocity,
                    self.reference_sign,
                    row.rhs,
                );
                row.lower_limit = -self.max_motor_impulse;
                row.upper_limit = self.max_motor_impulse;
            }
            if limit != 0 {
                k = info.fps * curr_erp;
                row.rhs = fma(k, limit_err, row.rhs);
                if self.flags & BT_HINGE_FLAGS_CFM_STOP != 0 {
                    row.cfm = self.stop_cfm;
                }
                if lostop == histop {
                    row.lower_limit = -SIMD_INFINITY;
                    row.upper_limit = SIMD_INFINITY;
                } else if limit == 1 {
                    row.lower_limit = 0.0;
                    row.upper_limit = SIMD_INFINITY;
                } else {
                    row.lower_limit = -SIMD_INFINITY;
                    row.upper_limit = 0.0;
                }
                let bounce = self.limit.relaxation_factor;
                if bounce > 0.0 {
                    let mut vel = ang_vel_a.dot(&ax1);
                    vel -= ang_vel_b.dot(&ax1);
                    if limit == 1 {
                        if vel < 0.0 {
                            let newc = -bounce * vel;
                            if newc > row.rhs {
                                row.rhs = newc;
                            }
                        }
                    } else if vel > 0.0 {
                        let newc = -bounce * vel;
                        if newc < row.rhs {
                            row.rhs = newc;
                        }
                    }
                }
                row.rhs *= self.limit.bias_factor;
            }
        }
    }

    /// `setParam`.
    pub fn set_param(&mut self, num: i32, value: f64, axis: i32) {
        if axis == -1 || axis == 5 {
            match num {
                BT_CONSTRAINT_STOP_ERP => {
                    self.stop_erp = value;
                    self.flags |= BT_HINGE_FLAGS_ERP_STOP;
                }
                BT_CONSTRAINT_STOP_CFM => {
                    self.stop_cfm = value;
                    self.flags |= BT_HINGE_FLAGS_CFM_STOP;
                }
                BT_CONSTRAINT_CFM => {
                    self.normal_cfm = value;
                    self.flags |= BT_HINGE_FLAGS_CFM_NORM;
                }
                _ => {}
            }
        }
    }

    /// `getParam`.
    pub fn get_param(&self, num: i32, axis: i32) -> f64 {
        if axis == -1 || axis == 5 {
            match num {
                BT_CONSTRAINT_STOP_ERP => return self.stop_erp,
                BT_CONSTRAINT_STOP_CFM => return self.stop_cfm,
                BT_CONSTRAINT_CFM => return self.normal_cfm,
                _ => {}
            }
        }
        0.0
    }
}

/// Writes x, y, z of `v` into a row vector (Bullet's `J[srow+i] = v[i]`
/// loops leave the fourth component alone).
#[inline(always)]
pub(crate) fn set3(dst: &mut Vec3, v: &Vec3) {
    dst.x = v.x;
    dst.y = v.y;
    dst.z = v.z;
}

#[inline(always)]
pub(crate) fn neg3(v: &Vec3) -> Vec3 {
    Vec3::new(-v.x, -v.y, -v.z)
}
