// Bullet Continuous Collision Detection and Physics Library
// btConeTwistConstraint is Copyright (c) 2007 Starbreeze Studios
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
// Written by: Marcus Hennix
//
// Altered for pz-rs: Rust translation of btConeTwistConstraint (Bullet 2.82,
// double precision), solver path only.

//! `btConeTwistConstraint`: a ball-socket joint with an elliptic swing cone
//! around the frames' x axis and a twist limit about it.
//!
//! Only the `getInfo1`/`getInfo2` path exists: `m_useSolveConstraintObsolete`
//! is `CONETWIST_USE_OBSOLETE_SOLVER` (false) and nothing in the game turns
//! it on, so `buildJacobian` and `solveConstraintObsolete` do no work. The
//! motor (`enableMotor`, `setMotorTarget*`) is never enabled by the game and
//! is not ported; `calcAngleInfo2`'s motor branch is therefore never taken.

use crate::bt::collision_object::{CollisionObject, ObjId};
use crate::bt::linear_math::*;

use super::hinge::{neg3, set3};
use super::typed_constraint::*;

pub const BT_CONETWIST_FLAGS_LIN_CFM: i32 = 1;
pub const BT_CONETWIST_FLAGS_LIN_ERP: i32 = 2;
pub const BT_CONETWIST_FLAGS_ANG_CFM: i32 = 4;

/// `CONETWIST_DEF_FIX_THRESH` (`.05f`).
pub const CONETWIST_DEF_FIX_THRESH: f64 = flt(0.05);

/// `vTwist`: the twist axis in constraint space.
const V_TWIST: Vec3 = Vec3::new(1.0, 0.0, 0.0);

/// `computeAngularImpulseDenominator(axis, invInertiaWorld)`.
#[inline(always)]
fn compute_angular_impulse_denominator(axis: &Vec3, inv_inertia_world: &Mat3) -> f64 {
    let vec = *axis * *inv_inertia_world;
    axis.dot(&vec)
}

/// `q1 * q2`, plus the negated x, y and z of the product the way the game's
/// build computes them for `qB.inverse()` inside `quatRotate(qB, ..)`: clang
/// folded the negation into the last fused multiply-add (`fnmadd`), which
/// gives `+0` where an exact negation gives `-0`.
fn quat_mul_with_negated_xyz(q1: &Quat, q2: &Quat) -> (Quat, Vec3) {
    let px = fma(q1.y, q2.z, fma(q1.w, q2.x, q1.x * q2.w));
    let py = fma(q1.z, q2.x, fma(q1.w, q2.y, q1.y * q2.w));
    let pz = fma(q1.x, q2.y, fma(q1.w, q2.z, q1.z * q2.w));
    let q = *q1 * *q2;
    let ninv = Vec3::new(
        fma(q1.z, q2.y, -px),
        fma(q1.x, q2.z, -py),
        fma(q1.y, q2.x, -pz),
    );
    (q, ninv)
}

/// `quatRotate(qB, -v)` as the game's `calcAngleInfo2` evaluates it: the
/// first product is the plain `qB * (-v)`; the product with `qB.inverse()`
/// uses the fused negated components `ninv` for three terms and `+qB` for
/// the fourth (`-q1.z * -qB.y` became `q1.z * qB.y`, and so on).
fn rotate_by_q_b_negated(q_b: &Quat, ninv: &Vec3, v: &Vec3) -> Vec3 {
    let q = *q_b * -*v;
    Vec3::new(
        fma(q.z, q_b.y, fma(q.y, ninv.z, fma(q.w, ninv.x, q_b.w * q.x))),
        fma(q.x, q_b.z, fma(q.z, ninv.x, fma(q.w, ninv.y, q_b.w * q.y))),
        fma(q.y, q_b.x, fma(q.x, ninv.y, fma(q.w, ninv.z, q_b.w * q.z))),
    )
}

/// `-a.cross(b)` with the negation folded into the fused multiply-add, as
/// the game's build has it (an exact zero comes out as `+0`).
fn neg_cross(a: &Vec3, b: &Vec3) -> Vec3 {
    Vec3::new(
        fma(-a.y, b.z, a.z * b.y),
        fma(-a.z, b.x, a.x * b.z),
        fma(-a.x, b.y, a.y * b.x),
    )
}

