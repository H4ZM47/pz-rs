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
// Altered for pz-rs: Rust translation of btPoint2PointConstraint (Bullet 2.82,
// double precision).

//! `btPoint2PointConstraint`.

use crate::bt::collision_object::ObjId;
use crate::bt::linear_math::*;

use super::typed_constraint::*;

pub const BT_P2P_FLAGS_ERP: i32 = 1;
pub const BT_P2P_FLAGS_CFM: i32 = 2;

/// `btConstraintSetting`.
#[derive(Clone, Copy, Debug)]
pub struct ConstraintSetting {
    pub tau: f64,
    pub damping: f64,
    pub impulse_clamp: f64,
}

impl Default for ConstraintSetting {
    fn default() -> Self {
        ConstraintSetting {
            tau: 0.3,
            damping: 1.0,
            impulse_clamp: 0.0,
        }
    }
}

/// `btPoint2PointConstraint`.
#[derive(Clone, Debug)]
pub struct Point2PointConstraint {
    pub pivot_in_a: Vec3,
    pub pivot_in_b: Vec3,
    pub flags: i32,
    pub erp: f64,
    pub cfm: f64,
    pub use_solve_constraint_obsolete: bool,
    pub setting: ConstraintSetting,
}

impl Point2PointConstraint {
    /// `btPoint2PointConstraint(rbA, rbB, pivotInA, pivotInB)`.
    pub fn new_typed(
        rb_a: ObjId,
        rb_b: ObjId,
        pivot_in_a: Vec3,
        pivot_in_b: Vec3,
    ) -> TypedConstraint {
        let c = Point2PointConstraint {
            pivot_in_a,
            pivot_in_b,
            flags: 0,
            erp: 0.0,
            cfm: 0.0,
            use_solve_constraint_obsolete: false,
            setting: ConstraintSetting::default(),
        };
        TypedConstraint::new(
            POINT2POINT_CONSTRAINT_TYPE,
            Some(rb_a),
            Some(rb_b),
            ConstraintKind::Point2Point(c),
        )
    }

    /// `getInfo1`.
    pub fn get_info1(&self, info: &mut ConstraintInfo1) {
        if self.use_solve_constraint_obsolete {
            info.num_constraint_rows = 0;
            info.nub = 0;
        } else {
            info.num_constraint_rows = 3;
            info.nub = 3;
        }
    }

    /// `getInfo2NonVirtual(info, body0_trans, body1_trans)`.
    pub fn get_info2(
        &self,
        info: &mut ConstraintInfo2,
        body0_trans: &Transform,
        body1_trans: &Transform,
    ) {
        let r = &mut *info.rows;
        r[0].contact_normal1[0] = 1.0;
        r[1].contact_normal1[1] = 1.0;
        r[2].contact_normal1[2] = 1.0;

        let a1 = body0_trans.basis * self.pivot_in_a;
        {
            let a1neg = -a1;
            let (mut v0, mut v1, mut v2) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
            a1neg.get_skew_symmetric_matrix(&mut v0, &mut v1, &mut v2);
            r[0].relpos1_cross_normal = v0;
            r[1].relpos1_cross_normal = v1;
            r[2].relpos1_cross_normal = v2;
        }
        r[0].contact_normal2[0] = -1.0;
        r[1].contact_normal2[1] = -1.0;
        r[2].contact_normal2[2] = -1.0;

        let a2 = body1_trans.basis * self.pivot_in_b;
        {
            let (mut v0, mut v1, mut v2) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
            a2.get_skew_symmetric_matrix(&mut v0, &mut v1, &mut v2);
            r[0].relpos2_cross_normal = v0;
            r[1].relpos2_cross_normal = v1;
            r[2].relpos2_cross_normal = v2;
        }

        let curr_erp = if self.flags & BT_P2P_FLAGS_ERP != 0 {
            self.erp
        } else {
            info.erp
        };
        let k = info.fps * curr_erp;
        for j in 0..3 {
            r[j].rhs = k * (a2[j] + body1_trans.origin[j] - a1[j] - body0_trans.origin[j]);
        }
        if self.flags & BT_P2P_FLAGS_CFM != 0 {
            for row in r.iter_mut().take(3) {
                row.cfm = self.cfm;
            }
        }
        let impulse_clamp = self.setting.impulse_clamp;
        for row in r.iter_mut().take(3) {
            if self.setting.impulse_clamp > 0.0 {
                row.lower_limit = -impulse_clamp;
                row.upper_limit = impulse_clamp;
            }
        }
        info.damping = self.setting.damping;
    }

    /// `setParam`.
    pub fn set_param(&mut self, num: i32, value: f64, axis: i32) {
        if axis == -1 {
            match num {
                BT_CONSTRAINT_ERP | BT_CONSTRAINT_STOP_ERP => {
                    self.erp = value;
                    self.flags |= BT_P2P_FLAGS_ERP;
                }
                BT_CONSTRAINT_CFM | BT_CONSTRAINT_STOP_CFM => {
                    self.cfm = value;
                    self.flags |= BT_P2P_FLAGS_CFM;
                }
                _ => {}
            }
        }
    }

    /// `getParam`.
    pub fn get_param(&self, num: i32, axis: i32) -> f64 {
        let mut ret = SIMD_INFINITY;
        if axis == -1 {
            match num {
                BT_CONSTRAINT_ERP | BT_CONSTRAINT_STOP_ERP => ret = self.erp,
                BT_CONSTRAINT_CFM | BT_CONSTRAINT_STOP_CFM => ret = self.cfm,
                _ => {}
            }
        }
        ret
    }
}
