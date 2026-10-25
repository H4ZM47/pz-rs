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
// Altered for pz-rs: Rust translation of btTypedConstraint and btAngularLimit
// (Bullet 2.82, double precision).

//! `btTypedConstraint`: common constraint data, the constraint arena and the
//! `btConstraintInfo1/2` row interface.
//!
//! A body reference of `None` is Bullet's static `getFixedBody()` (mass 0,
//! identity transform, never moved).

use std::sync::OnceLock;

use crate::bt::collision_object::*;
use crate::bt::linear_math::*;

use super::generic_6dof::Generic6DofConstraint;
use super::hinge::HingeConstraint;
use super::point2point::Point2PointConstraint;
use super::solver_body::SolverConstraint;

pub const POINT2POINT_CONSTRAINT_TYPE: i32 = 3;
pub const HINGE_CONSTRAINT_TYPE: i32 = 4;
pub const D6_CONSTRAINT_TYPE: i32 = 6;

/// `btConstraintParams`.
pub const BT_CONSTRAINT_ERP: i32 = 1;
pub const BT_CONSTRAINT_STOP_ERP: i32 = 2;
pub const BT_CONSTRAINT_CFM: i32 = 3;
pub const BT_CONSTRAINT_STOP_CFM: i32 = 4;

/// `btJointFeedback`.
#[derive(Clone, Copy, Debug, Default)]
pub struct JointFeedback {
    pub applied_force_body_a: Vec3,
    pub applied_torque_body_a: Vec3,
    pub applied_force_body_b: Vec3,
    pub applied_torque_body_b: Vec3,
}

/// `btConstraintInfo1`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConstraintInfo1 {
    pub num_constraint_rows: i32,
    pub nub: i32,
}

/// `btConstraintInfo2`: the rows are the solver's `btSolverConstraint`s of
/// this constraint. Bullet's raw pointers map to row fields:
/// `m_J1linearAxis` = `contact_normal1`, `m_J1angularAxis` =
/// `relpos1_cross_normal`, `m_J2linearAxis` = `contact_normal2`,
/// `m_J2angularAxis` = `relpos2_cross_normal`, `m_constraintError` = `rhs`,
/// `cfm`, `m_lowerLimit`, `m_upperLimit`.
pub struct ConstraintInfo2<'a> {
    pub fps: f64,
    pub erp: f64,
    pub rows: &'a mut [SolverConstraint],
    pub damping: f64,
    pub num_iterations: i32,
}

/// Accessors for the constraint rows' data, by body.
pub trait BodyAccess {
    fn obj(&self, id: ObjId) -> &CollisionObject;
}
impl BodyAccess for [CollisionObject] {
    #[inline]
    fn obj(&self, id: ObjId) -> &CollisionObject {
        &self[id]
    }
}
impl BodyAccess for Vec<CollisionObject> {
    #[inline]
    fn obj(&self, id: ObjId) -> &CollisionObject {
        &self[id]
    }
}
impl BodyAccess for Vec<Option<CollisionObject>> {
    #[inline]
    fn obj(&self, id: ObjId) -> &CollisionObject {
        self[id].as_ref().expect("dangling object id")
    }
}

/// Mutable object arena access, for the solver.
pub trait BodyArena: BodyAccess {
    fn obj_mut(&mut self, id: ObjId) -> &mut CollisionObject;
}
impl BodyArena for [CollisionObject] {
    #[inline]
    fn obj_mut(&mut self, id: ObjId) -> &mut CollisionObject {
        &mut self[id]
    }
}
impl BodyArena for Vec<CollisionObject> {
    #[inline]
    fn obj_mut(&mut self, id: ObjId) -> &mut CollisionObject {
        &mut self[id]
    }
}
impl BodyArena for Vec<Option<CollisionObject>> {
    #[inline]
    fn obj_mut(&mut self, id: ObjId) -> &mut CollisionObject {
        self[id].as_mut().expect("dangling object id")
    }
}

/// `btTypedConstraint::getFixedBody()`: `btRigidBody(0, 0, 0)` after
/// `setMassProps(0, 0)`.
pub fn fixed_body() -> &'static CollisionObject {
    static FIXED: OnceLock<CollisionObject> = OnceLock::new();
    FIXED.get_or_init(|| {
        let info = super::rigid_body::RigidBodyConstructionInfo::new(0.0, None, None, Vec3::ZERO);
        let mut o = CollisionObject::new_rigid_body(&info);
        o.set_mass_props(0.0, &Vec3::ZERO);
        o
    })
}

/// Resolves a constraint body reference (`None` = fixed body).
#[inline]
pub fn body_ref<A: BodyAccess + ?Sized>(objects: &A, id: Option<ObjId>) -> &CollisionObject {
    match id {
        Some(i) => objects.obj(i),
        None => fixed_body(),
    }
}

