//! The vector math the animation code runs on, ported so the float
//! operations happen in the same order as the original:
//! `org.lwjgl.util.vector.{Matrix4f, Quaternion, Vector3f}`,
//! `zombie.iso.Vector2`, `zombie.core.math.PZMath` and
//! `zombie.core.skinnedmodel.HelperFunctions`.
//!
//! [`Matrix4f`] keeps LWJGL's sixteen named fields. The original reads
//! them as `m{row}{col}` of a column-vector matrix (translation in
//! `m03, m13, m23`, `HelperFunctions.ApplyBlendBone` computes
//! `x = m00*x + m01*y + m02*z + m03`), while `Matrix4f.mul(a, b)` is
//! LWJGL's product, which in that reading is `b * a`. Conversions to the
//! column-major [`pz_model::Mat4`] (`mat[col][row]`) are
//! [`Matrix4f::to_mat4`] and [`Matrix4f::from_mat4`].

use pz_model::Mat4;

pub const PI_F: f32 = std::f64::consts::PI as f32;

/// `org.lwjgl.util.vector.Matrix4f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix4f {
    pub m00: f32,
    pub m01: f32,
    pub m02: f32,
    pub m03: f32,
    pub m10: f32,
    pub m11: f32,
    pub m12: f32,
    pub m13: f32,
    pub m20: f32,
    pub m21: f32,
    pub m22: f32,
    pub m23: f32,
    pub m30: f32,
    pub m31: f32,
    pub m32: f32,
    pub m33: f32,
}

impl Default for Matrix4f {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix4f {
    pub const IDENTITY: Matrix4f = Matrix4f {
        m00: 1.0,
        m01: 0.0,
        m02: 0.0,
        m03: 0.0,
        m10: 0.0,
        m11: 1.0,
        m12: 0.0,
        m13: 0.0,
        m20: 0.0,
        m21: 0.0,
        m22: 1.0,
        m23: 0.0,
        m30: 0.0,
        m31: 0.0,
        m32: 0.0,
        m33: 1.0,
    };

    pub fn set_identity(&mut self) {
        *self = Self::IDENTITY;
    }

    /// The fields in `Matrix4f.store` order (`m00, m01, m02, m03, m10, ...`).
    pub fn store(&self) -> [f32; 16] {
        [
            self.m00, self.m01, self.m02, self.m03, self.m10, self.m11, self.m12, self.m13, self.m20, self.m21, self.m22,
            self.m23, self.m30, self.m31, self.m32, self.m33,
        ]
    }

    /// Inverse of [`Matrix4f::store`].
    pub fn load(v: &[f32; 16]) -> Matrix4f {
        Matrix4f {
            m00: v[0],
            m01: v[1],
            m02: v[2],
            m03: v[3],
            m10: v[4],
            m11: v[5],
            m12: v[6],
            m13: v[7],
            m20: v[8],
            m21: v[9],
            m22: v[10],
            m23: v[11],
            m30: v[12],
            m31: v[13],
            m32: v[14],
            m33: v[15],
        }
    }

    /// To pz-render's column-major, column-vector convention: `mat[col][row]`
    /// is the original's `m{row}{col}`.
    pub fn to_mat4(&self) -> Mat4 {
        [
            [self.m00, self.m10, self.m20, self.m30],
            [self.m01, self.m11, self.m21, self.m31],
            [self.m02, self.m12, self.m22, self.m32],
            [self.m03, self.m13, self.m23, self.m33],
        ]
    }

    /// Inverse of [`Matrix4f::to_mat4`].
    pub fn from_mat4(m: &Mat4) -> Matrix4f {
        Matrix4f {
            m00: m[0][0],
            m10: m[0][1],
            m20: m[0][2],
            m30: m[0][3],
            m01: m[1][0],
            m11: m[1][1],
            m21: m[1][2],
            m31: m[1][3],
            m02: m[2][0],
            m12: m[2][1],
            m22: m[2][2],
            m32: m[2][3],
            m03: m[3][0],
            m13: m[3][1],
            m23: m[3][2],
            m33: m[3][3],
        }
    }