/// `btConeTwistConstraint`.
#[derive(Clone, Debug)]
pub struct ConeTwistConstraint {
    pub rb_a_frame: Transform,
    pub rb_b_frame: Transform,
    pub limit_softness: f64,
    pub bias_factor: f64,
    pub relaxation_factor: f64,
    pub damping: f64,
    pub swing_span1: f64,
    pub swing_span2: f64,
    pub twist_span: f64,
    pub fix_thresh: f64,
    pub swing_axis: Vec3,
    pub twist_axis: Vec3,
    pub k_swing: f64,
    pub k_twist: f64,
    pub twist_limit_sign: f64,
    pub swing_correction: f64,
    pub twist_correction: f64,
    pub twist_angle: f64,
    pub acc_swing_limit_impulse: f64,
    pub acc_twist_limit_impulse: f64,
    pub angular_only: bool,
    pub solve_twist_limit: bool,
    pub solve_swing_limit: bool,
    pub use_solve_constraint_obsolete: bool,
    pub swing_limit_ratio: f64,
    pub twist_limit_ratio: f64,
    pub twist_axis_a: Vec3,
    pub max_motor_impulse: f64,
    pub flags: i32,
    pub lin_cfm: f64,
    pub lin_erp: f64,
    pub ang_cfm: f64,
}

impl ConeTwistConstraint {
    fn with_frames(rb_a_frame: Transform, rb_b_frame: Transform) -> ConeTwistConstraint {
        let mut c = ConeTwistConstraint {
            rb_a_frame,
            rb_b_frame,
            limit_softness: 0.0,
            bias_factor: 0.0,
            relaxation_factor: 0.0,
            damping: 0.0,
            swing_span1: 0.0,
            swing_span2: 0.0,
            twist_span: 0.0,
            fix_thresh: 0.0,
            swing_axis: Vec3::ZERO,
            twist_axis: Vec3::ZERO,
            k_swing: 0.0,
            k_twist: 0.0,
            twist_limit_sign: 0.0,
            swing_correction: 0.0,
            twist_correction: 0.0,
            twist_angle: 0.0,
            acc_swing_limit_impulse: 0.0,
            acc_twist_limit_impulse: 0.0,
            angular_only: false,
            solve_twist_limit: false,
            solve_swing_limit: false,
            use_solve_constraint_obsolete: false,
            swing_limit_ratio: 0.0,
            twist_limit_ratio: 0.0,
            twist_axis_a: Vec3::ZERO,
            max_motor_impulse: 0.0,
            flags: 0,
            lin_cfm: 0.0,
            lin_erp: 0.0,
            ang_cfm: 0.0,
        };
        c.init();
        c
    }

    /// `btConeTwistConstraint(rbA, rbB, rbAFrame, rbBFrame)`.
    pub fn new_typed(
        rb_a: ObjId,
        rb_b: ObjId,
        rb_a_frame: Transform,
        rb_b_frame: Transform,
    ) -> TypedConstraint {
        TypedConstraint::new(
            CONETWIST_CONSTRAINT_TYPE,
            Some(rb_a),
            Some(rb_b),
            ConstraintKind::ConeTwist(Box::new(ConeTwistConstraint::with_frames(
                rb_a_frame, rb_b_frame,
            ))),
        )
    }

    /// `btConeTwistConstraint(rbA, rbAFrame)`: body B is the fixed body and
    /// its frame is rbAFrame's rotation at the origin.
    pub fn new_typed_single(rb_a: ObjId, rb_a_frame: Transform) -> TypedConstraint {
        let mut rb_b_frame = rb_a_frame;
        rb_b_frame.origin = Vec3::new(0.0, 0.0, 0.0);
        TypedConstraint::new(
            CONETWIST_CONSTRAINT_TYPE,
            Some(rb_a),
            None,
            ConstraintKind::ConeTwist(Box::new(ConeTwistConstraint::with_frames(
                rb_a_frame, rb_b_frame,
            ))),
        )
    }

