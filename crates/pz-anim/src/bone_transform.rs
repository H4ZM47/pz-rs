//! `BoneTransform`, `TwistableBoneTransform` and `AnimatorsBoneTransform`:
//! a bone's local transform kept either as position/rotation/scale or as a
//! matrix, each derived from the other on demand.

use crate::math::{helpers, Matrix4f, Quaternion, Vector3f};

/// `BoneTransform`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneTransform {
    matrix_valid: bool,
    matrix: Matrix4f,
    prs_valid: bool,
    pos: Vector3f,
    rot: Quaternion,
    scale: Vector3f,
}

impl Default for BoneTransform {
    fn default() -> Self {
        BoneTransform {
            matrix_valid: true,
            matrix: Matrix4f::IDENTITY,
            prs_valid: true,
            pos: Vector3f::ZERO,
            rot: Quaternion::IDENTITY,
            scale: Vector3f::ONE,
        }
    }
}

impl BoneTransform {
    pub fn set_identity(&mut self) {
        *self = BoneTransform::default();
    }

    /// `set(Vector3f, Quaternion, Vector3f)`.
    pub fn set_prs(&mut self, pos: Vector3f, rot: Quaternion, scale: Vector3f) {
        if !self.matrix_valid && self.prs_valid && self.pos == pos && self.rot == rot && self.scale == scale {
            return;
        }
        self.matrix_valid = false;
        self.prs_valid = true;
        self.pos = pos;
        self.rot = rot;
        self.scale = scale;
    }

    /// `set(Matrix4f)`.
    pub fn set_matrix(&mut self, m: &Matrix4f) {
        self.matrix_valid = true;
        self.matrix = *m;
        self.prs_valid = false;
    }

    /// `mul(a, b)`: this = `Matrix4f.mul(a, b)`.
    pub fn mul_into(&mut self, a: &Matrix4f, b: &Matrix4f) {
        self.matrix_valid = true;
        self.prs_valid = false;
        self.matrix = Matrix4f::mul(a, b);
    }

    /// `getMatrix`.
    pub fn matrix(&mut self) -> Matrix4f {
        self.validate_matrix();
        self.matrix
    }

    /// `getPRS`.
    pub fn prs(&mut self) -> (Vector3f, Quaternion, Vector3f) {
        self.validate_prs();
        (self.pos, self.rot, self.scale)
    }

    pub fn set_position(&mut self, p: Vector3f) {
        self.validate_prs();
        self.pos = p;
    }

    pub fn position(&mut self) -> Vector3f {
        self.validate_prs();
        self.pos
    }

    pub fn rotation(&mut self) -> Quaternion {
        self.validate_prs();
        self.rot
    }

    fn validate_matrix(&mut self) {
        if self.matrix_valid {
            return;
        }
        self.matrix_valid = true;
        self.matrix = helpers::create_from_qps(self.pos, &mut self.rot, self.scale);
    }

    fn validate_prs(&mut self) {
        if self.prs_valid {
            return;
        }
        self.prs_valid = true;
        self.pos = helpers::get_position(&self.matrix);
        self.rot = helpers::get_rotation(&self.matrix);
        self.scale = Vector3f::ONE;
    }
}

/// `AnimatorsBoneTransform` (a `TwistableBoneTransform` that also keeps the
/// previous frame's transform).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatorsBoneTransform {
    pub transform: BoneTransform,
    /// `TwistableBoneTransform.blendWeight`.
    pub blend_weight: f32,
    /// `TwistableBoneTransform.twist`.
    pub twist: f32,
    pub time_delta: f32,
    pub previous: BoneTransform,
    pub previous_blend_weight: f32,
    pub previous_twist: f32,
}

impl Default for AnimatorsBoneTransform {
    fn default() -> Self {
        AnimatorsBoneTransform {
            transform: BoneTransform::default(),
            blend_weight: 0.0,
            twist: 0.0,
            time_delta: -1.0,
            previous: BoneTransform::default(),
            previous_blend_weight: 0.0,
            previous_twist: 0.0,
        }
    }
}

impl AnimatorsBoneTransform {
    pub fn reset(&mut self) {
        *self = AnimatorsBoneTransform::default();
    }

    /// `setIdentity` (only the transform; twist values are kept).
    pub fn set_identity(&mut self) {
        self.transform.set_identity();
    }

    /// `nextFrame`.
    pub fn next_frame(&mut self, time_delta: f32) {
        self.time_delta = time_delta;
        self.previous = self.transform;
        self.previous_blend_weight = self.blend_weight;
        self.previous_twist = self.twist;
    }
}