    /// `Matrix4f.mul(left, right, dest)`.
    pub fn mul(left: &Matrix4f, right: &Matrix4f) -> Matrix4f {
        Matrix4f {
            m00: left.m00 * right.m00 + left.m10 * right.m01 + left.m20 * right.m02 + left.m30 * right.m03,
            m01: left.m01 * right.m00 + left.m11 * right.m01 + left.m21 * right.m02 + left.m31 * right.m03,
            m02: left.m02 * right.m00 + left.m12 * right.m01 + left.m22 * right.m02 + left.m32 * right.m03,
            m03: left.m03 * right.m00 + left.m13 * right.m01 + left.m23 * right.m02 + left.m33 * right.m03,
            m10: left.m00 * right.m10 + left.m10 * right.m11 + left.m20 * right.m12 + left.m30 * right.m13,
            m11: left.m01 * right.m10 + left.m11 * right.m11 + left.m21 * right.m12 + left.m31 * right.m13,
            m12: left.m02 * right.m10 + left.m12 * right.m11 + left.m22 * right.m12 + left.m32 * right.m13,
            m13: left.m03 * right.m10 + left.m13 * right.m11 + left.m23 * right.m12 + left.m33 * right.m13,
            m20: left.m00 * right.m20 + left.m10 * right.m21 + left.m20 * right.m22 + left.m30 * right.m23,
            m21: left.m01 * right.m20 + left.m11 * right.m21 + left.m21 * right.m22 + left.m31 * right.m23,
            m22: left.m02 * right.m20 + left.m12 * right.m21 + left.m22 * right.m22 + left.m32 * right.m23,
            m23: left.m03 * right.m20 + left.m13 * right.m21 + left.m23 * right.m22 + left.m33 * right.m23,
            m30: left.m00 * right.m30 + left.m10 * right.m31 + left.m20 * right.m32 + left.m30 * right.m33,
            m31: left.m01 * right.m30 + left.m11 * right.m31 + left.m21 * right.m32 + left.m31 * right.m33,
            m32: left.m02 * right.m30 + left.m12 * right.m31 + left.m22 * right.m32 + left.m32 * right.m33,
            m33: left.m03 * right.m30 + left.m13 * right.m31 + left.m23 * right.m32 + left.m33 * right.m33,
        }
    }

    pub fn transpose(&self) -> Matrix4f {
        Matrix4f {
            m00: self.m00,
            m01: self.m10,
            m02: self.m20,
            m03: self.m30,
            m10: self.m01,
            m11: self.m11,
            m12: self.m21,
            m13: self.m31,
            m20: self.m02,
            m21: self.m12,
            m22: self.m22,
            m23: self.m32,
            m30: self.m03,
            m31: self.m13,
            m32: self.m23,
            m33: self.m33,
        }
    }

    pub fn determinant(&self) -> f32 {
        let s = self;
        let mut f = s.m00
            * (s.m11 * s.m22 * s.m33 + s.m12 * s.m23 * s.m31 + s.m13 * s.m21 * s.m32
                - s.m13 * s.m22 * s.m31
                - s.m11 * s.m23 * s.m32
                - s.m12 * s.m21 * s.m33);
        f -= s.m01
            * (s.m10 * s.m22 * s.m33 + s.m12 * s.m23 * s.m30 + s.m13 * s.m20 * s.m32
                - s.m13 * s.m22 * s.m30
                - s.m10 * s.m23 * s.m32
                - s.m12 * s.m20 * s.m33);
        f += s.m02
            * (s.m10 * s.m21 * s.m33 + s.m11 * s.m23 * s.m30 + s.m13 * s.m20 * s.m31
                - s.m13 * s.m21 * s.m30
                - s.m10 * s.m23 * s.m31
                - s.m11 * s.m20 * s.m33);
        f -= s.m03
            * (s.m10 * s.m21 * s.m32 + s.m11 * s.m22 * s.m30 + s.m12 * s.m20 * s.m31
                - s.m12 * s.m21 * s.m30
                - s.m10 * s.m22 * s.m31
                - s.m11 * s.m20 * s.m32);
        f
    }