    /// `init()`.
    fn init(&mut self) {
        self.angular_only = false;
        self.solve_twist_limit = false;
        self.solve_swing_limit = false;
        self.max_motor_impulse = -1.0;
        self.set_limit(
            BT_LARGE_FLOAT,
            BT_LARGE_FLOAT,
            BT_LARGE_FLOAT,
            1.0,
            flt(0.3),
            1.0,
        );
        self.damping = 0.01;
        self.fix_thresh = CONETWIST_DEF_FIX_THRESH;
        self.flags = 0;
        self.lin_cfm = 0.0;
        self.lin_erp = flt(0.7);
        self.ang_cfm = 0.0;
    }

    /// `setLimit(swingSpan1, swingSpan2, twistSpan, softness = 1.f,
    /// biasFactor = 0.3f, relaxationFactor = 1.0f)`.
    pub fn set_limit(
        &mut self,
        swing_span1: f64,
        swing_span2: f64,
        twist_span: f64,
        softness: f64,
        bias_factor: f64,
        relaxation_factor: f64,
    ) {
        self.swing_span1 = swing_span1;
        self.swing_span2 = swing_span2;
        self.twist_span = twist_span;
        self.limit_softness = softness;
        self.bias_factor = bias_factor;
        self.relaxation_factor = relaxation_factor;
    }

    /// `setLimit(limitIndex, limitValue)`: 3 = twist, 4 = swing 2,
    /// 5 = swing 1.
    pub fn set_limit_index(&mut self, limit_index: i32, limit_value: f64) {
        match limit_index {
            3 => self.twist_span = limit_value,
            4 => self.swing_span2 = limit_value,
            5 => self.swing_span1 = limit_value,
            _ => {}
        }
    }

    /// `setDamping` (only read by the obsolete solver).
    pub fn set_damping(&mut self, damping: f64) {
        self.damping = damping;
    }

    /// `setAngularOnly`.
    pub fn set_angular_only(&mut self, angular_only: bool) {
        self.angular_only = angular_only;
    }

    /// `setFrames(frameA, frameB)` (its `buildJacobian` call does nothing).
    pub fn set_frames(&mut self, frame_a: &Transform, frame_b: &Transform) {
        self.rb_a_frame = *frame_a;
        self.rb_b_frame = *frame_b;
    }

    /// `getInfo1`.
    pub fn get_info1(
        &mut self,
        info: &mut ConstraintInfo1,
        a: &CollisionObject,
        b: &CollisionObject,
    ) {
        if self.use_solve_constraint_obsolete {
            info.num_constraint_rows = 0;
            info.nub = 0;
            return;
        }
        info.num_constraint_rows = 3;
        info.nub = 3;
        self.calc_angle_info2(
            &a.world_transform,
            &b.world_transform,
            &a.rb().inv_inertia_tensor_world,
            &b.rb().inv_inertia_tensor_world,
        );
        if self.solve_swing_limit {
            info.num_constraint_rows += 1;
            info.nub -= 1;
            if self.swing_span1 < self.fix_thresh && self.swing_span2 < self.fix_thresh {
                info.num_constraint_rows += 1;
                info.nub -= 1;
            }
        }
        if self.solve_twist_limit {
            info.num_constraint_rows += 1;
            info.nub -= 1;
        }
    }

