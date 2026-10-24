// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2009 Erwin Coumans  http://bulletphysics.org
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
// Altered for pz-rs: Rust translation of LinearMath (btScalar, btMinMax,
// btVector3, btQuadWord, btQuaternion, btMatrix3x3, btTransform,
// btTransformUtil, btAabbUtil2) from Bullet 2.82, double precision, scalar
// (non-SIMD) code paths.

//! `LinearMath` in double precision.
//!
//! Every function reproduces the arm64 build bit for bit. The game's library
//! was compiled by clang with `-ffp-contract=on`: inside one C++ expression,
//! `a * b + c` and `a * b - c` (and `c - a * b`) become a single fused
//! multiply-add, operands checked left first (`a*b + c*d` is
//! `fma(a, b, c*d)`); separate statements and inline-function boundaries
//! never fuse. The functions here spell out each fused operation with
//! [`fma`]. `btVector3` keeps its fourth (`w`) component because `setMax`,
//! `setMin` and `operator==` read it.

#![allow(clippy::too_many_arguments)]

use std::ops::{Add, AddAssign, Div, Index, IndexMut, Mul, MulAssign, Neg, Sub, SubAssign};

pub type Scalar = f64;

/// `DBL_EPSILON`.
pub const SIMD_EPSILON: f64 = f64::EPSILON;
/// `DBL_MAX`.
pub const SIMD_INFINITY: f64 = f64::MAX;
pub const BT_LARGE_FLOAT: f64 = 1e30;
pub const SIMD_PI: f64 = std::f64::consts::PI;
pub const SIMD_2_PI: f64 = 2.0 * SIMD_PI;
pub const SIMD_HALF_PI: f64 = SIMD_PI * 0.5;
pub const SIMDSQRT12: f64 = 0.707_106_781_186_547_6;

/// Fused multiply-add `a * b + c` with a single rounding (`fmadd`).
#[inline(always)]
pub fn fma(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

/// Converts a C `float` literal (`0.1f`) to the double the binary uses.
#[inline(always)]
pub const fn flt(x: f32) -> f64 {
    x as f64
}

// libm calls. The binary calls these functions from the platform libm; going
// through `extern "C"` keeps LLVM from merging a separate `sin`/`cos` pair into
// `__sincos_stret` where the original code did not.
mod libm {
    extern "C" {
        pub fn sin(x: f64) -> f64;
        pub fn cos(x: f64) -> f64;
        pub fn acos(x: f64) -> f64;
        pub fn asin(x: f64) -> f64;
        pub fn atan2(y: f64, x: f64) -> f64;
        pub fn fmod(x: f64, y: f64) -> f64;
        pub fn pow(x: f64, y: f64) -> f64;
    }
}

#[inline]
pub fn bt_sin(x: f64) -> f64 {
    // SAFETY: plain libm call.
    unsafe { libm::sin(x) }
}
#[inline]
pub fn bt_cos(x: f64) -> f64 {
    // SAFETY: plain libm call.
    unsafe { libm::cos(x) }
}
/// `sin(x)` and `cos(x)` of one argument, as clang emits them when both are
/// used (`__sincos_stret` on Apple platforms).
#[inline]
pub fn bt_sincos(x: f64) -> (f64, f64) {
    #[cfg(target_vendor = "apple")]
    {
        #[repr(C)]
        struct SinCos {
            s: f64,
            c: f64,
        }
        extern "C" {
            fn __sincos_stret(x: f64) -> SinCos;
        }
        // SAFETY: Apple libm entry point, returns {sin, cos} in d0/d1.
        let r = unsafe { __sincos_stret(x) };
        (r.s, r.c)
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        (bt_sin(x), bt_cos(x))
    }
}
/// `btAcos`: clamps to [-1, 1] first.
#[inline]
pub fn bt_acos(mut x: f64) -> f64 {
    if x < -1.0 {
        x = -1.0;
    }
    if x > 1.0 {
        x = 1.0;
    }
    // SAFETY: plain libm call.
    unsafe { libm::acos(x) }
}
/// `btAsin`: clamps to [-1, 1] first.
#[inline]
pub fn bt_asin(mut x: f64) -> f64 {
    if x < -1.0 {
        x = -1.0;
    }
    if x > 1.0 {
        x = 1.0;
    }
    // SAFETY: plain libm call.
    unsafe { libm::asin(x) }
}
#[inline]
pub fn bt_atan2(y: f64, x: f64) -> f64 {
    // SAFETY: plain libm call.
    unsafe { libm::atan2(y, x) }
}
#[inline]
pub fn bt_fmod(x: f64, y: f64) -> f64 {
    // SAFETY: plain libm call.
    unsafe { libm::fmod(x, y) }
}
#[inline]
pub fn bt_pow(x: f64, y: f64) -> f64 {
    // SAFETY: plain libm call.
    unsafe { libm::pow(x, y) }
}
#[inline(always)]
pub fn bt_sqrt(x: f64) -> f64 {
    x.sqrt()
}
#[inline(always)]
pub fn bt_fabs(x: f64) -> f64 {
    x.abs()
}
/// `btRecipSqrt(x)`: `1.0 / sqrt(x)`.
#[inline(always)]
pub fn bt_recip_sqrt(x: f64) -> f64 {
    1.0 / x.sqrt()
}
#[inline(always)]
pub fn bt_fuzzy_zero(x: f64) -> bool {
    x.abs() < SIMD_EPSILON
}
/// `btNormalizeAngle`: result in [-pi, pi].
#[inline]
pub fn bt_normalize_angle(angle: f64) -> f64 {
    let a = bt_fmod(angle, SIMD_2_PI);
    if a < -SIMD_PI {
        a + SIMD_2_PI
    } else if a > SIMD_PI {
        a - SIMD_2_PI
    } else {
        a
    }
}

/// `btMin` (`a < b ? a : b`).
#[inline(always)]
pub fn bt_min<T: PartialOrd + Copy>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}
/// `btMax` (`a > b ? a : b`).
#[inline(always)]
pub fn bt_max<T: PartialOrd + Copy>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}
/// `btSetMin`: `if (b < a) a = b`.
#[inline(always)]
pub fn bt_set_min<T: PartialOrd + Copy>(a: &mut T, b: T) {
    if b < *a {
        *a = b;
    }
}
/// `btSetMax`: `if (a < b) a = b`.
#[inline(always)]
pub fn bt_set_max<T: PartialOrd + Copy>(a: &mut T, b: T) {
    if *a < b {
        *a = b;
    }
}
/// `btClamp`.
#[inline(always)]
pub fn bt_clamp<T: PartialOrd + Copy>(a: &mut T, lb: T, ub: T) {
    if *a < lb {
        *a = lb;
    } else if ub < *a {
        *a = ub;
    }
}
/// `btClamped` (`a < lb ? lb : (ub < a ? ub : a)`).
#[inline(always)]
pub fn bt_clamped<T: PartialOrd + Copy>(a: T, lb: T, ub: T) -> T {
    if a < lb {
        lb
    } else if ub < a {
        ub
    } else {
        a
    }
}

// ---------------------------------------------------------------------------
// btVector3
// ---------------------------------------------------------------------------