    /// `Matrix4f.invert(src, dest)`; `None` where the original returns null.
    pub fn invert(&self) -> Option<Matrix4f> {
        let det = self.determinant();
        if det == 0.0 {
            return None;
        }
        let s = self;
        let d3 = |t00: f32, t01: f32, t02: f32, t10: f32, t11: f32, t12: f32, t20: f32, t21: f32, t22: f32| {
            t00 * (t11 * t22 - t12 * t21) + t01 * (t12 * t20 - t10 * t22) + t02 * (t10 * t21 - t11 * t20)
        };
        let inv = 1.0 / det;
        let t00 = d3(s.m11, s.m12, s.m13, s.m21, s.m22, s.m23, s.m31, s.m32, s.m33);
        let t01 = -d3(s.m10, s.m12, s.m13, s.m20, s.m22, s.m23, s.m30, s.m32, s.m33);
        let t02 = d3(s.m10, s.m11, s.m13, s.m20, s.m21, s.m23, s.m30, s.m31, s.m33);
        let t03 = -d3(s.m10, s.m11, s.m12, s.m20, s.m21, s.m22, s.m30, s.m31, s.m32);
        let t10 = -d3(s.m01, s.m02, s.m03, s.m21, s.m22, s.m23, s.m31, s.m32, s.m33);
        let t11 = d3(s.m00, s.m02, s.m03, s.m20, s.m22, s.m23, s.m30, s.m32, s.m33);
        let t12 = -d3(s.m00, s.m01, s.m03, s.m20, s.m21, s.m23, s.m30, s.m31, s.m33);
        let t13 = d3(s.m00, s.m01, s.m02, s.m20, s.m21, s.m22, s.m30, s.m31, s.m32);
        let t20 = d3(s.m01, s.m02, s.m03, s.m11, s.m12, s.m13, s.m31, s.m32, s.m33);
        let t21 = -d3(s.m00, s.m02, s.m03, s.m10, s.m12, s.m13, s.m30, s.m32, s.m33);
        let t22 = d3(s.m00, s.m01, s.m03, s.m10, s.m11, s.m13, s.m30, s.m31, s.m33);
        let t23 = -d3(s.m00, s.m01, s.m02, s.m10, s.m11, s.m12, s.m30, s.m31, s.m32);
        let t30 = -d3(s.m01, s.m02, s.m03, s.m11, s.m12, s.m13, s.m21, s.m22, s.m23);
        let t31 = d3(s.m00, s.m02, s.m03, s.m10, s.m12, s.m13, s.m20, s.m22, s.m23);
        let t32 = -d3(s.m00, s.m01, s.m03, s.m10, s.m11, s.m13, s.m20, s.m21, s.m23);
        let t33 = d3(s.m00, s.m01, s.m02, s.m10, s.m11, s.m12, s.m20, s.m21, s.m22);
        Some(Matrix4f {
            m00: t00 * inv,
            m11: t11 * inv,
            m22: t22 * inv,
            m33: t33 * inv,
            m01: t10 * inv,
            m10: t01 * inv,
            m20: t02 * inv,
            m02: t20 * inv,
            m12: t21 * inv,
            m21: t12 * inv,
            m03: t30 * inv,
            m30: t03 * inv,
            m13: t31 * inv,
            m31: t13 * inv,
            m32: t23 * inv,
            m23: t32 * inv,
        })
    }

    /// `Matrix4f.scale(vec)` (in place).
    pub fn scale(&mut self, v: Vector3f) {
        self.m00 *= v.x;
        self.m01 *= v.x;
        self.m02 *= v.x;
        self.m03 *= v.x;
        self.m10 *= v.y;
        self.m11 *= v.y;
        self.m12 *= v.y;
        self.m13 *= v.y;
        self.m20 *= v.z;
        self.m21 *= v.z;
        self.m22 *= v.z;
        self.m23 *= v.z;
    }