    /// `getInfo2` (`getInfo2NonVirtual` with the bodies' transforms and
    /// world inverse inertias).
    pub fn get_info2(
        &mut self,
        info: &mut ConstraintInfo2,
        a: &CollisionObject,
        b: &CollisionObject,
    ) {
        let trans_a = a.world_transform;
        let trans_b = b.world_transform;
        self.calc_angle_info2(
            &trans_a,
            &trans_b,
            &a.rb().inv_inertia_tensor_world,
            &b.rb().inv_inertia_tensor_world,
        );
        let r = &mut *info.rows;
        r[0].contact_normal1.x = 1.0;
        r[1].contact_normal1.y = 1.0;
        r[2].contact_normal1.z = 1.0;
        let a1 = trans_a.basis * self.rb_a_frame.origin;
        {
            let a1neg = -a1;
            let (mut v0, mut v1, mut v2) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
            a1neg.get_skew_symmetric_matrix(&mut v0, &mut v1, &mut v2);
            r[0].relpos1_cross_normal = v0;
            r[1].relpos1_cross_normal = v1;
            r[2].relpos1_cross_normal = v2;
        }
        r[0].contact_normal2.x = -1.0;
        r[1].contact_normal2.y = -1.0;
        r[2].contact_normal2.z = -1.0;
        let a2 = trans_b.basis * self.rb_b_frame.origin;
        {
            let (mut v0, mut v1, mut v2) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
            a2.get_skew_symmetric_matrix(&mut v0, &mut v1, &mut v2);
            r[0].relpos2_cross_normal = v0;
            r[1].relpos2_cross_normal = v1;
            r[2].relpos2_cross_normal = v2;
        }
        let lin_erp = if self.flags & BT_CONETWIST_FLAGS_LIN_ERP != 0 {
            self.lin_erp
        } else {
            info.erp
        };
        let k = info.fps * lin_erp;
        for j in 0..3 {
            r[j].rhs = k * (a2[j] + trans_b.origin[j] - a1[j] - trans_a.origin[j]);
            r[j].lower_limit = -SIMD_INFINITY;
            r[j].upper_limit = SIMD_INFINITY;
            if self.flags & BT_CONETWIST_FLAGS_LIN_CFM != 0 {
                r[j].cfm = self.lin_cfm;
            }
        }
        let mut srow = 3;
        if self.solve_swing_limit {
            if self.swing_span1 < self.fix_thresh && self.swing_span2 < self.fix_thresh {
                let tr_a = trans_a * self.rb_a_frame;
                let p = tr_a.basis.get_column(1);
                let q = tr_a.basis.get_column(2);
                let srow1 = srow + 1;
                set3(&mut r[srow].relpos1_cross_normal, &p);
                set3(&mut r[srow1].relpos1_cross_normal, &q);
                set3(&mut r[srow].relpos2_cross_normal, &neg3(&p));
                set3(&mut r[srow1].relpos2_cross_normal, &neg3(&q));
                let fact = info.fps * self.relaxation_factor;
                r[srow].rhs = fact * self.swing_axis.dot(&p);
                r[srow1].rhs = fact * self.swing_axis.dot(&q);
                r[srow].lower_limit = -SIMD_INFINITY;
                r[srow].upper_limit = SIMD_INFINITY;
                r[srow1].lower_limit = -SIMD_INFINITY;
                r[srow1].upper_limit = SIMD_INFINITY;
                srow = srow1 + 1;
            } else {
                let ax1 = self.swing_axis * self.relaxation_factor * self.relaxation_factor;
                set3(&mut r[srow].relpos1_cross_normal, &ax1);
                set3(&mut r[srow].relpos2_cross_normal, &neg3(&ax1));
                let k = info.fps * self.bias_factor;
                r[srow].rhs = k * self.swing_correction;
                if self.flags & BT_CONETWIST_FLAGS_ANG_CFM != 0 {
                    r[srow].cfm = self.ang_cfm;
                }
                r[srow].lower_limit = 0.0;
                r[srow].upper_limit = SIMD_INFINITY;
                srow += 1;
            }
        }
        if self.solve_twist_limit {
            let ax1 = self.twist_axis * self.relaxation_factor * self.relaxation_factor;
            set3(&mut r[srow].relpos1_cross_normal, &ax1);
            set3(&mut r[srow].relpos2_cross_normal, &neg3(&ax1));
            let k = info.fps * self.bias_factor;
            r[srow].rhs = k * self.twist_correction;
            if self.flags & BT_CONETWIST_FLAGS_ANG_CFM != 0 {
                r[srow].cfm = self.ang_cfm;
            }
            if self.twist_span > 0.0 {
                if self.twist_correction > 0.0 {
                    r[srow].lower_limit = 0.0;
                    r[srow].upper_limit = SIMD_INFINITY;
                } else {
                    r[srow].lower_limit = -SIMD_INFINITY;
                    r[srow].upper_limit = 0.0;
                }
            } else {
                r[srow].lower_limit = -SIMD_INFINITY;
                r[srow].upper_limit = SIMD_INFINITY;
            }
        }
    }

