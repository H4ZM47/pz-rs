//! The `org.joml.Matrix4f` operations the atlas and model-camera code
//! calls, with JOML's float arithmetic (`Math.sin` through doubles,
//! `cosFromSin`, `fma` as a separate multiply and add): post-multiplying
//! builders, `project` and `unprojectInv`. Storage is column-major,
//! `m[c][r]` holding JOML's `m<c><r>`.

// Loops index matrix columns and rows as JOML's fields name them.
#![allow(clippy::needless_range_loop, clippy::approx_constant)]

/// `org.joml.Math.sin(float)`.
pub fn sin(a: f32) -> f32 {
    (a as f64).sin() as f32
}

/// `org.joml.Math.cosFromSin(sin, angle)` (without `FASTMATH`).
pub fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = (1.0f32 - sin * sin).sqrt();
    let two_pi = std::f32::consts::PI * 2.0;
    let a = angle + 1.570_796_4;
    let mut b = a - ((a / two_pi) as i32) as f32 * two_pi;
    if (b as f64) < 0.0 {
        b += two_pi;
    }
    if b >= std::f32::consts::PI {
        -cos
    } else {
        cos
    }
}

/// `Math.fma` with `joml.useMathFma` off.
fn fma(a: f32, b: f32, c: f32) -> f32 {
    a * b + c
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix4f {
    pub m: [[f32; 4]; 4],
}

impl Default for Matrix4f {
    fn default() -> Self {
        Self::identity()
    }
}

impl Matrix4f {
    pub const fn identity() -> Self {
        Matrix4f {
            m: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub fn from_cols(m: [[f32; 4]; 4]) -> Self {
        Matrix4f { m }
    }

    /// `get(float[])`: column-major.
    pub fn to_array(&self) -> [f32; 16] {
        let mut a = [0.0; 16];
        for c in 0..4 {
            a[c * 4..c * 4 + 4].copy_from_slice(&self.m[c]);
        }
        a
    }

    /// `setOrtho(l, r, b, t, n, f)`.
    pub fn set_ortho(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> Self {
        let mut o = Self::identity();
        o.m[0][0] = 2.0 / (r - l);
        o.m[1][1] = 2.0 / (t - b);
        o.m[2][2] = 2.0 / (n - f);
        o.m[3][0] = (r + l) / (l - r);
        o.m[3][1] = (t + b) / (b - t);
        o.m[3][2] = (f + n) / (n - f);
        o
    }

    /// `setOrtho2D(l, r, b, t)`.
    pub fn set_ortho_2d(l: f32, r: f32, b: f32, t: f32) -> Self {
        let mut o = Self::identity();
        o.m[0][0] = 2.0 / (r - l);
        o.m[1][1] = 2.0 / (t - b);
        o.m[2][2] = -1.0;
        o.m[3][0] = (r + l) / (l - r);
        o.m[3][1] = (t + b) / (b - t);
        o
    }

    /// `ortho(l, r, b, t, n, f)`: this times an orthographic projection.
    pub fn ortho(&self, l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> Self {
        let rm00 = 2.0 / (r - l);
        let rm11 = 2.0 / (t - b);
        let rm22 = 2.0 / (n - f);
        let rm30 = (l + r) / (l - r);
        let rm31 = (t + b) / (b - t);
        let rm32 = (f + n) / (n - f);
        let m = &self.m;
        let mut o = *self;
        for k in 0..4 {
            o.m[3][k] = m[0][k] * rm30 + m[1][k] * rm31 + m[2][k] * rm32 + m[3][k];
            o.m[0][k] = m[0][k] * rm00;
            o.m[1][k] = m[1][k] * rm11;
            o.m[2][k] = m[2][k] * rm22;
        }
        o
    }

    /// `scale(x, y, z)`.
    pub fn scale(&self, x: f32, y: f32, z: f32) -> Self {
        let mut o = *self;
        for k in 0..4 {
            o.m[0][k] *= x;
            o.m[1][k] *= y;
            o.m[2][k] *= z;
        }
        o
    }

    /// `scale(xyz)`.
    pub fn scale1(&self, s: f32) -> Self {
        self.scale(s, s, s)
    }

    /// `translate(x, y, z)`.
    pub fn translate(&self, x: f32, y: f32, z: f32) -> Self {
        let m = &self.m;
        let mut o = *self;
        for k in 0..4 {
            o.m[3][k] = fma(m[0][k], x, fma(m[1][k], y, fma(m[2][k], z, m[3][k])));
        }
        o
    }

    /// `translation(x, y, z)`.
    pub fn translation(x: f32, y: f32, z: f32) -> Self {
        let mut o = Self::identity();
        o.m[3][0] = x;
        o.m[3][1] = y;
        o.m[3][2] = z;
        o
    }

    /// `rotateX(ang)`.
    pub fn rotate_x(&self, ang: f32) -> Self {
        let s = sin(ang);
        let c = cos_from_sin(s, ang);
        let m = &self.m;
        let mut o = *self;
        for k in 0..4 {
            o.m[2][k] = m[1][k] * -s + m[2][k] * c;
            o.m[1][k] = m[1][k] * c + m[2][k] * s;
        }
        o
    }

    /// `rotateY(ang)`.
    pub fn rotate_y(&self, ang: f32) -> Self {
        let s = sin(ang);
        let c = cos_from_sin(s, ang);
        let m = &self.m;
        let mut o = *self;
        for k in 0..4 {
            o.m[0][k] = m[0][k] * c + m[2][k] * -s;
            o.m[2][k] = m[0][k] * s + m[2][k] * c;
        }
        o
    }

    /// `rotateZ(ang)` (`rotateTowardsXY(sin, cos)`).
    pub fn rotate_z(&self, ang: f32) -> Self {
        let s = sin(ang);
        let c = cos_from_sin(s, ang);
        let m = &self.m;
        let mut o = *self;
        for k in 0..4 {
            o.m[0][k] = m[0][k] * c + m[1][k] * s;
            o.m[1][k] = m[0][k] * -s + m[1][k] * c;
        }
        o
    }

    /// `rotate(ang, x, y, z)` about a unit axis (the only form the ported
    /// code passes).
    pub fn rotate(&self, ang: f32, x: f32, y: f32, z: f32) -> Self {
        let one = |v: f32| v.abs() == 1.0;
        if y == 0.0 && z == 0.0 && one(x) {
            self.rotate_x(x * ang)
        } else if x == 0.0 && z == 0.0 && one(y) {
            self.rotate_y(y * ang)
        } else if x == 0.0 && y == 0.0 && one(z) {
            self.rotate_z(z * ang)
        } else {
            unimplemented!("rotate about a non-axis vector")
        }
    }

    /// `rotateXYZ(ax, ay, az)`.
    pub fn rotate_xyz(&self, ax: f32, ay: f32, az: f32) -> Self {
        let (sx, sy, sz) = (sin(ax), sin(ay), sin(az));
        let (cx, cy, cz) = (
            cos_from_sin(sx, ax),
            cos_from_sin(sy, ay),
            cos_from_sin(sz, az),
        );
        let m = &self.m;
        let mut o = *self;
        for k in 0..4 {
            let n1 = m[1][k] * cx + m[2][k] * sx;
            let n2 = m[1][k] * -sx + m[2][k] * cx;
            let n0 = m[0][k] * cy + n2 * -sy;
            o.m[2][k] = m[0][k] * sy + n2 * cy;
            o.m[0][k] = n0 * cz + n1 * sz;
            o.m[1][k] = n0 * -sz + n1 * cz;
        }
        o
    }

    /// `mul(right)`: `this * right`.
    pub fn mul(&self, r: &Matrix4f) -> Self {
        let m = &self.m;
        let mut o = Matrix4f { m: [[0.0; 4]; 4] };
        for c in 0..4 {
            let rc = r.m[c];
            for k in 0..4 {
                o.m[c][k] = fma(
                    m[0][k],
                    rc[0],
                    fma(m[1][k], rc[1], fma(m[2][k], rc[2], m[3][k] * rc[3])),
                );
            }
        }
        o
    }

    /// `transpose()`.
    pub fn transpose(&self) -> Self {
        let mut o = *self;
        for c in 0..4 {
            for r in 0..4 {
                o.m[c][r] = self.m[r][c];
            }
        }
        o
    }

    /// `invertAffine()` (what `invert()` does for the affine matrices the
    /// ported code inverts).
    pub fn invert_affine(&self) -> Self {
        let m = |c: usize, r: usize| self.m[c][r];
        let m11m00 = m(0, 0) * m(1, 1);
        let m10m01 = m(0, 1) * m(1, 0);
        let m10m02 = m(0, 2) * m(1, 0);
        let m12m00 = m(0, 0) * m(1, 2);
        let m12m01 = m(0, 1) * m(1, 2);
        let m11m02 = m(0, 2) * m(1, 1);
        let det =
            (m11m00 - m10m01) * m(2, 2) + (m10m02 - m12m00) * m(2, 1) + (m12m01 - m11m02) * m(2, 0);
        let s = 1.0 / det;
        let m10m22 = m(1, 0) * m(2, 2);
        let m10m21 = m(1, 0) * m(2, 1);
        let m11m22 = m(1, 1) * m(2, 2);
        let m11m20 = m(1, 1) * m(2, 0);
        let m12m21 = m(1, 2) * m(2, 1);
        let m12m20 = m(1, 2) * m(2, 0);
        let m20m02 = m(2, 0) * m(0, 2);
        let m20m01 = m(2, 0) * m(0, 1);
        let m21m02 = m(2, 1) * m(0, 2);
        let m21m00 = m(2, 1) * m(0, 0);
        let m22m01 = m(2, 2) * m(0, 1);
        let m22m00 = m(2, 2) * m(0, 0);
        let (m30, m31, m32) = (m(3, 0), m(3, 1), m(3, 2));
        let nm31 = (m20m02 * m31 - m20m01 * m32 + m21m00 * m32 - m21m02 * m30 + m22m01 * m30
            - m22m00 * m31)
            * s;
        let nm32 = (m11m02 * m30 - m12m01 * m30 + m12m00 * m31 - m10m02 * m31 + m10m01 * m32
            - m11m00 * m32)
            * s;
        let nm30 = (m10m22 * m31 - m10m21 * m32 + m11m20 * m32 - m11m22 * m30 + m12m21 * m30
            - m12m20 * m31)
            * s;
        Matrix4f {
            m: [
                [
                    (m11m22 - m12m21) * s,
                    (m21m02 - m22m01) * s,
                    (m12m01 - m11m02) * s,
                    0.0,
                ],
                [
                    (m12m20 - m10m22) * s,
                    (m22m00 - m20m02) * s,
                    (m10m02 - m12m00) * s,
                    0.0,
                ],
                [
                    (m10m21 - m11m20) * s,
                    (m20m01 - m21m00) * s,
                    (m11m00 - m10m01) * s,
                    0.0,
                ],
                [nm30, nm31, nm32, 1.0],
            ],
        }
    }

    /// `project(x, y, z, viewport, dest)`: window coordinates.
    pub fn project(&self, x: f32, y: f32, z: f32, viewport: [i32; 4]) -> [f32; 3] {
        let m = &self.m;
        let inv_w = 1.0 / (m[0][3] * x + m[1][3] * y + m[2][3] * z + m[3][3]);
        let nx = (m[0][0] * x + m[1][0] * y + m[2][0] * z + m[3][0]) * inv_w;
        let ny = (m[0][1] * x + m[1][1] * y + m[2][1] * z + m[3][1]) * inv_w;
        let nz = (m[0][2] * x + m[1][2] * y + m[2][2] * z + m[3][2]) * inv_w;
        [
            (nx * 0.5 + 0.5) * viewport[2] as f32 + viewport[0] as f32,
            (ny * 0.5 + 0.5) * viewport[3] as f32 + viewport[1] as f32,
            (1.0 + nz) * 0.5,
        ]
    }

    /// `unprojectInv(winX, winY, winZ, viewport, dest)`.
    pub fn unproject_inv(&self, wx: f32, wy: f32, wz: f32, viewport: [i32; 4]) -> [f32; 3] {
        let m = &self.m;
        let x = (wx - viewport[0] as f32) / viewport[2] as f32 * 2.0 - 1.0;
        let y = (wy - viewport[1] as f32) / viewport[3] as f32 * 2.0 - 1.0;
        let z = wz + wz - 1.0;
        let inv_w = 1.0 / (m[0][3] * x + m[1][3] * y + m[2][3] * z + m[3][3]);
        [
            (m[0][0] * x + m[1][0] * y + m[2][0] * z + m[3][0]) * inv_w,
            (m[0][1] * x + m[1][1] * y + m[2][1] * z + m[3][1]) * inv_w,
            (m[0][2] * x + m[1][2] * y + m[2][2] * z + m[3][2]) * inv_w,
        ]
    }

    /// The matrix applied to a point (column vector), with `w`.
    pub fn transform(&self, p: [f32; 4]) -> [f32; 4] {
        std::array::from_fn(|k| {
            self.m[0][k] * p[0] + self.m[1][k] * p[1] + self.m[2][k] * p[2] + self.m[3][k] * p[3]
        })
    }
}

impl Matrix4f {
    /// `transformPosition(v)` (`Vector3f.mulPosition`).
    pub fn transform_position(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.m;
        std::array::from_fn(|k| {
            fma(
                m[0][k],
                v[0],
                fma(m[1][k], v[1], fma(m[2][k], v[2], m[3][k])),
            )
        })
    }

    /// `transformDirection(v)` (`Vector3f.mulDirection`).
    pub fn transform_direction(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.m;
        std::array::from_fn(|k| fma(m[0][k], v[0], fma(m[1][k], v[1], m[2][k] * v[2])))
    }

    /// `transformProject(v)` (`Vector3f.mulProject`).
    pub fn transform_project(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.m;
        let inv_w = 1.0
            / fma(
                m[0][3],
                v[0],
                fma(m[1][3], v[1], fma(m[2][3], v[2], m[3][3])),
            );
        std::array::from_fn(|k| {
            fma(
                m[0][k],
                v[0],
                fma(m[1][k], v[1], fma(m[2][k], v[2], m[3][k])),
            ) * inv_w
        })
    }

    /// `getTranslation(dest)`.
    pub fn get_translation(&self) -> [f32; 3] {
        [self.m[3][0], self.m[3][1], self.m[3][2]]
    }

    /// `getScale(dest)`: the length of each basis column.
    pub fn get_scale(&self) -> [f32; 3] {
        let m = &self.m;
        let l = |c: usize| {
            (((m[c][0] * m[c][0] + m[c][1] * m[c][1] + m[c][2] * m[c][2]) as f64).sqrt()) as f32
        };
        [l(0), l(1), l(2)]
    }

    /// `rotationX(ang)`.
    pub fn rotation_x(ang: f32) -> Self {
        let (s, c) = (sin(ang), cos_from_sin(sin(ang), ang));
        let mut o = Self::identity();
        (o.m[1][1], o.m[1][2], o.m[2][1], o.m[2][2]) = (c, s, -s, c);
        o
    }

    /// `rotationY(ang)`.
    pub fn rotation_y(ang: f32) -> Self {
        let (s, c) = (sin(ang), cos_from_sin(sin(ang), ang));
        let mut o = Self::identity();
        (o.m[0][0], o.m[0][2], o.m[2][0], o.m[2][2]) = (c, -s, s, c);
        o
    }

    /// `rotationZ(ang)`.
    pub fn rotation_z(ang: f32) -> Self {
        let (s, c) = (sin(ang), cos_from_sin(sin(ang), ang));
        let mut o = Self::identity();
        (o.m[0][0], o.m[0][1], o.m[1][0], o.m[1][1]) = (c, s, -s, c);
        o
    }

    /// `rotation(angle, x, y, z)` about a unit coordinate axis.
    pub fn rotation(ang: f32, x: f32, y: f32, z: f32) -> Self {
        let one = |v: f32| v.abs() == 1.0;
        if y == 0.0 && z == 0.0 && one(x) {
            Self::rotation_x(x * ang)
        } else if x == 0.0 && z == 0.0 && one(y) {
            Self::rotation_y(y * ang)
        } else if x == 0.0 && y == 0.0 && one(z) {
            Self::rotation_z(z * ang)
        } else {
            unimplemented!("rotation about a non-axis vector")
        }
    }

    /// The 3x3 of `rotationXYZ` / `setRotationXYZ`.
    fn rotation_xyz_3x3(ax: f32, ay: f32, az: f32) -> [[f32; 3]; 3] {
        let (sx, sy, sz) = (sin(ax), sin(ay), sin(az));
        let (cx, cy, cz) = (
            cos_from_sin(sx, ax),
            cos_from_sin(sy, ay),
            cos_from_sin(sz, az),
        );
        let nm01 = -sx * -sy;
        let nm02 = cx * -sy;
        [
            [cy * cz, nm01 * cz + cx * sz, nm02 * cz + sx * sz],
            [cy * -sz, nm01 * -sz + cx * cz, nm02 * -sz + sx * cz],
            [sy, -sx * cy, cx * cy],
        ]
    }

    /// `rotationXYZ(ax, ay, az)`.
    pub fn rotation_xyz(ax: f32, ay: f32, az: f32) -> Self {
        Self::identity().set_rotation_xyz(ax, ay, az)
    }

    /// `setRotationXYZ(ax, ay, az)`: replaces the upper 3x3.
    pub fn set_rotation_xyz(&self, ax: f32, ay: f32, az: f32) -> Self {
        let r = Self::rotation_xyz_3x3(ax, ay, az);
        let mut o = *self;
        for c in 0..3 {
            o.m[c][..3].copy_from_slice(&r[c]);
        }
        o
    }

    /// `invert()` for the affine matrices the ported code builds
    /// (`invertAffine`, as JOML picks for a matrix flagged affine).
    pub fn invert(&self) -> Self {
        if *self == Self::identity() {
            return *self;
        }
        self.invert_affine()
    }
}

/// `org.joml.Vector3f` operations, with JOML's arithmetic.
pub mod vec3 {
    pub type V = [f32; 3];
    pub fn add(a: V, b: V) -> V {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }
    pub fn sub(a: V, b: V) -> V {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    pub fn mul(a: V, s: f32) -> V {
        [a[0] * s, a[1] * s, a[2] * s]
    }
    /// `dot(v)`.
    pub fn dot(a: V, b: V) -> f32 {
        a[0] * b[0] + (a[1] * b[1] + a[2] * b[2])
    }
    /// `length()`.
    pub fn length(a: V) -> f32 {
        ((a[0] * a[0] + (a[1] * a[1] + a[2] * a[2])) as f64).sqrt() as f32
    }
    /// `normalize()`.
    pub fn normalize(a: V) -> V {
        super::normalize(a)
    }
    /// `cross(v, dest)`.
    pub fn cross(a: V, b: V) -> V {
        [
            a[1] * b[2] + -a[2] * b[1],
            a[2] * b[0] + -a[0] * b[2],
            a[0] * b[1] + -a[1] * b[0],
        ]
    }
    /// `equals` (bitwise, as `Float.floatToIntBits`).
    pub fn bit_eq(a: V, b: V) -> bool {
        a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits())
    }
}

/// `org.joml.Quaternionf`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quaternionf {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quaternionf {
    /// `fromAxisAngleRad(x, y, z, angle)`.
    pub fn from_axis_angle_rad(ax: f32, ay: f32, az: f32, angle: f32) -> Self {
        let h = angle / 2.0;
        let s = sin(h);
        let len = ((ax * ax + ay * ay + az * az) as f64).sqrt() as f32;
        Quaternionf {
            x: ax / len * s,
            y: ay / len * s,
            z: az / len * s,
            w: cos_from_sin(s, h),
        }
    }

    /// `fromAxisAngleDeg(axis, angle)`.
    pub fn from_axis_angle_deg(axis: [f32; 3], angle: f32) -> Self {
        Self::from_axis_angle_rad(
            axis[0],
            axis[1],
            axis[2],
            (angle as f64).to_radians() as f32,
        )
    }

    /// `rotationXYZ(ax, ay, az)`.
    pub fn rotation_xyz(ax: f32, ay: f32, az: f32) -> Self {
        let (sx, sy, sz) = (sin(ax * 0.5), sin(ay * 0.5), sin(az * 0.5));
        let (cx, cy, cz) = (
            cos_from_sin(sx, ax * 0.5),
            cos_from_sin(sy, ay * 0.5),
            cos_from_sin(sz, az * 0.5),
        );
        let (cycz, sysz, sycz, cysz) = (cy * cz, sy * sz, sy * cz, cy * sz);
        Quaternionf {
            w: cx * cycz - sx * sysz,
            x: sx * cycz + cx * sysz,
            y: cx * sycz - sx * cysz,
            z: cx * cysz + sx * sycz,
        }
    }

    /// `mul(q)`: `this * q`.
    pub fn mul(&self, q: &Quaternionf) -> Self {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        Quaternionf {
            x: fma(w, q.x, fma(x, q.w, fma(y, q.z, -z * q.y))),
            y: fma(w, q.y, fma(-x, q.z, fma(y, q.w, z * q.x))),
            z: fma(w, q.z, fma(x, q.y, fma(-y, q.x, z * q.w))),
            w: fma(w, q.w, fma(-x, q.x, fma(-y, q.y, -z * q.z))),
        }
    }

    /// `getEulerAnglesXYZ(dest)`.
    pub fn euler_angles_xyz(&self) -> [f32; 3] {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        let atan2 = |a: f32, b: f32| (a as f64).atan2(b as f64) as f32;
        let r = 2.0 * (x * z + y * w);
        let asin = if r <= -1.0 {
            -1.5707964
        } else if r >= 1.0 {
            1.5707964
        } else {
            (r as f64).asin() as f32
        };
        [
            atan2(2.0 * (x * w - y * z), 1.0 - 2.0 * (x * x + y * y)),
            asin,
            atan2(2.0 * (z * w - x * y), 1.0 - 2.0 * (y * y + z * z)),
        ]
    }
}

/// `Vector3f.rotateY(angle)`.
pub fn rotate_y_vec(v: [f32; 3], angle: f32) -> [f32; 3] {
    let s = sin(angle);
    let c = cos_from_sin(s, angle);
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

/// `Vector3f.normalize()`.
pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let s = 1.0 / (v[0] * v[0] + (v[1] * v[1] + v[2] * v[2])).sqrt();
    [v[0] * s, v[1] * s, v[2] * s]
}

/// `UI3DScene.intersect_ray_plane` with a plane through `point` with
/// normal `normal`: the hit, or `None` when the ray is parallel or the hit
/// lies behind its origin.
pub fn intersect_ray_plane(
    point: [f32; 3],
    normal: [f32; 3],
    origin: [f32; 3],
    dir: [f32; 3],
) -> Option<[f32; 3]> {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + (a[1] * b[1] + a[2] * b[2]);
    // u = dir * 100, w = origin - point.
    let u = [dir[0] * 100.0, dir[1] * 100.0, dir[2] * 100.0];
    let w = [
        origin[0] - point[0],
        origin[1] - point[1],
        origin[2] - point[2],
    ];
    let d = dot(normal, u);
    let n = -dot(normal, w);
    if d.abs() < 1.0e-8 {
        return None;
    }
    let s = n / d;
    if s < 0.0 || s > 1.0 {
        return None;
    }
    Some([
        origin[0] + u[0] * s,
        origin[1] + u[1] * s,
        origin[2] + u[2] * s,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos_from_sin_signs() {
        for a in [0.0f32, 0.5, 1.5, 2.0, 3.0, -0.5, -2.0, 4.0] {
            assert!((cos_from_sin(sin(a), a) - a.cos()).abs() < 1e-6, "{a}");
        }
    }

    #[test]
    fn invert_and_project_round_trip() {
        let m = Matrix4f::set_ortho(-1.0, 1.0, -1.0, 1.0, -10.0, 10.0)
            .scale(-2.0, 2.0, 2.0)
            .rotate(0.5235988, 1.0, 0.0, 0.0)
            .rotate(0.7, 0.0, 1.0, 0.0)
            .translate(0.3, 0.0, -0.2);
        let vp = [0, 0, 512, 512];
        let w = m.project(0.1, 0.2, 0.3, vp);
        let back = m.invert_affine().unproject_inv(w[0], w[1], w[2], vp);
        for (a, b) in back.iter().zip([0.1, 0.2, 0.3]) {
            assert!((a - b).abs() < 1e-5, "{back:?}");
        }
        let i = m.mul(&m.invert_affine());
        for c in 0..4 {
            for r in 0..4 {
                assert!((i.m[c][r] - f32::from(c == r)).abs() < 1e-5);
            }
        }
    }
}