    /// `Matrix4f.rotate(angle, axis)` (in place).
    pub fn rotate(&mut self, angle: f32, axis: Vector3f) {
        let c = (angle as f64).cos() as f32;
        let s = (angle as f64).sin() as f32;
        let oneminusc = 1.0 - c;
        let xy = axis.x * axis.y;
        let yz = axis.y * axis.z;
        let xz = axis.x * axis.z;
        let xs = axis.x * s;
        let ys = axis.y * s;
        let zs = axis.z * s;
        let f00 = axis.x * axis.x * oneminusc + c;
        let f01 = xy * oneminusc + zs;
        let f02 = xz * oneminusc - ys;
        let f10 = xy * oneminusc - zs;
        let f11 = axis.y * axis.y * oneminusc + c;
        let f12 = yz * oneminusc + xs;
        let f20 = xz * oneminusc + ys;
        let f21 = yz * oneminusc - xs;
        let f22 = axis.z * axis.z * oneminusc + c;
        let src = *self;
        let t00 = src.m00 * f00 + src.m10 * f01 + src.m20 * f02;
        let t01 = src.m01 * f00 + src.m11 * f01 + src.m21 * f02;
        let t02 = src.m02 * f00 + src.m12 * f01 + src.m22 * f02;
        let t03 = src.m03 * f00 + src.m13 * f01 + src.m23 * f02;
        let t10 = src.m00 * f10 + src.m10 * f11 + src.m20 * f12;
        let t11 = src.m01 * f10 + src.m11 * f11 + src.m21 * f12;
        let t12 = src.m02 * f10 + src.m12 * f11 + src.m22 * f12;
        let t13 = src.m03 * f10 + src.m13 * f11 + src.m23 * f12;
        self.m20 = src.m00 * f20 + src.m10 * f21 + src.m20 * f22;
        self.m21 = src.m01 * f20 + src.m11 * f21 + src.m21 * f22;
        self.m22 = src.m02 * f20 + src.m12 * f21 + src.m22 * f22;
        self.m23 = src.m03 * f20 + src.m13 * f21 + src.m23 * f22;
        self.m00 = t00;
        self.m01 = t01;
        self.m02 = t02;
        self.m03 = t03;
        self.m10 = t10;
        self.m11 = t11;
        self.m12 = t12;
        self.m13 = t13;
    }
}

/// `org.lwjgl.util.vector.Quaternion`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quaternion {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quaternion {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quaternion {
    pub const IDENTITY: Quaternion = Quaternion { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Quaternion { x, y, z, w }
    }

    pub fn from_model(q: pz_model::Quat) -> Self {
        Quaternion::new(q.x, q.y, q.z, q.w)
    }

    pub fn length_squared(&self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w
    }

    pub fn length(&self) -> f32 {
        self.length_squared().sqrt()
    }

    /// `Quaternion.normalise()` (in place).
    pub fn normalise(&mut self) {
        let inv_l = 1.0 / self.length();
        *self = Quaternion::new(self.x * inv_l, self.y * inv_l, self.z * inv_l, self.w * inv_l);
    }

    pub fn scale(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
        self.z *= s;
        self.w *= s;
    }

    /// `Quaternion.setFromMatrix(Matrix4f)`.
    pub fn from_matrix(m: &Matrix4f) -> Quaternion {
        let (m00, m01, m02, m10, m11, m12, m20, m21, m22) = (m.m00, m.m01, m.m02, m.m10, m.m11, m.m12, m.m20, m.m21, m.m22);
        let mut q = Quaternion::IDENTITY;
        let tr = m00 + m11 + m22;
        if tr as f64 >= 0.0 {
            let mut s = ((tr as f64) + 1.0).sqrt() as f32;
            q.w = s * 0.5;
            s = 0.5 / s;
            q.x = (m21 - m12) * s;
            q.y = (m02 - m20) * s;
            q.z = (m10 - m01) * s;
        } else {
            let max = m00.max(m11).max(m22);
            if max == m00 {
                let mut s = (((m00 - (m11 + m22)) as f64) + 1.0).sqrt() as f32;
                q.x = s * 0.5;
                s = 0.5 / s;
                q.y = (m01 + m10) * s;
                q.z = (m20 + m02) * s;
                q.w = (m21 - m12) * s;
            } else if max == m11 {
                let mut s = (((m11 - (m22 + m00)) as f64) + 1.0).sqrt() as f32;
                q.y = s * 0.5;
                s = 0.5 / s;
                q.z = (m12 + m21) * s;
                q.x = (m01 + m10) * s;
                q.w = (m02 - m20) * s;
            } else {
                let mut s = (((m22 - (m00 + m11)) as f64) + 1.0).sqrt() as f32;
                q.z = s * 0.5;
                s = 0.5 / s;
                q.x = (m20 + m02) * s;
                q.y = (m12 + m21) * s;
                q.w = (m10 - m01) * s;
            }
        }
        q
    }
}