    /// `calcAngleInfo2(transA, transB, invInertiaWorldA, invInertiaWorldB)`
    /// (without the motor branch, see the module docs).
    pub fn calc_angle_info2(
        &mut self,
        trans_a: &Transform,
        trans_b: &Transform,
        inv_inertia_world_a: &Mat3,
        inv_inertia_world_b: &Mat3,
    ) {
        self.swing_correction = 0.0;
        self.twist_limit_sign = 0.0;
        self.solve_twist_limit = false;
        self.solve_swing_limit = false;

        let q_a = trans_a.get_rotation() * self.rb_a_frame.get_rotation();
        let (q_b, q_b_ninv) =
            quat_mul_with_negated_xyz(&trans_b.get_rotation(), &self.rb_b_frame.get_rotation());
        let q_ab = q_b.inverse() * q_a;
        let mut v_cone_no_twist = quat_rotate(&q_ab, &V_TWIST);
        v_cone_no_twist.normalize();
        let mut q_ab_cone = shortest_arc_quat(&V_TWIST, &v_cone_no_twist);
        q_ab_cone.normalize();
        let mut q_ab_twist = q_ab_cone.inverse() * q_ab;
        q_ab_twist.normalize();

        if self.swing_span1 >= self.fix_thresh && self.swing_span2 >= self.fix_thresh {
            let (swing_angle, mut swing_axis, swing_limit) =
                self.compute_cone_limit_info(&q_ab_cone);
            if swing_angle > swing_limit * self.limit_softness {
                self.solve_swing_limit = true;
                self.swing_limit_ratio = 1.0;
                if swing_angle < swing_limit && self.limit_softness < flt(1.0) - SIMD_EPSILON {
                    self.swing_limit_ratio = fma(-swing_limit, self.limit_softness, swing_angle)
                        / fma(-swing_limit, self.limit_softness, swing_limit);
                }
                self.swing_correction = fma(-swing_limit, self.limit_softness, swing_angle);
                self.adjust_swing_axis_to_use_ellipse_normal(&mut swing_axis);
                self.swing_axis = rotate_by_q_b_negated(&q_b, &q_b_ninv, &swing_axis);
                self.twist_axis_a.set_value(0.0, 0.0, 0.0);
                self.k_swing = 1.0
                    / (compute_angular_impulse_denominator(&self.swing_axis, inv_inertia_world_a)
                        + compute_angular_impulse_denominator(
                            &self.swing_axis,
                            inv_inertia_world_b,
                        ));
            }
        } else {
            let iv_a = trans_a.basis * self.rb_a_frame.basis.get_column(0);
            let jv_a = trans_a.basis * self.rb_a_frame.basis.get_column(1);
            let kv_a = trans_a.basis * self.rb_a_frame.basis.get_column(2);
            let iv_b = trans_b.basis * self.rb_b_frame.basis.get_column(0);
            let mut x = iv_b.dot(&iv_a);
            let mut y = iv_b.dot(&jv_a);
            let mut z = iv_b.dot(&kv_a);
            if self.swing_span1 < self.fix_thresh && self.swing_span2 < self.fix_thresh {
                if !bt_fuzzy_zero(y) || !bt_fuzzy_zero(z) {
                    self.solve_swing_limit = true;
                    self.swing_axis = neg_cross(&iv_b, &iv_a);
                }
            } else {
                if self.swing_span1 < self.fix_thresh {
                    if !bt_fuzzy_zero(x) || !bt_fuzzy_zero(z) {
                        self.solve_swing_limit = true;
                        if self.swing_span2 >= self.fix_thresh {
                            y = 0.0;
                            let span2 = bt_atan2(z, x);
                            // btCos/btSin of one argument: one __sincos_stret.
                            if span2 > self.swing_span2 {
                                let (sn, cs) = bt_sincos(self.swing_span2);
                                x = cs;
                                z = sn;
                            } else if span2 < -self.swing_span2 {
                                let (sn, cs) = bt_sincos(self.swing_span2);
                                x = cs;
                                z = -sn;
                            }
                        }
                    }
                } else if !bt_fuzzy_zero(x) || !bt_fuzzy_zero(y) {
                    self.solve_swing_limit = true;
                    if self.swing_span1 >= self.fix_thresh {
                        z = 0.0;
                        let span1 = bt_atan2(y, x);
                        if span1 > self.swing_span1 {
                            let (sn, cs) = bt_sincos(self.swing_span1);
                            x = cs;
                            y = sn;
                        } else if span1 < -self.swing_span1 {
                            let (sn, cs) = bt_sincos(self.swing_span1);
                            x = cs;
                            y = -sn;
                        }
                    }
                }
                let mut target = Vec3::ZERO;
                target.x = fma(z, kv_a.x, fma(x, iv_a.x, y * jv_a.x));
                target.y = fma(z, kv_a.y, fma(x, iv_a.y, y * jv_a.y));
                target.z = fma(z, kv_a.z, fma(x, iv_a.z, y * jv_a.z));
                target.normalize();
                self.swing_axis = -iv_b.cross(&target);
                self.swing_correction = self.swing_axis.length();
                self.swing_axis.normalize();
            }
        }

        if self.twist_span >= 0.0 {
            let (twist_angle, twist_axis) = Self::compute_twist_limit_info(&q_ab_twist);
            self.twist_angle = twist_angle;
            if self.twist_angle > self.twist_span * self.limit_softness {
                self.solve_twist_limit = true;
                self.twist_limit_ratio = 1.0;
                if self.twist_angle < self.twist_span
                    && self.limit_softness < flt(1.0) - SIMD_EPSILON
                {
                    self.twist_limit_ratio =
                        fma(-self.twist_span, self.limit_softness, self.twist_angle)
                            / fma(-self.twist_span, self.limit_softness, self.twist_span);
                }
                self.twist_correction =
                    fma(-self.twist_span, self.limit_softness, self.twist_angle);
                self.twist_axis = rotate_by_q_b_negated(&q_b, &q_b_ninv, &twist_axis);
                self.k_twist = 1.0
                    / (compute_angular_impulse_denominator(&self.twist_axis, inv_inertia_world_a)
                        + compute_angular_impulse_denominator(
                            &self.twist_axis,
                            inv_inertia_world_b,
                        ));
            }
            if self.solve_swing_limit {
                self.twist_axis_a = quat_rotate(&q_a, &-twist_axis);
            }
        } else {
            self.twist_angle = 0.0;
        }
    }