/// The concrete constraint.
#[derive(Clone, Debug)]
pub enum ConstraintKind {
    Point2Point(Point2PointConstraint),
    Hinge(Box<HingeConstraint>),
    Generic6Dof(Box<Generic6DofConstraint>),
}

/// `btTypedConstraint`.
#[derive(Clone, Debug)]
pub struct TypedConstraint {
    pub object_type: i32,
    pub user_constraint_type: i32,
    pub user_constraint_id: i32,
    pub breaking_impulse_threshold: f64,
    pub is_enabled: bool,
    pub needs_feedback: bool,
    pub override_num_solver_iterations: i32,
    /// `m_rbA` (`None` = fixed body).
    pub rb_a: Option<ObjId>,
    /// `m_rbB` (`None` = fixed body).
    pub rb_b: Option<ObjId>,
    pub applied_impulse: f64,
    pub dbg_draw_size: f64,
    pub joint_feedback: Option<JointFeedback>,
    pub kind: ConstraintKind,
}

impl TypedConstraint {
    /// `btTypedConstraint(type, rbA, rbB)` around a concrete constraint.
    pub fn new(
        object_type: i32,
        rb_a: Option<ObjId>,
        rb_b: Option<ObjId>,
        kind: ConstraintKind,
    ) -> TypedConstraint {
        TypedConstraint {
            object_type,
            user_constraint_type: -1,
            user_constraint_id: -1,
            breaking_impulse_threshold: SIMD_INFINITY,
            is_enabled: true,
            needs_feedback: false,
            override_num_solver_iterations: -1,
            rb_a,
            rb_b,
            applied_impulse: 0.0,
            dbg_draw_size: flt(0.3),
            joint_feedback: None,
            kind,
        }
    }

    /// `buildJacobian`: only does work with `m_useSolveConstraintObsolete`,
    /// which the game never enables.
    pub fn build_jacobian(&mut self) {}

    /// `getInfo1`.
    pub fn get_info1<A: BodyAccess + ?Sized>(&mut self, objects: &A, info: &mut ConstraintInfo1) {
        let ta = body_ref(objects, self.rb_a).world_transform;
        let tb = body_ref(objects, self.rb_b).world_transform;
        let ma = body_ref(objects, self.rb_a).rb().inverse_mass;
        let mb = body_ref(objects, self.rb_b).rb().inverse_mass;
        match &mut self.kind {
            ConstraintKind::Point2Point(c) => c.get_info1(info),
            ConstraintKind::Hinge(c) => c.get_info1(info, &ta, &tb),
            ConstraintKind::Generic6Dof(c) => c.get_info1(info, &ta, &tb, ma, mb),
        }
    }

    /// `getInfo2`.
    pub fn get_info2<A: BodyAccess + ?Sized>(&mut self, objects: &A, info: &mut ConstraintInfo2) {
        let a = body_ref(objects, self.rb_a);
        let b = body_ref(objects, self.rb_b);
        match &mut self.kind {
            ConstraintKind::Point2Point(c) => {
                c.get_info2(info, &a.world_transform, &b.world_transform)
            }
            ConstraintKind::Hinge(c) => c.get_info2(info, a, b),
            ConstraintKind::Generic6Dof(c) => c.get_info2(info, a, b),
        }
    }

    /// `setParam(num, value, axis)`.
    pub fn set_param(&mut self, num: i32, value: f64, axis: i32) {
        match &mut self.kind {
            ConstraintKind::Point2Point(c) => c.set_param(num, value, axis),
            ConstraintKind::Hinge(c) => c.set_param(num, value, axis),
            ConstraintKind::Generic6Dof(c) => c.set_param(num, value, axis),
        }
    }

    /// `getParam(num, axis)`.
    pub fn get_param(&self, num: i32, axis: i32) -> f64 {
        match &self.kind {
            ConstraintKind::Point2Point(c) => c.get_param(num, axis),
            ConstraintKind::Hinge(c) => c.get_param(num, axis),
            ConstraintKind::Generic6Dof(c) => c.get_param(num, axis),
        }
    }
}

/// Constraint arena keyed by [`ConstraintId`].
#[derive(Clone, Debug, Default)]
pub struct ConstraintArena {
    pub slots: Vec<Option<TypedConstraint>>,
    free: Vec<ConstraintId>,
}