/// `org.lwjgl.util.vector.Vector3f`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vector3f {
    pub const ZERO: Vector3f = Vector3f { x: 0.0, y: 0.0, z: 0.0 };
    pub const ONE: Vector3f = Vector3f { x: 1.0, y: 1.0, z: 1.0 };

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Vector3f { x, y, z }
    }

    pub fn from_array(v: [f32; 3]) -> Self {
        Vector3f::new(v[0], v[1], v[2])
    }

    pub fn length_squared(&self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    /// `Vector.normalise()`; the original throws on a zero vector, this
    /// leaves it unchanged.
    pub fn normalise(&mut self) {
        let len = self.length_squared().sqrt();
        if len != 0.0 {
            let l = 1.0 / len;
            self.x *= l;
            self.y *= l;
            self.z *= l;
        }
    }

    pub fn dot(a: Vector3f, b: Vector3f) -> f32 {
        a.x * b.x + a.y * b.y + a.z * b.z
    }

    pub fn cross(l: Vector3f, r: Vector3f) -> Vector3f {
        Vector3f::new(l.y * r.z - l.z * r.y, r.x * l.z - r.z * l.x, l.x * r.y - l.y * r.x)
    }
}

/// `zombie.iso.Vector2`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

impl Vector2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Vector2 { x, y }
    }

    pub fn length_squared(&self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    pub fn length(&self) -> f32 {
        ((self.x * self.x + self.y * self.y) as f64).sqrt() as f32
    }

    /// `Vector2.normalize()`: returns the previous length.
    pub fn normalize(&mut self) -> f32 {
        let length_sq = self.length_squared();
        if pzmath::equal(length_sq, 1.0, 1.0e-5) {
            return 1.0;
        }
        if length_sq == 0.0 {
            self.x = 0.0;
            self.y = 0.0;
            return 0.0;
        }
        let length = (length_sq as f64).sqrt() as f32;
        self.x /= length;
        self.y /= length;
        length
    }

    pub fn rotate(&mut self, radians: f32) {
        let r = radians as f64;
        let rx = self.x as f64 * r.cos() - self.y as f64 * r.sin();
        let ry = self.x as f64 * r.sin() + self.y as f64 * r.cos();
        self.x = rx as f32;
        self.y = ry as f32;
    }

    pub fn scale(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
    }

    pub fn set_length(&mut self, length: f32) {
        self.normalize();
        self.x *= length;
        self.y *= length;
    }

    /// `Vector2.getDirection(x, y)`.
    pub fn direction(x: f32, y: f32) -> f32 {
        let angle = (y as f64).atan2(x as f64) as f32;
        pzmath::wrap_range(angle, -PI_F, PI_F)
    }

    pub fn add_scaled(a: Vector2, b: Vector2, scale: f32) -> Vector2 {
        Vector2::new(a.x + b.x * scale, a.y + b.y * scale)
    }
}

/// `zombie.core.math.PZMath`.
pub mod pzmath {
    use super::{Quaternion, Vector3f, PI_F};

    pub fn lerp(src: f32, dest: f32, alpha: f32) -> f32 {
        src + (dest - src) * alpha
    }