/// `btVector3` (`m_floats[4]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };

    /// `btVector3(x, y, z)`: `w = 0`.
    #[inline(always)]
    pub const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z, w: 0.0 }
    }
    /// `btVector4(x, y, z, w)`.
    #[inline(always)]
    pub const fn new4(x: f64, y: f64, z: f64, w: f64) -> Vec3 {
        Vec3 { x, y, z, w }
    }
    #[inline(always)]
    pub const fn splat(s: f64) -> Vec3 {
        Vec3::new(s, s, s)
    }
    /// `setValue(x, y, z)`: also clears `w`.
    #[inline(always)]
    pub fn set_value(&mut self, x: f64, y: f64, z: f64) {
        *self = Vec3::new(x, y, z);
    }
    #[inline(always)]
    pub fn set_zero(&mut self) {
        *self = Vec3::ZERO;
    }
    #[inline(always)]
    pub fn get(&self, i: usize) -> f64 {
        self[i]
    }

    /// `dot`: `x*vx + y*vy + z*vz`, fused as `fma(z, vz, fma(x, vx, y*vy))`.
    #[inline(always)]
    pub fn dot(&self, v: &Vec3) -> f64 {
        fma(self.z, v.z, fma(self.x, v.x, self.y * v.y))
    }
    #[inline(always)]
    pub fn length2(&self) -> f64 {
        self.dot(self)
    }
    #[inline(always)]
    pub fn length(&self) -> f64 {
        self.length2().sqrt()
    }
    #[inline(always)]
    pub fn norm(&self) -> f64 {
        self.length()
    }
    /// `(v - *this).length2()`.
    #[inline(always)]
    pub fn distance2(&self, v: &Vec3) -> f64 {
        (*v - *self).length2()
    }
    #[inline(always)]
    pub fn distance(&self, v: &Vec3) -> f64 {
        (*v - *self).length()
    }
    /// `*this /= s`: multiplies by `1/s`, keeps `w` scaled too? No: Bullet's
    /// `operator*=` only touches x, y, z.
    #[inline(always)]
    pub fn div_assign_scalar(&mut self, s: f64) {
        *self *= 1.0 / s;
    }
    /// `safeNormalize`.
    pub fn safe_normalize(&mut self) -> &mut Vec3 {
        let abs_vec = self.absolute();
        let max_index = abs_vec.max_axis();
        if abs_vec[max_index] > 0.0 {
            self.div_assign_scalar(abs_vec[max_index]);
            let l = self.length();
            self.div_assign_scalar(l);
            return self;
        }
        self.set_value(1.0, 0.0, 0.0);
        self
    }
    /// `normalize`: `*this /= length()` (keeps `w`).
    #[inline(always)]
    pub fn normalize(&mut self) -> &mut Vec3 {
        let l = self.length();
        self.div_assign_scalar(l);
        self
    }
    #[inline(always)]
    pub fn normalized(&self) -> Vec3 {
        let mut n = *self;
        n.normalize();
        n
    }
    /// `rotate(wAxis, angle)`.
    pub fn rotate(&self, w_axis: &Vec3, angle: f64) -> Vec3 {
        let o = *w_axis * w_axis.dot(self);
        let x = *self - o;
        let y = w_axis.cross(self);
        o + x * bt_cos(angle) + y * bt_sin(angle)
    }
    /// `angle(v)`.
    pub fn angle(&self, v: &Vec3) -> f64 {
        let s = (self.length2() * v.length2()).sqrt();
        bt_acos(self.dot(v) / s)
    }
    #[inline(always)]
    pub fn absolute(&self) -> Vec3 {
        Vec3::new(self.x.abs(), self.y.abs(), self.z.abs())
    }
    /// `btVector4::absolute4`.
    #[inline(always)]
    pub fn absolute4(&self) -> Vec3 {
        Vec3::new4(self.x.abs(), self.y.abs(), self.z.abs(), self.w.abs())
    }
    /// `cross`: each component `a*b - c*d` fused as `fma(a, b, -(c*d))`.
    #[inline(always)]
    pub fn cross(&self, v: &Vec3) -> Vec3 {
        Vec3::new(
            fma(self.y, v.z, -(self.z * v.y)),
            fma(self.z, v.x, -(self.x * v.z)),
            fma(self.x, v.y, -(self.y * v.x)),
        )
    }
    /// `triple(v1, v2)`.
    #[inline(always)]
    pub fn triple(&self, v1: &Vec3, v2: &Vec3) -> f64 {
        let a = fma(v1.y, v2.z, -(v1.z * v2.y));
        let b = fma(v1.z, v2.x, -(v1.x * v2.z));
        let c = fma(v1.x, v2.y, -(v1.y * v2.x));
        fma(self.z, c, fma(self.x, a, self.y * b))
    }
    #[inline(always)]
    pub fn min_axis(&self) -> usize {
        if self.x < self.y {
            if self.x < self.z {
                0
            } else {
                2
            }
        } else if self.y < self.z {
            1
        } else {
            2
        }
    }
    #[inline(always)]
    pub fn max_axis(&self) -> usize {
        if self.x < self.y {
            if self.y < self.z {
                2
            } else {
                1
            }
        } else if self.x < self.z {
            2
        } else {
            0
        }
    }
    #[inline(always)]
    pub fn furthest_axis(&self) -> usize {
        self.absolute().min_axis()
    }
    #[inline(always)]
    pub fn closest_axis(&self) -> usize {
        self.absolute().max_axis()
    }
    /// `btVector4::maxAxis4` (-1 if every component is <= -BT_LARGE_FLOAT).
    pub fn max_axis4(&self) -> i32 {
        let mut max_index = -1;
        let mut max_val = -BT_LARGE_FLOAT;
        for i in 0..4 {
            if self[i] > max_val {
                max_index = i as i32;
                max_val = self[i];
            }
        }
        max_index
    }
    /// `btVector4::minAxis4`.
    pub fn min_axis4(&self) -> i32 {
        let mut min_index = -1;
        let mut min_val = BT_LARGE_FLOAT;
        for i in 0..4 {
            if self[i] < min_val {
                min_index = i as i32;
                min_val = self[i];
            }
        }
        min_index
    }
    /// `btVector4::closestAxis4`.
    pub fn closest_axis4(&self) -> i32 {
        self.absolute4().max_axis4()
    }
    /// `setInterpolate3(v0, v1, rt)`: `s*v0 + rt*v1` fused, keeps `w`.
    #[inline(always)]
    pub fn set_interpolate3(&mut self, v0: &Vec3, v1: &Vec3, rt: f64) {
        let s = 1.0 - rt;
        self.x = fma(s, v0.x, rt * v1.x);
        self.y = fma(s, v0.y, rt * v1.y);
        self.z = fma(s, v0.z, rt * v1.z);
    }
    /// `lerp(v, t)`: `x + (vx - x) * t` fused.
    #[inline(always)]
    pub fn lerp(&self, v: &Vec3, t: f64) -> Vec3 {
        Vec3::new(
            fma(v.x - self.x, t, self.x),
            fma(v.y - self.y, t, self.y),
            fma(v.z - self.z, t, self.z),
        )
    }
    /// `setMax`: componentwise including `w`.
    #[inline(always)]
    pub fn set_max(&mut self, o: &Vec3) {
        bt_set_max(&mut self.x, o.x);
        bt_set_max(&mut self.y, o.y);
        bt_set_max(&mut self.z, o.z);
        bt_set_max(&mut self.w, o.w);
    }
    /// `setMin`: componentwise including `w`.
    #[inline(always)]
    pub fn set_min(&mut self, o: &Vec3) {
        bt_set_min(&mut self.x, o.x);
        bt_set_min(&mut self.y, o.y);
        bt_set_min(&mut self.z, o.z);
        bt_set_min(&mut self.w, o.w);
    }
    #[inline(always)]
    pub fn is_zero(&self) -> bool {
        self.x == 0.0 && self.y == 0.0 && self.z == 0.0
    }
    #[inline(always)]
    pub fn fuzzy_zero(&self) -> bool {
        self.length2() < SIMD_EPSILON
    }
    /// `dot3(v0, v1, v2)`.
    #[inline(always)]
    pub fn dot3(&self, v0: &Vec3, v1: &Vec3, v2: &Vec3) -> Vec3 {
        Vec3::new(self.dot(v0), self.dot(v1), self.dot(v2))
    }
    /// `maxDot` over a point array: index of the first maximum (`-1` if none).
    pub fn max_dot(&self, array: &[Vec3], dot_out: &mut f64) -> i64 {
        let mut max_dot = -SIMD_INFINITY;
        let mut pt_index = -1i64;
        for (i, p) in array.iter().enumerate() {
            let d = p.dot(self);
            if d > max_dot {
                max_dot = d;
                pt_index = i as i64;
            }
        }
        *dot_out = max_dot;
        pt_index
    }
    /// `minDot` over a point array.
    pub fn min_dot(&self, array: &[Vec3], dot_out: &mut f64) -> i64 {
        let mut min_dot = SIMD_INFINITY;
        let mut pt_index = -1i64;
        for (i, p) in array.iter().enumerate() {
            let d = p.dot(self);
            if d < min_dot {
                min_dot = d;
                pt_index = i as i64;
            }
        }
        *dot_out = min_dot;
        pt_index
    }
    /// `getSkewSymmetricMatrix`.
    pub fn get_skew_symmetric_matrix(&self, v0: &mut Vec3, v1: &mut Vec3, v2: &mut Vec3) {
        v0.set_value(0.0, -self.z, self.y);
        v1.set_value(self.z, 0.0, -self.x);
        v2.set_value(-self.y, self.x, 0.0);
    }
    /// Componentwise `operator/(v1, v2)`.
    #[inline(always)]
    pub fn div_vec(&self, v: &Vec3) -> Vec3 {
        Vec3::new(self.x / v.x, self.y / v.y, self.z / v.z)
    }
    /// `operator*=(const btVector3&)` (keeps `w`).
    #[inline(always)]
    pub fn mul_assign_vec(&mut self, v: &Vec3) {
        self.x *= v.x;
        self.y *= v.y;
        self.z *= v.z;
    }
}