    /// `computeConeLimitInfo(qCone, swingAngle, vSwingAxis, swingLimit)`:
    /// returns `(swingAngle, vSwingAxis, swingLimit)`. With a swing angle
    /// at or below epsilon Bullet leaves the axis untouched; the caller's
    /// value is uninitialised there and never read, so zero is returned.
    pub fn compute_cone_limit_info(&self, q_cone: &Quat) -> (f64, Vec3, f64) {
        let swing_angle = q_cone.get_angle();
        let mut v_swing_axis = Vec3::ZERO;
        let mut swing_limit = 0.0;
        if swing_angle > SIMD_EPSILON {
            v_swing_axis = Vec3::new(q_cone.x, q_cone.y, q_cone.z);
            v_swing_axis.normalize();
            let x_ellipse = v_swing_axis.y;
            let y_ellipse = -v_swing_axis.z;
            swing_limit = self.swing_span1;
            if x_ellipse.abs() > SIMD_EPSILON {
                let surface_slope2 = (y_ellipse * y_ellipse) / (x_ellipse * x_ellipse);
                let mut norm = 1.0 / (self.swing_span2 * self.swing_span2);
                norm += surface_slope2 / (self.swing_span1 * self.swing_span1);
                let swing_limit2 = (1.0 + surface_slope2) / norm;
                swing_limit = swing_limit2.sqrt();
            }
        }
        (swing_angle, v_swing_axis, swing_limit)
    }