impl ConstraintArena {
    pub fn insert(&mut self, c: TypedConstraint) -> ConstraintId {
        if let Some(id) = self.free.pop() {
            self.slots[id] = Some(c);
            id
        } else {
            self.slots.push(Some(c));
            self.slots.len() - 1
        }
    }
    pub fn remove(&mut self, id: ConstraintId) -> Option<TypedConstraint> {
        let c = self.slots.get_mut(id).and_then(|s| s.take());
        if c.is_some() {
            self.free.push(id);
        }
        c
    }
    #[inline]
    pub fn get(&self, id: ConstraintId) -> &TypedConstraint {
        self.slots[id].as_ref().expect("dangling constraint id")
    }
    #[inline]
    pub fn get_mut(&mut self, id: ConstraintId) -> &mut TypedConstraint {
        self.slots[id].as_mut().expect("dangling constraint id")
    }
}

/// `btTypedConstraint::getMotorFactor`.
pub fn get_motor_factor(pos: f64, low_lim: f64, upp_lim: f64, vel: f64, time_fact: f64) -> f64 {
    if low_lim > upp_lim {
        return 1.0;
    } else if low_lim == upp_lim {
        return 0.0;
    }
    let delta_max = vel / time_fact;
    if delta_max < 0.0 {
        if pos >= low_lim && pos < (low_lim - delta_max) {
            (low_lim - pos) / delta_max
        } else if pos < low_lim {
            0.0
        } else {
            1.0
        }
    } else if delta_max > 0.0 {
        if pos <= upp_lim && pos > (upp_lim - delta_max) {
            (upp_lim - pos) / delta_max
        } else if pos > upp_lim {
            0.0
        } else {
            1.0
        }
    } else {
        0.0
    }
}

/// `btAdjustAngleToLimits`.
pub fn adjust_angle_to_limits(angle: f64, lower: f64, upper: f64) -> f64 {
    if lower >= upper {
        angle
    } else if angle < lower {
        let diff_lo = bt_normalize_angle(lower - angle).abs();
        let diff_hi = bt_normalize_angle(upper - angle).abs();
        if diff_lo < diff_hi {
            angle
        } else {
            angle + SIMD_2_PI
        }
    } else if angle > upper {
        let diff_hi = bt_normalize_angle(angle - upper).abs();
        let diff_lo = bt_normalize_angle(angle - lower).abs();
        if diff_lo < diff_hi {
            angle - SIMD_2_PI
        } else {
            angle
        }
    } else {
        angle
    }
}

/// `btAngularLimit`.
#[derive(Clone, Copy, Debug)]
pub struct AngularLimit {
    pub center: f64,
    pub half_range: f64,
    pub softness: f64,
    pub bias_factor: f64,
    pub relaxation_factor: f64,
    pub correction: f64,
    pub sign: f64,
    pub solve_limit: bool,
}

impl Default for AngularLimit {
    fn default() -> Self {
        AngularLimit {
            center: 0.0,
            half_range: -1.0,
            softness: flt(0.9),
            bias_factor: flt(0.3),
            relaxation_factor: 1.0,
            correction: 0.0,
            sign: 0.0,
            solve_limit: false,
        }
    }
}

impl AngularLimit {
    /// `set(low, high, softness, biasFactor, relaxationFactor)`.
    pub fn set(
        &mut self,
        low: f64,
        high: f64,
        softness: f64,
        bias_factor: f64,
        relaxation_factor: f64,
    ) {
        self.half_range = (high - low) / 2.0;
        self.center = bt_normalize_angle(low + self.half_range);
        self.softness = softness;
        self.bias_factor = bias_factor;
        self.relaxation_factor = relaxation_factor;
    }
    /// `test(angle)`.
    pub fn test(&mut self, angle: f64) {
        self.correction = 0.0;
        self.sign = 0.0;
        self.solve_limit = false;
        if self.half_range >= 0.0 {
            let deviation = bt_normalize_angle(angle - self.center);
            if deviation < -self.half_range {
                self.solve_limit = true;
                self.correction = -(deviation + self.half_range);
                self.sign = 1.0;
            } else if deviation > self.half_range {
                self.solve_limit = true;
                self.correction = self.half_range - deviation;
                self.sign = -1.0;
            }
        }
    }
    pub fn get_error(&self) -> f64 {
        self.correction * self.sign
    }
    /// `fit(angle)`.
    pub fn fit(&self, angle: &mut f64) {
        if self.half_range > 0.0 {
            let relative_angle = bt_normalize_angle(*angle - self.center);
            // btEqual(a, eps): (a <= eps) && !(a < -eps)
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            let equal = relative_angle <= self.half_range && !(relative_angle < -self.half_range);
            if !equal {
                if relative_angle > 0.0 {
                    *angle = self.get_high();
                } else {
                    *angle = self.get_low();
                }
            }
        }
    }
    pub fn get_low(&self) -> f64 {
        bt_normalize_angle(self.center - self.half_range)
    }
    pub fn get_high(&self) -> f64 {
        bt_normalize_angle(self.center + self.half_range)
    }
    pub fn is_limit(&self) -> bool {
        self.solve_limit
    }
}