impl Index<usize> for Vec3 {
    type Output = f64;
    #[inline(always)]
    fn index(&self, i: usize) -> &f64 {
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            3 => &self.w,
            _ => panic!("Vec3 index {i}"),
        }
    }
}
impl IndexMut<usize> for Vec3 {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut f64 {
        match i {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            3 => &mut self.w,
            _ => panic!("Vec3 index {i}"),
        }
    }
}
/// `operator+(v1, v2)`: `w = 0`.
impl Add for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn add(self, v: Vec3) -> Vec3 {
        Vec3::new(self.x + v.x, self.y + v.y, self.z + v.z)
    }
}
/// `operator-(v1, v2)`: `w = 0`.
impl Sub for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn sub(self, v: Vec3) -> Vec3 {
        Vec3::new(self.x - v.x, self.y - v.y, self.z - v.z)
    }
}
/// Componentwise `operator*(v1, v2)`.
impl Mul for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn mul(self, v: Vec3) -> Vec3 {
        Vec3::new(self.x * v.x, self.y * v.y, self.z * v.z)
    }
}
impl Mul<f64> for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Mul<Vec3> for f64 {
    type Output = Vec3;
    #[inline(always)]
    fn mul(self, v: Vec3) -> Vec3 {
        v * self
    }
}
/// `operator/(v, s)`: `v * (1/s)`.
impl Div<f64> for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn div(self, s: f64) -> Vec3 {
        self * (1.0 / s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}
/// `operator+=` (keeps `w`).
impl AddAssign for Vec3 {
    #[inline(always)]
    fn add_assign(&mut self, v: Vec3) {
        self.x += v.x;
        self.y += v.y;
        self.z += v.z;
    }
}
/// `operator-=` (keeps `w`).
impl SubAssign for Vec3 {
    #[inline(always)]
    fn sub_assign(&mut self, v: Vec3) {
        self.x -= v.x;
        self.y -= v.y;
        self.z -= v.z;
    }
}
/// `operator*=(s)` (keeps `w`).
impl MulAssign<f64> for Vec3 {
    #[inline(always)]
    fn mul_assign(&mut self, s: f64) {
        self.x *= s;
        self.y *= s;
        self.z *= s;
    }
}

/// `btPlaneSpace1(n, p, q)`. Only x, y and z of `p` and `q` are written.
pub fn plane_space1(n: &Vec3, p: &mut Vec3, q: &mut Vec3) {
    if n.z.abs() > SIMDSQRT12 {
        let a = fma(n.y, n.y, n.z * n.z);
        let k = bt_recip_sqrt(a);
        p.x = 0.0;
        p.y = -n.z * k;
        p.z = n.y * k;
        q.x = a * k;
        q.y = -n.x * p.z;
        q.z = n.x * p.y;
    } else {
        let a = fma(n.x, n.x, n.y * n.y);
        let k = bt_recip_sqrt(a);
        p.x = -n.y * k;
        p.y = n.x * k;
        p.z = 0.0;
        q.x = -n.z * p.y;
        q.y = n.z * p.x;
        q.z = a * k;
    }
}

// ---------------------------------------------------------------------------
// btQuaternion
// ---------------------------------------------------------------------------

/// `btQuaternion`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Quat {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    #[inline(always)]
    pub const fn new(x: f64, y: f64, z: f64, w: f64) -> Quat {
        Quat { x, y, z, w }
    }
    /// `btQuaternion(axis, angle)`.
    pub fn from_axis_angle(axis: &Vec3, angle: f64) -> Quat {
        let mut q = Quat::IDENTITY;
        q.set_rotation(axis, angle);
        q
    }
    /// `setRotation(axis, angle)`: `sin(angle*0.5)/d` and `cos(angle*0.5)`
    /// share their argument, so clang calls `__sincos_stret` once.
    pub fn set_rotation(&mut self, axis: &Vec3, angle: f64) {
        let d = axis.length();
        let (sn, cs) = bt_sincos(angle * 0.5);
        let s = sn / d;
        *self = Quat::new(axis.x * s, axis.y * s, axis.z * s, cs);
    }
    /// `dot`: `((x*qx + y*qy) + z*qz) + w*qw` fused.
    #[inline(always)]
    pub fn dot(&self, q: &Quat) -> f64 {
        fma(
            self.w,
            q.w,
            fma(self.z, q.z, fma(self.x, q.x, self.y * q.y)),
        )
    }
    #[inline(always)]
    pub fn length2(&self) -> f64 {
        self.dot(self)
    }
    #[inline(always)]
    pub fn length(&self) -> f64 {
        self.length2().sqrt()
    }
    /// `*this /= length()`.
    #[inline(always)]
    pub fn normalize(&mut self) -> &mut Quat {
        let l = self.length();
        self.mul_assign_scalar(1.0 / l);
        self
    }
    #[inline(always)]
    pub fn normalized(&self) -> Quat {
        *self / self.length()
    }
    #[inline(always)]
    pub fn mul_assign_scalar(&mut self, s: f64) {
        self.x *= s;
        self.y *= s;
        self.z *= s;
        self.w *= s;
    }
    #[inline(always)]
    pub fn inverse(&self) -> Quat {
        Quat::new(-self.x, -self.y, -self.z, self.w)
    }
    /// `getAngle`: `2 * acos(w)`.
    #[inline(always)]
    pub fn get_angle(&self) -> f64 {
        2.0 * bt_acos(self.w)
    }
    /// `getAxis`.
    pub fn get_axis(&self) -> Vec3 {
        let s_squared = fma(-self.w, self.w, 1.0);
        if s_squared < 10.0 * SIMD_EPSILON {
            return Vec3::new(1.0, 0.0, 0.0);
        }
        let s = 1.0 / s_squared.sqrt();
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
    /// `angle(q)`.
    pub fn angle(&self, q: &Quat) -> f64 {
        let s = (self.length2() * q.length2()).sqrt();
        bt_acos(self.dot(q) / s)
    }
    /// `nearest(qd)`.
    pub fn nearest(&self, qd: &Quat) -> Quat {
        let diff = *self - *qd;
        let sum = *self + *qd;
        if diff.dot(&diff) < sum.dot(&sum) {
            *qd
        } else {
            -*qd
        }
    }
    /// `farthest(qd)`.
    pub fn farthest(&self, qd: &Quat) -> Quat {
        let diff = *self - *qd;
        let sum = *self + *qd;
        if diff.dot(&diff) > sum.dot(&sum) {
            *qd
        } else {
            -*qd
        }
    }
    #[inline(always)]
    pub fn set_value(&mut self, x: f64, y: f64, z: f64, w: f64) {
        *self = Quat::new(x, y, z, w);
    }
}

impl Index<usize> for Quat {
    type Output = f64;
    #[inline(always)]
    fn index(&self, i: usize) -> &f64 {
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            3 => &self.w,
            _ => panic!("Quat index {i}"),
        }
    }
}
impl IndexMut<usize> for Quat {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut f64 {
        match i {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            3 => &mut self.w,
            _ => panic!("Quat index {i}"),
        }
    }
}
impl Add for Quat {
    type Output = Quat;
    #[inline(always)]
    fn add(self, q: Quat) -> Quat {
        Quat::new(self.x + q.x, self.y + q.y, self.z + q.z, self.w + q.w)
    }
}
impl Sub for Quat {
    type Output = Quat;
    #[inline(always)]
    fn sub(self, q: Quat) -> Quat {
        Quat::new(self.x - q.x, self.y - q.y, self.z - q.z, self.w - q.w)
    }
}
impl Neg for Quat {
    type Output = Quat;
    #[inline(always)]
    fn neg(self) -> Quat {
        Quat::new(-self.x, -self.y, -self.z, -self.w)
    }
}
impl Mul<f64> for Quat {
    type Output = Quat;
    #[inline(always)]
    fn mul(self, s: f64) -> Quat {
        Quat::new(self.x * s, self.y * s, self.z * s, self.w * s)
    }
}
impl Div<f64> for Quat {
    type Output = Quat;
    #[inline(always)]
    fn div(self, s: f64) -> Quat {
        self * (1.0 / s)
    }
}
/// `operator*(q1, q2)` (same expressions as `operator*=`).
impl Mul for Quat {
    type Output = Quat;
    #[inline(always)]
    fn mul(self, q2: Quat) -> Quat {
        let q1 = self;
        Quat::new(
            fma(-q1.z, q2.y, fma(q1.y, q2.z, fma(q1.w, q2.x, q1.x * q2.w))),
            fma(-q1.x, q2.z, fma(q1.z, q2.x, fma(q1.w, q2.y, q1.y * q2.w))),
            fma(-q1.y, q2.x, fma(q1.x, q2.y, fma(q1.w, q2.z, q1.z * q2.w))),
            fma(
                -q1.z,
                q2.z,
                fma(-q1.y, q2.y, fma(q1.w, q2.w, -(q1.x * q2.x))),
            ),
        )
    }
}
/// `operator*(q, v)`.
impl Mul<Vec3> for Quat {
    type Output = Quat;
    #[inline(always)]
    fn mul(self, w: Vec3) -> Quat {
        let q = self;
        Quat::new(
            fma(-q.z, w.y, fma(q.w, w.x, q.y * w.z)),
            fma(-q.x, w.z, fma(q.w, w.y, q.z * w.x)),
            fma(-q.y, w.x, fma(q.w, w.z, q.x * w.y)),
            fma(-q.z, w.z, fma(-q.x, w.x, -(q.y * w.y))),
        )
    }
}
/// `operator*(v, q)`.
impl Mul<Quat> for Vec3 {
    type Output = Quat;
    #[inline(always)]
    fn mul(self, q: Quat) -> Quat {
        let w = self;
        Quat::new(
            fma(-w.z, q.y, fma(w.x, q.w, w.y * q.z)),
            fma(-w.x, q.z, fma(w.y, q.w, w.z * q.x)),
            fma(-w.y, q.x, fma(w.z, q.w, w.x * q.y)),
            fma(-w.z, q.z, fma(-w.x, q.x, -(w.y * q.y))),
        )
    }
}

