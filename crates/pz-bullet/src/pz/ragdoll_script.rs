//! `PZRagdollScript`: the ragdoll definitions Java uploads through the
//! `defineRagdoll*` natives (constraints, anchors, body-part shapes and body
//! dynamics), reconstructed from the shipped library (PZRagdollScript.cpp).
//!
//! Every table starts with value-initialised records (all keys 0), and the
//! getters return the first record whose key matches, as the original does.

use crate::bt::linear_math::*;

/// `btTypedConstraintType` values the ragdoll uses.
pub const HINGE_CONSTRAINT_TYPE: i32 = 4;
pub const CONETWIST_CONSTRAINT_TYPE: i32 = 5;

/// `RagdollConstraint` (0xd0 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RagdollConstraint {
    pub joint: i32,
    pub constraint_type: i32,
    pub part_a: i32,
    pub part_b: i32,
    /// Euler angles (x, y, z) of the frame in body A.
    pub axis_a: Vec3,
    pub axis_b: Vec3,
    pub position_offset_a: Vec3,
    pub position_offset_b: Vec3,
    /// Cone-twist: swing span 1, swing span 2, twist span. Hinge: low, high.
    pub limit: Vec3,
    pub limit_extended: Vec3,
}

/// `RagdollAnchor` (12 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RagdollAnchor {
    pub bone: i32,
    pub body_part: i32,
    pub reverse: bool,
    pub original: bool,
    pub enabled: bool,
}

/// `BodyPartInfo` (0x40 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyPartInfo {
    pub part: i32,
    pub calculate_length: bool,
    pub radius: f32,
    pub height: f32,
    pub gap: f32,
    /// 0 capsule, 1 box, 2 sphere.
    pub shape: i32,
    pub mass: f64,
    pub offset: Vec3,
}

/// `RagdollBodyDynamics` (32 bytes, floats as Java sends them).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RagdollBodyDynamics {
    pub part: i32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub deactivation_time: f32,
    pub linear_sleeping_threshold: f32,
    pub angular_sleeping_threshold: f32,
    pub friction: f32,
    pub rolling_friction: f32,
}

/// `PZRagdollScript`.
#[derive(Clone, Debug)]
pub struct RagdollScript {
    /// `+0x00..+0x18`: 70.0, 1.5 and 0.5 (unused by the ragdoll code).
    pub defaults: [f64; 3],
    /// `+0x18`: 10 joints.
    pub constraints: Vec<RagdollConstraint>,
    /// `+0x30`: 35 bones.
    pub anchors: Vec<RagdollAnchor>,
    /// `+0x48`: 11 body parts.
    pub body_parts: Vec<BodyPartInfo>,
    /// `+0x60`: 11 body parts.
    pub body_dynamics: Vec<RagdollBodyDynamics>,
}

impl Default for RagdollScript {
    fn default() -> Self {
        RagdollScript::new()
    }
}

/// `(int)(f + (f >= 0 ? 0.5 : -0.5))` in double, as the parsers round.
pub fn round_index(f: f32) -> i32 {
    let h = if 0.0 <= f { 0.5 } else { -0.5 };
    (h + f as f64) as i32
}

fn vec3(p: &[f32]) -> Vec3 {
    Vec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
}

impl RagdollScript {
    /// `PZRagdollScript()`.
    pub fn new() -> RagdollScript {
        RagdollScript {
            defaults: [70.0, 1.5, 0.5],
            constraints: vec![RagdollConstraint::default(); 10],
            anchors: vec![RagdollAnchor::default(); 35],
            body_parts: vec![BodyPartInfo::default(); 11],
            body_dynamics: vec![RagdollBodyDynamics::default(); 11],
        }
    }

    /// `constraintsFromJava(params, n)`: records of 22 floats, stored at the
    /// index of their joint. Returns `true` once 10 records were read.
    pub fn constraints_from_java(&mut self, p: &[f32]) -> bool {
        let mut i = 0usize;
        for k in 0..10 {
            if i + 22 > p.len() {
                return k > 9;
            }
            let r = &p[i..i + 22];
            let c = RagdollConstraint {
                joint: round_index(r[0]),
                constraint_type: round_index(r[1]),
                part_a: round_index(r[2]),
                part_b: round_index(r[3]),
                axis_a: vec3(&r[4..]),
                axis_b: vec3(&r[7..]),
                position_offset_a: vec3(&r[10..]),
                position_offset_b: vec3(&r[13..]),
                limit: vec3(&r[16..]),
                limit_extended: vec3(&r[19..]),
            };
            // (The original writes out of bounds for other joints.)
            if let Some(slot) = self.constraints.get_mut(c.joint as usize) {
                *slot = c;
            }
            i += 22;
        }
        true
    }