    /// `computeTwistLimitInfo(qTwist, twistAngle, vTwistAxis)`: returns
    /// `(twistAngle, vTwistAxis)`.
    pub fn compute_twist_limit_info(q_twist: &Quat) -> (f64, Vec3) {
        let mut q_min_twist = *q_twist;
        let mut twist_angle = q_twist.get_angle();
        if twist_angle > SIMD_PI {
            q_min_twist = -*q_twist;
            twist_angle = q_min_twist.get_angle();
        }
        let mut v_twist_axis = Vec3::new(q_min_twist.x, q_min_twist.y, q_min_twist.z);
        if twist_angle > SIMD_EPSILON {
            v_twist_axis.normalize();
        }
        (twist_angle, v_twist_axis)
    }

    /// `adjustSwingAxisToUseEllipseNormal(vSwingAxis)`.
    pub fn adjust_swing_axis_to_use_ellipse_normal(&self, v_swing_axis: &mut Vec3) {
        let mut y = -v_swing_axis.z;
        let z = v_swing_axis.y;
        if z.abs() > SIMD_EPSILON {
            let mut grad = y / z;
            grad *= self.swing_span2 / self.swing_span1;
            if y > 0.0 {
                y = (grad * z).abs();
            } else {
                y = -(grad * z).abs();
            }
            v_swing_axis.z = -y;
            v_swing_axis.y = z;
            v_swing_axis.normalize();
        }
    }

    /// `GetPointForAngle(fAngleInRadians, fLength)` (debug drawing).
    pub fn get_point_for_angle(&self, f_angle_in_radians: f64, f_length: f64) -> Vec3 {
        let (y_ellipse, x_ellipse) = bt_sincos(f_angle_in_radians);
        // `float swingLimit`: the span and the computed limit are rounded to
        // float.
        let mut swing_limit = self.swing_span1 as f32;
        if x_ellipse.abs() > SIMD_EPSILON {
            let surface_slope2 = (y_ellipse * y_ellipse) / (x_ellipse * x_ellipse);
            let mut norm = 1.0 / (self.swing_span2 * self.swing_span2);
            norm += surface_slope2 / (self.swing_span1 * self.swing_span1);
            let swing_limit2 = (1.0 + surface_slope2) / norm;
            swing_limit = swing_limit2.sqrt() as f32;
        }
        let v_swing_axis = Vec3::new(0.0, x_ellipse, -y_ellipse);
        let q_swing = Quat::from_axis_angle(&v_swing_axis, swing_limit as f64);
        let v_point_in_constraint_space = Vec3::new(f_length, 0.0, 0.0);
        quat_rotate(&q_swing, &v_point_in_constraint_space)
    }

    /// `setParam(num, value, axis)`.
    pub fn set_param(&mut self, num: i32, value: f64, axis: i32) {
        match num {
            BT_CONSTRAINT_ERP | BT_CONSTRAINT_STOP_ERP => {
                if (0..3).contains(&axis) {
                    self.lin_erp = value;
                    self.flags |= BT_CONETWIST_FLAGS_LIN_ERP;
                } else {
                    self.bias_factor = value;
                }
            }
            BT_CONSTRAINT_CFM | BT_CONSTRAINT_STOP_CFM => {
                if (0..3).contains(&axis) {
                    self.lin_cfm = value;
                    self.flags |= BT_CONETWIST_FLAGS_LIN_CFM;
                } else {
                    self.ang_cfm = value;
                    self.flags |= BT_CONETWIST_FLAGS_ANG_CFM;
                }
            }
            _ => {}
        }
    }

    /// `getParam(num, axis)`.
    pub fn get_param(&self, num: i32, axis: i32) -> f64 {
        match num {
            BT_CONSTRAINT_ERP | BT_CONSTRAINT_STOP_ERP => {
                if (0..3).contains(&axis) {
                    self.lin_erp
                } else if (3..6).contains(&axis) {
                    self.bias_factor
                } else {
                    0.0
                }
            }
            BT_CONSTRAINT_CFM | BT_CONSTRAINT_STOP_CFM => {
                if (0..3).contains(&axis) {
                    self.lin_cfm
                } else if (3..6).contains(&axis) {
                    self.ang_cfm
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }
}