/// `quatRotate(rotation, v)`.
#[inline]
pub fn quat_rotate(rotation: &Quat, v: &Vec3) -> Vec3 {
    let q = *rotation * *v;
    let q = q * rotation.inverse();
    Vec3::new(q.x, q.y, q.z)
}

/// `shortestArcQuat(v0, v1)`.
pub fn shortest_arc_quat(v0: &Vec3, v1: &Vec3) -> Quat {
    let c = v0.cross(v1);
    let d = v0.dot(v1);
    if d < -1.0 + SIMD_EPSILON {
        let mut n = Vec3::ZERO;
        let mut unused = Vec3::ZERO;
        plane_space1(v0, &mut n, &mut unused);
        return Quat::new(n.x, n.y, n.z, 0.0);
    }
    let s = ((1.0 + d) * 2.0).sqrt();
    let rs = 1.0 / s;
    Quat::new(c.x * rs, c.y * rs, c.z * rs, s * 0.5)
}

// ---------------------------------------------------------------------------
// btMatrix3x3
// ---------------------------------------------------------------------------

/// `btMatrix3x3` (three row vectors).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Mat3 {
    pub el: [Vec3; 3],
}

impl Index<usize> for Mat3 {
    type Output = Vec3;
    #[inline(always)]
    fn index(&self, i: usize) -> &Vec3 {
        &self.el[i]
    }
}
impl IndexMut<usize> for Mat3 {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut Vec3 {
        &mut self.el[i]
    }
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 {
        el: [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
    };
    pub const ZERO: Mat3 = Mat3 {
        el: [Vec3::ZERO; 3],
    };

    #[inline(always)]
    pub const fn new(
        xx: f64,
        xy: f64,
        xz: f64,
        yx: f64,
        yy: f64,
        yz: f64,
        zx: f64,
        zy: f64,
        zz: f64,
    ) -> Mat3 {
        Mat3 {
            el: [
                Vec3::new(xx, xy, xz),
                Vec3::new(yx, yy, yz),
                Vec3::new(zx, zy, zz),
            ],
        }
    }
    #[inline(always)]
    pub fn set_value(
        &mut self,
        xx: f64,
        xy: f64,
        xz: f64,
        yx: f64,
        yy: f64,
        yz: f64,
        zx: f64,
        zy: f64,
        zz: f64,
    ) {
        *self = Mat3::new(xx, xy, xz, yx, yy, yz, zx, zy, zz);
    }
    #[inline(always)]
    pub fn set_identity(&mut self) {
        *self = Mat3::IDENTITY;
    }
    /// `btMatrix3x3(q)`.
    pub fn from_quat(q: &Quat) -> Mat3 {
        let mut m = Mat3::IDENTITY;
        m.set_rotation(q);
        m
    }
    #[inline(always)]
    pub fn get_column(&self, i: usize) -> Vec3 {
        Vec3::new(self.el[0][i], self.el[1][i], self.el[2][i])
    }
    #[inline(always)]
    pub fn get_row(&self, i: usize) -> Vec3 {
        self.el[i]
    }
    /// `setRotation(q)`: separate statements, so no fusion except in
    /// `q.length2()`.
    pub fn set_rotation(&mut self, q: &Quat) {
        let d = q.length2();
        let s = 2.0 / d;
        let xs = q.x * s;
        let ys = q.y * s;
        let zs = q.z * s;
        let wx = q.w * xs;
        let wy = q.w * ys;
        let wz = q.w * zs;
        let xx = q.x * xs;
        let xy = q.x * ys;
        let xz = q.x * zs;
        let yy = q.y * ys;
        let yz = q.y * zs;
        let zz = q.z * zs;
        self.set_value(
            1.0 - (yy + zz),
            xy - wz,
            xz + wy,
            xy + wz,
            1.0 - (xx + zz),
            yz - wx,
            xz - wy,
            yz + wx,
            1.0 - (xx + yy),
        );
    }
    /// `setEulerZYX(eulerX, eulerY, eulerZ)`.
    pub fn set_euler_zyx(&mut self, euler_x: f64, euler_y: f64, euler_z: f64) {
        let (si, ci) = bt_sincos(euler_x);
        let (sj, cj) = bt_sincos(euler_y);
        let (sh, ch) = bt_sincos(euler_z);
        let cc = ci * ch;
        let cs = ci * sh;
        let sc = si * ch;
        let ss = si * sh;
        self.set_value(
            cj * ch,
            fma(sj, sc, -cs),
            fma(sj, cc, ss),
            cj * sh,
            fma(sj, ss, cc),
            fma(sj, cs, -sc),
            -sj,
            cj * si,
            cj * ci,
        );
    }
    /// `getRotation(q)`.
    pub fn get_rotation(&self) -> Quat {
        let m = &self.el;
        let trace = m[0].x + m[1].y + m[2].z;
        let mut temp = [0.0f64; 4];
        if trace > 0.0 {
            let mut s = (trace + 1.0).sqrt();
            temp[3] = s * 0.5;
            s = 0.5 / s;
            temp[0] = (m[2].y - m[1].z) * s;
            temp[1] = (m[0].z - m[2].x) * s;
            temp[2] = (m[1].x - m[0].y) * s;
        } else {
            let i = if m[0].x < m[1].y {
                if m[1].y < m[2].z {
                    2
                } else {
                    1
                }
            } else if m[0].x < m[2].z {
                2
            } else {
                0
            };
            let j = (i + 1) % 3;
            let k = (i + 2) % 3;
            let mut s = (m[i][i] - m[j][j] - m[k][k] + 1.0).sqrt();
            temp[i] = s * 0.5;
            s = 0.5 / s;
            temp[3] = (m[k][j] - m[j][k]) * s;
            temp[j] = (m[j][i] + m[i][j]) * s;
            temp[k] = (m[k][i] + m[i][k]) * s;
        }
        Quat::new(temp[0], temp[1], temp[2], temp[3])
    }
    /// `getEulerZYX(yaw, pitch, roll, 1)`.
    pub fn get_euler_zyx(&self) -> (f64, f64, f64) {
        let m = &self.el;
        if m[2].x.abs() >= 1.0 {
            let delta = bt_atan2(m[0].x, m[0].z);
            if m[2].x > 0.0 {
                let pitch = SIMD_PI / 2.0;
                (0.0, pitch, pitch + delta)
            } else {
                let pitch = -SIMD_PI / 2.0;
                (0.0, pitch, -pitch + delta)
            }
        } else {
            let pitch = -bt_asin(m[2].x);
            let c = bt_cos(pitch);
            let roll = bt_atan2(m[2].y / c, m[2].z / c);
            let yaw = bt_atan2(m[1].x / c, m[0].x / c);
            (yaw, pitch, roll)
        }
    }
    /// `scaled(s)`.
    pub fn scaled(&self, s: &Vec3) -> Mat3 {
        let m = &self.el;
        Mat3::new(
            m[0].x * s.x,
            m[0].y * s.y,
            m[0].z * s.z,
            m[1].x * s.x,
            m[1].y * s.y,
            m[1].z * s.z,
            m[2].x * s.x,
            m[2].y * s.y,
            m[2].z * s.z,
        )
    }
    /// `tdotx(v)`: `m00*vx + m10*vy + m20*vz` fused.
    #[inline(always)]
    pub fn tdotx(&self, v: &Vec3) -> f64 {
        fma(
            self.el[2].x,
            v.z,
            fma(self.el[0].x, v.x, self.el[1].x * v.y),
        )
    }
    #[inline(always)]
    pub fn tdoty(&self, v: &Vec3) -> f64 {
        fma(
            self.el[2].y,
            v.z,
            fma(self.el[0].y, v.x, self.el[1].y * v.y),
        )
    }
    #[inline(always)]
    pub fn tdotz(&self, v: &Vec3) -> f64 {
        fma(
            self.el[2].z,
            v.z,
            fma(self.el[0].z, v.x, self.el[1].z * v.y),
        )
    }
    #[inline(always)]
    pub fn determinant(&self) -> f64 {
        self.el[0].triple(&self.el[1], &self.el[2])
    }
    pub fn absolute(&self) -> Mat3 {
        let m = &self.el;
        Mat3::new(
            m[0].x.abs(),
            m[0].y.abs(),
            m[0].z.abs(),
            m[1].x.abs(),
            m[1].y.abs(),
            m[1].z.abs(),
            m[2].x.abs(),
            m[2].y.abs(),
            m[2].z.abs(),
        )
    }
    pub fn transpose(&self) -> Mat3 {
        let m = &self.el;
        Mat3::new(
            m[0].x, m[1].x, m[2].x, m[0].y, m[1].y, m[2].y, m[0].z, m[1].z, m[2].z,
        )
    }
    /// `cofac(r1, c1, r2, c2)`: `m[r1][c1]*m[r2][c2] - m[r1][c2]*m[r2][c1]` fused.
    #[inline(always)]
    pub fn cofac(&self, r1: usize, c1: usize, r2: usize, c2: usize) -> f64 {
        let m = &self.el;
        fma(m[r1][c1], m[r2][c2], -(m[r1][c2] * m[r2][c1]))
    }
    pub fn adjoint(&self) -> Mat3 {
        Mat3::new(
            self.cofac(1, 1, 2, 2),
            self.cofac(0, 2, 2, 1),
            self.cofac(0, 1, 1, 2),
            self.cofac(1, 2, 2, 0),
            self.cofac(0, 0, 2, 2),
            self.cofac(0, 2, 1, 0),
            self.cofac(1, 0, 2, 1),
            self.cofac(0, 1, 2, 0),
            self.cofac(0, 0, 1, 1),
        )
    }
    /// `inverse()`.
    pub fn inverse(&self) -> Mat3 {
        let co = Vec3::new(
            self.cofac(1, 1, 2, 2),
            self.cofac(1, 2, 2, 0),
            self.cofac(1, 0, 2, 1),
        );
        let det = self.el[0].dot(&co);
        let s = 1.0 / det;
        Mat3::new(
            co.x * s,
            self.cofac(0, 2, 2, 1) * s,
            self.cofac(0, 1, 1, 2) * s,
            co.y * s,
            self.cofac(0, 0, 2, 2) * s,
            self.cofac(0, 2, 1, 0) * s,
            co.z * s,
            self.cofac(0, 1, 2, 0) * s,
            self.cofac(0, 0, 1, 1) * s,
        )
    }
    /// `transposeTimes(m)`: `this^T * m`.
    pub fn transpose_times(&self, m: &Mat3) -> Mat3 {
        let a = &self.el;
        let b = &m.el;
        let e =
            |i: usize, j: usize| fma(a[2][i], b[2][j], fma(a[0][i], b[0][j], a[1][i] * b[1][j]));
        Mat3::new(
            e(0, 0),
            e(0, 1),
            e(0, 2),
            e(1, 0),
            e(1, 1),
            e(1, 2),
            e(2, 0),
            e(2, 1),
            e(2, 2),
        )
    }
    /// `timesTranspose(m)`: `this * m^T`.
    pub fn times_transpose(&self, m: &Mat3) -> Mat3 {
        let a = &self.el;
        let b = &m.el;
        Mat3::new(
            a[0].dot(&b[0]),
            a[0].dot(&b[1]),
            a[0].dot(&b[2]),
            a[1].dot(&b[0]),
            a[1].dot(&b[1]),
            a[1].dot(&b[2]),
            a[2].dot(&b[0]),
            a[2].dot(&b[1]),
            a[2].dot(&b[2]),
        )
    }
    /// `operator*=(m)`.
    #[inline(always)]
    pub fn mul_assign_mat(&mut self, m: &Mat3) {
        *self = *self * *m;
    }
    /// `diagonalize(rot, threshold, maxSteps)`.
    pub fn diagonalize(&mut self, rot: &mut Mat3, threshold: f64, max_steps: i32) {
        rot.set_identity();
        let mut step = max_steps;
        while step > 0 {
            let mut p = 0usize;
            let mut q = 1usize;
            let mut r = 2usize;
            let mut max = self.el[0][1].abs();
            let mut v = self.el[0][2].abs();
            if v > max {
                q = 2;
                r = 1;
                max = v;
            }
            v = self.el[1][2].abs();
            if v > max {
                p = 1;
                q = 2;
                r = 0;
                max = v;
            }
            let mut t =
                threshold * (self.el[0][0].abs() + self.el[1][1].abs() + self.el[2][2].abs());
            if max <= t {
                if max <= SIMD_EPSILON * t {
                    return;
                }
                step = 1;
            }
            let mpq = self.el[p][q];
            let theta = (self.el[q][q] - self.el[p][p]) / (2.0 * mpq);
            let theta2 = theta * theta;
            let cos;
            let sin;
            if theta2 * theta2 < 10.0 / SIMD_EPSILON {
                t = if theta >= 0.0 {
                    1.0 / (theta + (1.0 + theta2).sqrt())
                } else {
                    1.0 / (theta - (1.0 + theta2).sqrt())
                };
                cos = 1.0 / fma(t, t, 1.0).sqrt();
                sin = cos * t;
            } else {
                t = 1.0 / (theta * (2.0 + 0.5 / theta2));
                cos = fma(-(0.5 * t), t, 1.0);
                sin = cos * t;
            }
            self.el[p][q] = 0.0;
            self.el[q][p] = 0.0;
            self.el[p][p] = fma(-t, mpq, self.el[p][p]);
            self.el[q][q] = fma(t, mpq, self.el[q][q]);
            let mrp = self.el[r][p];
            let mrq = self.el[r][q];
            let a = fma(cos, mrp, -(sin * mrq));
            self.el[p][r] = a;
            self.el[r][p] = a;
            let b = fma(cos, mrq, sin * mrp);
            self.el[q][r] = b;
            self.el[r][q] = b;
            for i in 0..3 {
                let row = &mut rot.el[i];
                let mrp = row[p];
                let mrq = row[q];
                row[p] = fma(cos, mrp, -(sin * mrq));
                row[q] = fma(cos, mrq, sin * mrp);
            }
            step -= 1;
        }
    }
}

/// `operator*(m1, m2)`.
impl Mul for Mat3 {
    type Output = Mat3;
    #[inline(always)]
    fn mul(self, m2: Mat3) -> Mat3 {
        let m1 = &self.el;
        Mat3::new(
            m2.tdotx(&m1[0]),
            m2.tdoty(&m1[0]),
            m2.tdotz(&m1[0]),
            m2.tdotx(&m1[1]),
            m2.tdoty(&m1[1]),
            m2.tdotz(&m1[1]),
            m2.tdotx(&m1[2]),
            m2.tdoty(&m1[2]),
            m2.tdotz(&m1[2]),
        )
    }
}
/// `operator*(m, v)`: row dots.
impl Mul<Vec3> for Mat3 {
    type Output = Vec3;
    #[inline(always)]
    fn mul(self, v: Vec3) -> Vec3 {
        Vec3::new(self.el[0].dot(&v), self.el[1].dot(&v), self.el[2].dot(&v))
    }
}
/// `operator*(v, m)`: column dots.
impl Mul<Mat3> for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn mul(self, m: Mat3) -> Vec3 {
        Vec3::new(m.tdotx(&self), m.tdoty(&self), m.tdotz(&self))
    }
}
/// `operator*(m, k)`.
impl Mul<f64> for Mat3 {
    type Output = Mat3;
    #[inline(always)]
    fn mul(self, k: f64) -> Mat3 {
        let m = &self.el;
        Mat3::new(
            m[0].x * k,
            m[0].y * k,
            m[0].z * k,
            m[1].x * k,
            m[1].y * k,
            m[1].z * k,
            m[2].x * k,
            m[2].y * k,
            m[2].z * k,
        )
    }
}
impl Add for Mat3 {
    type Output = Mat3;
    fn add(self, o: Mat3) -> Mat3 {
        let a = &self.el;
        let b = &o.el;
        Mat3::new(
            a[0][0] + b[0][0],
            a[0][1] + b[0][1],
            a[0][2] + b[0][2],
            a[1][0] + b[1][0],
            a[1][1] + b[1][1],
            a[1][2] + b[1][2],
            a[2][0] + b[2][0],
            a[2][1] + b[2][1],
            a[2][2] + b[2][2],
        )
    }
}
impl Sub for Mat3 {
    type Output = Mat3;
    fn sub(self, o: Mat3) -> Mat3 {
        let a = &self.el;
        let b = &o.el;
        Mat3::new(
            a[0][0] - b[0][0],
            a[0][1] - b[0][1],
            a[0][2] - b[0][2],
            a[1][0] - b[1][0],
            a[1][1] - b[1][1],
            a[1][2] - b[1][2],
            a[2][0] - b[2][0],
            a[2][1] - b[2][1],
            a[2][2] - b[2][2],
        )
    }
}