    /// `anchorsFromJava(params, n)`: `SkeletonBone::Count()` records of 5
    /// floats, stored at the index of their bone.
    pub fn anchors_from_java(&mut self, p: &[f32]) -> bool {
        let mut i = 0usize;
        for _ in 0..35 {
            if i + 5 > p.len() {
                return false;
            }
            let r = &p[i..i + 5];
            let a = RagdollAnchor {
                bone: round_index(r[0]),
                body_part: round_index(r[1]),
                reverse: 0.0 < r[2],
                original: 0.0 < r[3],
                enabled: 0.0 < r[4],
            };
            if let Some(slot) = self.anchors.get_mut(a.bone as usize) {
                *slot = a;
            }
            i += 5;
        }
        true
    }

    /// `bodyPartInfoFromJava(params, n)`: records of 10 floats, stored at the
    /// index of their body part.
    pub fn body_part_info_from_java(&mut self, p: &[f32]) -> bool {
        let mut i = 0usize;
        for k in 0..11 {
            if i + 10 > p.len() {
                return k > 10;
            }
            let r = &p[i..i + 10];
            let b = BodyPartInfo {
                part: round_index(r[0]),
                calculate_length: 0.0 < r[1],
                radius: r[2],
                height: r[3],
                gap: r[4],
                shape: round_index(r[5]),
                mass: r[6] as f64,
                offset: vec3(&r[7..]),
            };
            if let Some(slot) = self.body_parts.get_mut(b.part as usize) {
                *slot = b;
            }
            i += 10;
        }
        true
    }

    /// One `RagdollBodyDynamics` record from 8 floats.
    fn dynamics(r: &[f32]) -> RagdollBodyDynamics {
        RagdollBodyDynamics {
            part: round_index(r[0]),
            linear_damping: r[1],
            angular_damping: r[2],
            deactivation_time: r[3],
            linear_sleeping_threshold: r[4],
            angular_sleeping_threshold: r[5],
            friction: r[6],
            rolling_friction: r[7],
        }
    }

    /// `bodyDynamicsFromJava(params, n)`: records of 8 floats, stored at the
    /// index of their body part.
    pub fn body_dynamics_from_java(&mut self, p: &[f32]) -> bool {
        let mut i = 0usize;
        for k in 0..11 {
            if i + 8 > p.len() {
                return k > 10;
            }
            let d = Self::dynamics(&p[i..i + 8]);
            if let Some(slot) = self.body_dynamics.get_mut(d.part as usize) {
                *slot = d;
            }
            i += 8;
        }
        true
    }

    /// `setRagdollBodyDynamics(out, params, n)`: reads records of 8 floats
    /// into one `RagdollBodyDynamics` (the last complete one wins; fields
    /// of a cut-off record are written as far as they go). `out` starts as
    /// the caller's (uninitialised) value.
    pub fn set_ragdoll_body_dynamics(out: &mut RagdollBodyDynamics, p: &[f32]) {
        let mut i = 0usize;
        for _ in 0..11 {
            if i >= p.len() {
                return;
            }
            out.part = round_index(p[i]);
            let fields: [&mut f32; 7] = [
                &mut out.linear_damping,
                &mut out.angular_damping,
                &mut out.deactivation_time,
                &mut out.linear_sleeping_threshold,
                &mut out.angular_sleeping_threshold,
                &mut out.friction,
                &mut out.rolling_friction,
            ];
            for (k, f) in fields.into_iter().enumerate() {
                if i + 1 + k >= p.len() {
                    return;
                }
                *f = p[i + 1 + k];
            }
            i += 8;
        }
    }

    /// `getRagdollConstraint(joint)`.
    pub fn get_ragdoll_constraint(&self, joint: i32) -> Option<&RagdollConstraint> {
        self.constraints.iter().find(|c| c.joint == joint)
    }

    /// `getRagdollAnchor(bone)`.
    pub fn get_ragdoll_anchor(&self, bone: i32) -> Option<&RagdollAnchor> {
        self.anchors.iter().find(|a| a.bone == bone)
    }

    /// `getBodyPartInfo(part)`.
    pub fn get_body_part_info(&self, part: i32) -> Option<&BodyPartInfo> {
        self.body_parts.iter().find(|b| b.part == part)
    }

    /// `getRagdollBodyDynamics(part)`.
    pub fn get_ragdoll_body_dynamics(&self, part: i32) -> Option<&RagdollBodyDynamics> {
        self.body_dynamics.iter().find(|d| d.part == part)
    }
}
