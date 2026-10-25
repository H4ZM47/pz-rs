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