// ---------------------------------------------------------------------------
// btTransform
// ---------------------------------------------------------------------------

/// `btTransform`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Transform {
    pub basis: Mat3,
    pub origin: Vec3,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        basis: Mat3::IDENTITY,
        origin: Vec3::ZERO,
    };

    #[inline(always)]
    pub const fn new(basis: Mat3, origin: Vec3) -> Transform {
        Transform { basis, origin }
    }
    /// `btTransform(q, c)`.
    pub fn from_quat(q: &Quat, c: Vec3) -> Transform {
        Transform {
            basis: Mat3::from_quat(q),
            origin: c,
        }
    }
    #[inline(always)]
    pub fn set_identity(&mut self) {
        *self = Transform::IDENTITY;
    }
    #[inline(always)]
    pub fn set_rotation(&mut self, q: &Quat) {
        self.basis.set_rotation(q);
    }
    #[inline(always)]
    pub fn get_rotation(&self) -> Quat {
        self.basis.get_rotation()
    }
    /// `operator()(x)`: `x.dot3(rows) + origin`.
    #[inline(always)]
    pub fn apply(&self, x: &Vec3) -> Vec3 {
        x.dot3(&self.basis.el[0], &self.basis.el[1], &self.basis.el[2]) + self.origin
    }
    /// `invXform(v)`: `basis^T * (v - origin)`.
    #[inline(always)]
    pub fn inv_xform(&self, v: &Vec3) -> Vec3 {
        let v = *v - self.origin;
        self.basis.transpose() * v
    }
    /// `inverse()`.
    #[inline(always)]
    pub fn inverse(&self) -> Transform {
        let inv = self.basis.transpose();
        Transform::new(inv, inv * -self.origin)
    }
    /// `inverseTimes(t)`.
    #[inline(always)]
    pub fn inverse_times(&self, t: &Transform) -> Transform {
        let v = t.origin - self.origin;
        Transform::new(self.basis.transpose_times(&t.basis), v * self.basis)
    }
    /// `mult(t1, t2)`.
    #[inline(always)]
    pub fn mult(&mut self, t1: &Transform, t2: &Transform) {
        self.basis = t1.basis * t2.basis;
        self.origin = t1.apply(&t2.origin);
    }
    /// `operator*=(t)`.
    #[inline(always)]
    pub fn mul_assign(&mut self, t: &Transform) {
        self.origin += self.basis * t.origin;
        self.basis = self.basis * t.basis;
    }
    /// `getOpenGLMatrix`.
    pub fn get_opengl_matrix(&self, m: &mut [f64; 16]) {
        let b = &self.basis.el;
        m[0] = b[0].x;
        m[1] = b[1].x;
        m[2] = b[2].x;
        m[3] = 0.0;
        m[4] = b[0].y;
        m[5] = b[1].y;
        m[6] = b[2].y;
        m[7] = 0.0;
        m[8] = b[0].z;
        m[9] = b[1].z;
        m[10] = b[2].z;
        m[11] = 0.0;
        m[12] = self.origin.x;
        m[13] = self.origin.y;
        m[14] = self.origin.z;
        m[15] = 1.0;
    }
}
/// `operator*(t)`.
impl Mul for Transform {
    type Output = Transform;
    #[inline(always)]
    fn mul(self, t: Transform) -> Transform {
        Transform::new(self.basis * t.basis, self.apply(&t.origin))
    }
}
impl Mul<Vec3> for Transform {
    type Output = Vec3;
    #[inline(always)]
    fn mul(self, v: Vec3) -> Vec3 {
        self.apply(&v)
    }
}