    pub fn lerp_v3(a: Vector3f, b: Vector3f, t: f32) -> Vector3f {
        Vector3f::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t)
    }

    pub fn slerp(from: Quaternion, to: Quaternion, alpha: f32) -> Quaternion {
        let dot = (from.x * to.x + from.y * to.y + from.z * to.z + from.w * to.w) as f64;
        let abs_dot = if dot < 0.0 { -dot } else { dot };
        let mut scale0 = (1.0f32 - alpha) as f64;
        let mut scale1 = alpha as f64;
        if 1.0 - abs_dot > 0.1 {
            let angle = abs_dot.acos();
            let sin_angle = angle.sin();
            let inv_sin_angle = 1.0 / sin_angle;
            scale0 = (angle * (1.0 - alpha as f64)).sin() * inv_sin_angle;
            scale1 = (angle * alpha as f64).sin() * inv_sin_angle;
        }
        if dot < 0.0 {
            scale1 = -scale1;
        }
        Quaternion::new(
            (scale0 * from.x as f64 + scale1 * to.x as f64) as f32,
            (scale0 * from.y as f64 + scale1 * to.y as f64) as f32,
            (scale0 * from.z as f64 + scale1 * to.z as f64) as f32,
            (scale0 * from.w as f64 + scale1 * to.w as f64) as f32,
        )
    }

    pub fn sign(val: f32) -> i32 {
        if val > 0.0 {
            1
        } else if val < 0.0 {
            -1
        } else {
            0
        }
    }

    pub fn abs(val: f32) -> f32 {
        val * sign(val) as f32
    }

    pub fn equal(a: f32, b: f32, delta: f32) -> bool {
        abs(b - a) < delta
    }

    pub fn clamp(val: f32, min: f32, max: f32) -> f32 {
        if val < min {
            min
        } else if val > max {
            max
        } else {
            val
        }
    }

    pub fn clamp_i(val: i32, min: i32, max: i32) -> i32 {
        if val < min {
            min
        } else if val > max {
            max
        } else {
            val
        }
    }

    pub fn min(a: f32, b: f32) -> f32 {
        if a > b {
            b
        } else {
            a
        }
    }

    pub fn max(a: f32, b: f32) -> f32 {
        if a > b {
            a
        } else {
            b
        }
    }

    pub fn fastfloor(val: f32) -> i32 {
        let xi = val as i32;
        if val < xi as f32 {
            xi - 1
        } else {
            xi
        }
    }

    pub fn floor(val: f32) -> f32 {
        fastfloor(val) as f32
    }

    pub fn frac(val: f32) -> f32 {
        val - floor(val)
    }

    pub fn wrap(val: f32, range: f32) -> f32 {
        if range <= 0.0 {
            return 0.0;
        }
        if val < 0.0 {
            let multiple_negative = -val / range;
            let frac_up = 1.0 - frac(multiple_negative);
            return frac_up * range;
        }
        let multiple = val / range;
        frac(multiple) * range
    }

    pub fn wrap_range(val: f32, min_v: f32, max_v: f32) -> f32 {
        let real_max = max(max_v, min_v);
        let real_min = min(max_v, min_v);
        let range = real_max - real_min;
        let rel_val = val - real_min;
        real_min + wrap(rel_val, range)
    }

    pub fn get_closest_angle(rads_a: f32, rads_b: f32) -> f32 {
        let angle_a = wrap(rads_a, PI_F * 2.0);
        let angle_b = wrap(rads_b, PI_F * 2.0);
        let diff = angle_b - angle_a;
        wrap_range(diff, -PI_F, PI_F)
    }

    pub fn lerp_angle(src: f32, dest: f32, alpha: f32) -> f32 {
        let diff = get_closest_angle(src, dest);
        let lerped = src + alpha * diff;
        wrap_range(lerped, -PI_F, PI_F)
    }

    pub fn deg_to_rad(degrees: f32) -> f32 {
        PI_F / 180.0 * degrees
    }

    pub fn lerp_func_ease_out_quad(x: f32) -> f32 {
        x * x
    }

    pub fn lerp_func_ease_in_quad(x: f32) -> f32 {
        let rev_x = 1.0 - x;
        1.0 - rev_x * rev_x
    }

    pub fn lerp_func_ease_out_in_quad(x: f32) -> f32 {
        if x < 0.5 {
            return lerp_func_ease_out_quad(x) * 2.0;
        }
        0.5 + lerp_func_ease_in_quad(2.0 * x - 1.0) / 2.0
    }
}

/// `zombie.core.skinnedmodel.HelperFunctions`.
pub mod helpers {
    use super::{Matrix4f, Quaternion, Vector3f};