// ---------------------------------------------------------------------------
// btTransformUtil
// ---------------------------------------------------------------------------

pub const ANGULAR_MOTION_THRESHOLD: f64 = 0.5 * SIMD_HALF_PI;

/// `btAabbSupport(halfExtents, supportDir)`.
#[inline(always)]
pub fn aabb_support(half_extents: &Vec3, dir: &Vec3) -> Vec3 {
    Vec3::new(
        if dir.x < 0.0 {
            -half_extents.x
        } else {
            half_extents.x
        },
        if dir.y < 0.0 {
            -half_extents.y
        } else {
            half_extents.y
        },
        if dir.z < 0.0 {
            -half_extents.z
        } else {
            half_extents.z
        },
    )
}

/// `btTransformUtil`.
pub mod transform_util {
    use super::*;

    /// `integrateTransform`.
    pub fn integrate_transform(
        cur: &Transform,
        linvel: &Vec3,
        angvel: &Vec3,
        time_step: f64,
    ) -> Transform {
        let mut predicted = *cur;
        predicted.origin = cur.origin + *linvel * time_step;
        let mut f_angle = angvel.length();
        if f_angle * time_step > ANGULAR_MOTION_THRESHOLD {
            f_angle = ANGULAR_MOTION_THRESHOLD / time_step;
        }
        let axis = if f_angle < 0.001 {
            // 0.5*t - (t*t*t)*0.020833333333*fAngle*fAngle
            let tail = ((time_step * time_step * time_step) * 0.020833333333 * f_angle) * f_angle;
            *angvel * fma(0.5, time_step, -tail)
        } else {
            *angvel * (bt_sin(0.5 * f_angle * time_step) / f_angle)
        };
        let dorn = Quat::new(axis.x, axis.y, axis.z, bt_cos(f_angle * time_step * 0.5));
        let orn0 = cur.get_rotation();
        let mut predicted_orn = dorn * orn0;
        predicted_orn.normalize();
        predicted.set_rotation(&predicted_orn);
        predicted
    }

    /// `calculateDiffAxisAngleQuaternion`.
    pub fn calculate_diff_axis_angle_quaternion(orn0: &Quat, orn1a: &Quat) -> (Vec3, f64) {
        let orn1 = orn0.nearest(orn1a);
        let dorn = orn1 * orn0.inverse();
        let angle = dorn.get_angle();
        let mut axis = Vec3::new(dorn.x, dorn.y, dorn.z);
        axis.w = 0.0;
        let len = axis.length2();
        if len < SIMD_EPSILON * SIMD_EPSILON {
            axis = Vec3::new(1.0, 0.0, 0.0);
        } else {
            axis.div_assign_scalar(len.sqrt());
        }
        (axis, angle)
    }

    /// `calculateVelocityQuaternion`.
    pub fn calculate_velocity_quaternion(
        pos0: &Vec3,
        pos1: &Vec3,
        orn0: &Quat,
        orn1: &Quat,
        time_step: f64,
    ) -> (Vec3, Vec3) {
        let lin_vel = (*pos1 - *pos0) / time_step;
        let ang_vel = if orn0 != orn1 {
            let (axis, angle) = calculate_diff_axis_angle_quaternion(orn0, orn1);
            axis * angle / time_step
        } else {
            Vec3::ZERO
        };
        (lin_vel, ang_vel)
    }

    /// `calculateDiffAxisAngle`.
    pub fn calculate_diff_axis_angle(
        transform0: &Transform,
        transform1: &Transform,
    ) -> (Vec3, f64) {
        let dmat = transform1.basis * transform0.basis.inverse();
        let mut dorn = dmat.get_rotation();
        dorn.normalize();
        let angle = dorn.get_angle();
        let mut axis = Vec3::new(dorn.x, dorn.y, dorn.z);
        axis.w = 0.0;
        let len = axis.length2();
        if len < SIMD_EPSILON * SIMD_EPSILON {
            axis = Vec3::new(1.0, 0.0, 0.0);
        } else {
            axis.div_assign_scalar(len.sqrt());
        }
        (axis, angle)
    }

    /// `calculateVelocity`.
    pub fn calculate_velocity(
        transform0: &Transform,
        transform1: &Transform,
        time_step: f64,
    ) -> (Vec3, Vec3) {
        let lin_vel = (transform1.origin - transform0.origin) / time_step;
        let (axis, angle) = calculate_diff_axis_angle(transform0, transform1);
        let ang_vel = axis * angle / time_step;
        (lin_vel, ang_vel)
    }
}

// ---------------------------------------------------------------------------
// btAabbUtil2
// ---------------------------------------------------------------------------

/// `AabbExpand`.
#[inline(always)]
pub fn aabb_expand(
    aabb_min: &mut Vec3,
    aabb_max: &mut Vec3,
    expansion_min: &Vec3,
    expansion_max: &Vec3,
) {
    *aabb_min = *aabb_min + *expansion_min;
    *aabb_max = *aabb_max + *expansion_max;
}

/// `TestPointAgainstAabb2`.
#[inline(always)]
pub fn test_point_against_aabb2(aabb_min1: &Vec3, aabb_max1: &Vec3, point: &Vec3) -> bool {
    let mut overlap = true;
    if aabb_min1.x > point.x || aabb_max1.x < point.x {
        overlap = false;
    }
    if aabb_min1.z > point.z || aabb_max1.z < point.z {
        overlap = false;
    }
    if aabb_min1.y > point.y || aabb_max1.y < point.y {
        overlap = false;
    }
    overlap
}

/// `TestAabbAgainstAabb2`.
#[inline(always)]
pub fn test_aabb_against_aabb2(
    aabb_min1: &Vec3,
    aabb_max1: &Vec3,
    aabb_min2: &Vec3,
    aabb_max2: &Vec3,
) -> bool {
    let mut overlap = true;
    if aabb_min1.x > aabb_max2.x || aabb_max1.x < aabb_min2.x {
        overlap = false;
    }
    if aabb_min1.z > aabb_max2.z || aabb_max1.z < aabb_min2.z {
        overlap = false;
    }
    if aabb_min1.y > aabb_max2.y || aabb_max1.y < aabb_min2.y {
        overlap = false;
    }
    overlap
}