    /// `CreateFromQuaternion(q, result)`. Like the original this normalises
    /// `q` in place when it is not unit length.
    pub fn create_from_quaternion(q: &mut Quaternion) -> Matrix4f {
        let q_len_sq = q.length_squared();
        if q_len_sq > 0.0 && q_len_sq < 0.99999 || q_len_sq > 1.00001 {
            let q_len = (q_len_sq as f64).sqrt() as f32;
            let inv = 1.0 / q_len;
            q.scale(inv);
        }
        let xx = q.x * q.x;
        let xy = q.x * q.y;
        let xz = q.x * q.z;
        let wx = q.x * q.w;
        let yy = q.y * q.y;
        let yz = q.y * q.z;
        let wy = q.y * q.w;
        let zz = q.z * q.z;
        let wz = q.z * q.w;
        let r = Matrix4f {
            m00: 1.0 - 2.0 * (yy + zz),
            m10: 2.0 * (xy - wz),
            m20: 2.0 * (xz + wy),
            m30: 0.0,
            m01: 2.0 * (xy + wz),
            m11: 1.0 - 2.0 * (xx + zz),
            m21: 2.0 * (yz - wx) * 1.0,
            m31: 0.0,
            m02: 2.0 * (xz - wy),
            m12: 2.0 * (yz + wx),
            m22: 1.0 - 2.0 * (xx + yy),
            m32: 0.0,
            m03: 0.0,
            m13: 0.0,
            m23: 0.0,
            m33: 1.0,
        };
        r.transpose()
    }

    /// `CreateFromQuaternionPositionScale`; normalises `rot` in place like
    /// the original.
    pub fn create_from_qps(pos: Vector3f, rot: &mut Quaternion, scale: Vector3f) -> Matrix4f {
        let mut scl = Matrix4f::IDENTITY;
        scl.scale(scale);
        let rotm = create_from_quaternion(rot);
        let mut result = Matrix4f::mul(&scl, &rotm);
        set_position(&mut result, pos);
        result
    }

    pub fn get_position(m: &Matrix4f) -> Vector3f {
        Vector3f::new(m.m03, m.m13, m.m23)
    }

    pub fn set_position(m: &mut Matrix4f, p: Vector3f) {
        m.m03 = p.x;
        m.m13 = p.y;
        m.m23 = p.z;
    }

    pub fn get_rotation(m: &Matrix4f) -> Quaternion {
        Quaternion::from_matrix(m)
    }

    /// `transform(Quaternion, Vector3f)`; normalises the quaternion in place.
    pub fn transform(rotation: &mut Quaternion, v: Vector3f) -> Vector3f {
        rotation.normalise();
        let s = rotation.w;
        let (vx, vy, vz) = (rotation.x, rotation.y, rotation.z);
        let s2 = s * s;
        let v2 = vx * vx + vy * vy + vz * vz;
        let (px, py, pz) = (v.x, v.y, v.z);
        let vcpx = vy * pz - vz * py;
        let vcpy = vz * px - vx * pz;
        let vcpz = vx * py - vy * px;
        let vdp = px * vx + py * vy + pz * vz;
        Vector3f::new(
            (s2 - v2) * px + 2.0 * s * vcpx + 2.0 * vx * vdp,
            (s2 - v2) * py + 2.0 * s * vcpy + 2.0 * vy * vdp,
            (s2 - v2) * pz + 2.0 * s * vcpz + 2.0 * vz * vdp,
        )
    }

    pub fn transform_vector(m: &Matrix4f, v: Vector3f) -> Vector3f {
        Vector3f::new(
            m.m00 * v.x + m.m01 * v.y + m.m02 * v.z,
            m.m10 * v.x + m.m11 * v.y + m.m12 * v.z,
            m.m20 * v.x + m.m21 * v.y + m.m22 * v.z,
        )
    }

    pub fn get_rotation_y_matrix(m: &Matrix4f) -> f32 {
        let rxaxis_x = m.m00 * 1.0 + m.m01 * 0.0 + m.m02 * 0.0;
        let rxaxis_z = m.m20 * 1.0 + m.m21 * 0.0 + m.m22 * 0.0;
        let rzaxis_x = m.m00 * 0.0 + m.m01 * 0.0 + m.m02 * 1.0;
        let rzaxis_z = m.m20 * 0.0 + m.m21 * 0.0 + m.m22 * 1.0;
        let c = (rxaxis_x + rzaxis_z) / 2.0;
        let s = (rxaxis_z - rzaxis_x) / 2.0;
        let length = ((c * c + s * s) as f64).sqrt() as f32;
        let norm_c = c / length;
        let norm_s = s / length;
        ((-norm_s) as f64).atan2(norm_c as f64) as f32
    }