/// `btOutcode`.
#[inline(always)]
pub fn bt_outcode(p: &Vec3, half_extent: &Vec3) -> i32 {
    (if p.x < -half_extent.x { 0x01 } else { 0 })
        | (if p.x > half_extent.x { 0x08 } else { 0 })
        | (if p.y < -half_extent.y { 0x02 } else { 0 })
        | (if p.y > half_extent.y { 0x10 } else { 0 })
        | (if p.z < -half_extent.z { 0x4 } else { 0 })
        | (if p.z > half_extent.z { 0x20 } else { 0 })
}

/// `btRayAabb2`.
#[inline]
pub fn bt_ray_aabb2(
    ray_from: &Vec3,
    ray_inv_direction: &Vec3,
    ray_sign: &[u32; 3],
    bounds: &[Vec3; 2],
    tmin: &mut f64,
    lambda_min: f64,
    lambda_max: f64,
) -> bool {
    *tmin = (bounds[ray_sign[0] as usize].x - ray_from.x) * ray_inv_direction.x;
    let mut tmax = (bounds[1 - ray_sign[0] as usize].x - ray_from.x) * ray_inv_direction.x;
    let tymin = (bounds[ray_sign[1] as usize].y - ray_from.y) * ray_inv_direction.y;
    let tymax = (bounds[1 - ray_sign[1] as usize].y - ray_from.y) * ray_inv_direction.y;
    if (*tmin > tymax) || (tymin > tmax) {
        return false;
    }
    if tymin > *tmin {
        *tmin = tymin;
    }
    if tymax < tmax {
        tmax = tymax;
    }
    let tzmin = (bounds[ray_sign[2] as usize].z - ray_from.z) * ray_inv_direction.z;
    let tzmax = (bounds[1 - ray_sign[2] as usize].z - ray_from.z) * ray_inv_direction.z;
    if (*tmin > tzmax) || (tzmin > tmax) {
        return false;
    }
    if tzmin > *tmin {
        *tmin = tzmin;
    }
    if tzmax < tmax {
        tmax = tzmax;
    }
    (*tmin < lambda_max) && (tmax > lambda_min)
}

/// `btRayAabb`.
pub fn bt_ray_aabb(
    ray_from: &Vec3,
    ray_to: &Vec3,
    aabb_min: &Vec3,
    aabb_max: &Vec3,
    param: &mut f64,
    normal: &mut Vec3,
) -> bool {
    let aabb_half_extent = (*aabb_max - *aabb_min) * 0.5;
    let aabb_center = (*aabb_max + *aabb_min) * 0.5;
    let source = *ray_from - aabb_center;
    let target = *ray_to - aabb_center;
    let source_outcode = bt_outcode(&source, &aabb_half_extent);
    let target_outcode = bt_outcode(&target, &aabb_half_extent);
    if (source_outcode & target_outcode) == 0 {
        let mut lambda_enter = 0.0;
        let mut lambda_exit = *param;
        let r = target - source;
        let mut norm_sign = 1.0;
        let mut hit_normal = Vec3::ZERO;
        let mut bit = 1;
        for _j in 0..2 {
            for i in 0..3 {
                if source_outcode & bit != 0 {
                    // (-source[i] - aabbHalfExtent[i]*normSign) / r[i]
                    let lambda = fma(-aabb_half_extent[i], norm_sign, -source[i]) / r[i];
                    if lambda_enter <= lambda {
                        lambda_enter = lambda;
                        hit_normal.set_value(0.0, 0.0, 0.0);
                        hit_normal[i] = norm_sign;
                    }
                } else if target_outcode & bit != 0 {
                    let lambda = fma(-aabb_half_extent[i], norm_sign, -source[i]) / r[i];
                    bt_set_min(&mut lambda_exit, lambda);
                }
                bit <<= 1;
            }
            norm_sign = -1.0;
        }
        if lambda_enter <= lambda_exit {
            *param = lambda_enter;
            *normal = hit_normal;
            return true;
        }
    }
    false
}

/// `btTransformAabb(halfExtents, margin, t, min, max)`.
#[inline]
pub fn transform_aabb_half(half_extents: &Vec3, margin: f64, t: &Transform) -> (Vec3, Vec3) {
    let half_extents_with_margin = *half_extents + Vec3::new(margin, margin, margin);
    let abs_b = t.basis.absolute();
    let center = t.origin;
    let extent = half_extents_with_margin.dot3(&abs_b.el[0], &abs_b.el[1], &abs_b.el[2]);
    (center - extent, center + extent)
}

/// `btTransformAabb(localAabbMin, localAabbMax, margin, trans, min, max)`.
#[inline]
pub fn transform_aabb(
    local_aabb_min: &Vec3,
    local_aabb_max: &Vec3,
    margin: f64,
    trans: &Transform,
) -> (Vec3, Vec3) {
    let mut local_half_extents = 0.5 * (*local_aabb_max - *local_aabb_min);
    local_half_extents += Vec3::new(margin, margin, margin);
    let local_center = 0.5 * (*local_aabb_max + *local_aabb_min);
    let abs_b = trans.basis.absolute();
    let center = trans.apply(&local_center);
    let extent = local_half_extents.dot3(&abs_b.el[0], &abs_b.el[1], &abs_b.el[2]);
    (center - extent, center + extent)
}

// ---------------------------------------------------------------------------
// btAlignedObjectArray sorting
// ---------------------------------------------------------------------------

/// `btAlignedObjectArray::quickSort` (`quickSortInternal`): the same
/// unstable partitioning, so equal keys end up in the same order.
pub fn quick_sort<T: Copy, F: Fn(&T, &T) -> bool>(a: &mut [T], less: F) {
    if a.len() > 1 {
        quick_sort_internal(a, &less, 0, a.len() as i64 - 1);
    }
}

fn quick_sort_internal<T: Copy, F: Fn(&T, &T) -> bool>(a: &mut [T], less: &F, lo: i64, hi: i64) {
    let mut i = lo;
    let mut j = hi;
    let x = a[((lo + hi) / 2) as usize];
    loop {
        while less(&a[i as usize], &x) {
            i += 1;
        }
        while less(&x, &a[j as usize]) {
            j -= 1;
        }
        if i <= j {
            a.swap(i as usize, j as usize);
            i += 1;
            j -= 1;
        }
        if i > j {
            break;
        }
    }
    if lo < j {
        quick_sort_internal(a, less, lo, j);
    }
    if i < hi {
        quick_sort_internal(a, less, i, hi);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_is_fused_in_bullet_order() {
        let a = Vec3::new(1.0 + 1e-9, 3.0, 1e-3);
        let b = Vec3::new(1.0 - 1e-9, 7.0, 3e-3);
        assert_eq!(a.dot(&b), fma(a.z, b.z, fma(a.x, b.x, a.y * b.y)));
    }

    #[test]
    fn quat_matrix_roundtrip() {
        let q = Quat::new(0.1, 0.2, 0.3, 0.9).normalized();
        let m = Mat3::from_quat(&q);
        let r = m.get_rotation();
        assert!((r.x - q.x).abs() < 1e-12 && (r.w - q.w).abs() < 1e-12);
    }

    #[test]
    fn quick_sort_matches_partition_scheme() {
        let mut v = vec![(3, 'a'), (1, 'b'), (3, 'c'), (2, 'd'), (1, 'e')];
        quick_sort(&mut v, |a, b| a.0 < b.0);
        assert_eq!(
            v.iter().map(|p| p.0).collect::<Vec<_>>(),
            vec![1, 1, 2, 3, 3]
        );
    }
}