    /// `getRotationY(Quaternion)`; normalises the quaternion in place.
    pub fn get_rotation_y(rotation: &mut Quaternion) -> f32 {
        rotation.normalise();
        let qs = rotation.w;
        let (vx, vy, vz) = (rotation.x, rotation.y, rotation.z);
        let s2 = qs * qs;
        let v2 = vx * vx + vy * vy + vz * vz;
        let vcpx = vy * 0.0 - vz * 0.0;
        let vcpz = vx * 0.0 - vy * 1.0;
        let vdp = 1.0 * vx + 0.0 * vy + 0.0 * vz;
        let rxaxis_x = (s2 - v2) * 1.0 + 2.0 * qs * vcpx + 2.0 * vx * vdp;
        let rxaxis_z = (s2 - v2) * 0.0 + 2.0 * qs * vcpz + 2.0 * vz * vdp;
        let vcpx2 = vy * 1.0 - vz * 0.0;
        let vcpz2 = vx * 0.0 - vy * 0.0;
        let vdp2 = 0.0 * vx + 0.0 * vy + 1.0 * vz;
        let rzaxis_x = (s2 - v2) * 0.0 + 2.0 * qs * vcpx2 + 2.0 * vx * vdp2;
        let rzaxis_z = (s2 - v2) * 1.0 + 2.0 * qs * vcpz2 + 2.0 * vz * vdp2;
        let c = (rxaxis_x + rzaxis_z) / 2.0;
        let s = (rxaxis_z - rzaxis_x) / 2.0;
        let length = ((c * c + s * s) as f64).sqrt() as f32;
        let norm_c = c / length;
        let norm_s = s / length;
        ((-norm_s) as f64).atan2(norm_c as f64) as f32
    }

    pub fn get_rotation_z(rotation: &Quaternion) -> f32 {
        let s = rotation.w;
        let (vx, vy, vz) = (rotation.x, rotation.y, rotation.z);
        let s2 = s * s;
        let v2 = vx * vx + vy * vy + vz * vz;
        let vcpy = vz * 1.0;
        let vdp = 1.0 * vx;
        let rx = (s2 - v2) * 1.0 + 2.0 * vx * vdp;
        let ry = 2.0 * s * vcpy + 2.0 * vy * vdp;
        (ry as f64).atan2(rx as f64) as f32
    }

    pub fn to_euler_angles(rot: &Quaternion) -> Vector3f {
        let sinr_cosp = 2.0 * (rot.w * rot.x + rot.y * rot.z) as f64;
        let cosr_cosp = 1.0 - 2.0 * (rot.x * rot.x + rot.y * rot.y) as f64;
        let x = sinr_cosp.atan2(cosr_cosp) as f32;
        let sinp = 2.0 * (rot.w * rot.y - rot.z * rot.x) as f64;
        let y = if sinp.abs() >= 1.0 {
            (1.570_796_370_506_286_6f64).copysign(sinp) as f32
        } else {
            sinp.asin() as f32
        };
        let siny_cosp = 2.0 * (rot.w * rot.z + rot.x * rot.y) as f64;
        let cosy_cosp = 1.0 - 2.0 * (rot.y * rot.y + rot.z * rot.z) as f64;
        let z = siny_cosp.atan2(cosy_cosp) as f32;
        Vector3f::new(x, y, z)
    }

    pub fn to_quaternion(roll: f64, pitch: f64, yaw: f64) -> Quaternion {
        let cy = (yaw * 0.5).cos();
        let sy = (yaw * 0.5).sin();
        let cp = (pitch * 0.5).cos();
        let sp = (pitch * 0.5).sin();
        let cr = (roll * 0.5).cos();
        let sr = (roll * 0.5).sin();
        Quaternion::new(
            (cy * cp * sr - sy * sp * cr) as f32,
            (sy * cp * sr + cy * sp * cr) as f32,
            (sy * cp * cr - cy * sp * sr) as f32,
            (cy * cp * cr + sy * sp * sr) as f32,
        )
    }

    pub fn set_from_axis_angle(ax: f32, ay: f32, az: f32, angle: f32) -> Quaternion {
        let n = ((ax * ax + ay * ay + az * az) as f64).sqrt() as f32;
        let s = ((0.5 * angle as f64).sin() / n as f64) as f32;
        Quaternion::new(ax * s, ay * s, az * s, (0.5 * angle as f64).cos() as f32)
    }
}
