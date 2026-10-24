//! The assimp math types the importers and post-processing steps use
//! (`aiMatrix4x4`, `aiQuaternion`, `aiVector3D`), ported operation for
//! operation so results round the way the native library rounds them.
//!
//! `AiMat4` is row-major (`m[row][col]`, `a1 = m[0][0]`, `a4 = m[0][3]`)
//! and transforms column vectors (`M * v`), like assimp.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiMat4 {
    pub m: [[f32; 4]; 4],
}

impl Default for AiMat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl AiMat4 {
    pub const IDENTITY: AiMat4 = AiMat4 {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    pub fn is_identity(&self) -> bool {
        // aiMatrix4x4::IsIdentity with epsilon 1e-6 (ai_real float).
        let e = 1e-6f32;
        let m = &self.m;
        m[0][1] <= e
            && m[0][1] >= -e
            && m[0][2] <= e
            && m[0][2] >= -e
            && m[0][3] <= e
            && m[0][3] >= -e
            && m[1][0] <= e
            && m[1][0] >= -e
            && m[1][2] <= e
            && m[1][2] >= -e
            && m[1][3] <= e
            && m[1][3] >= -e
            && m[2][0] <= e
            && m[2][0] >= -e
            && m[2][1] <= e
            && m[2][1] >= -e
            && m[2][3] <= e
            && m[2][3] >= -e
            && m[3][0] <= e
            && m[3][0] >= -e
            && m[3][1] <= e
            && m[3][1] >= -e
            && m[3][2] <= e
            && m[3][2] >= -e
            && m[0][0] <= 1.0 + e
            && m[0][0] >= 1.0 - e
            && m[1][1] <= 1.0 + e
            && m[1][1] >= 1.0 - e
            && m[2][2] <= 1.0 + e
            && m[2][2] >= 1.0 - e
            && m[3][3] <= 1.0 + e
            && m[3][3] >= 1.0 - e
    }

    /// `aiMatrix4x4::operator*=` / `operator*`: `self * o`.
    pub fn mul(&self, o: &AiMat4) -> AiMat4 {
        let a = &self.m;
        let b = &o.m;
        let mut r = [[0.0f32; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                r[i][j] =
                    a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j] + a[i][3] * b[3][j];
            }
        }
        AiMat4 { m: r }
    }

    pub fn determinant(&self) -> f32 {
        let [[a1, a2, a3, a4], [b1, b2, b3, b4], [c1, c2, c3, c4], [d1, d2, d3, d4]] = self.m;
        a1 * b2 * c3 * d4 - a1 * b2 * c4 * d3 + a1 * b3 * c4 * d2 - a1 * b3 * c2 * d4
            + a1 * b4 * c2 * d3
            - a1 * b4 * c3 * d2
            - a2 * b3 * c4 * d1
            + a2 * b3 * c1 * d4
            - a2 * b4 * c1 * d3
            + a2 * b4 * c3 * d1
            - a2 * b1 * c3 * d4
            + a2 * b1 * c4 * d3
            + a3 * b4 * c1 * d2
            - a3 * b4 * c2 * d1
            + a3 * b1 * c2 * d4
            - a3 * b1 * c4 * d2
            + a3 * b2 * c4 * d1
            - a3 * b2 * c1 * d4
            - a4 * b1 * c2 * d3
            + a4 * b1 * c3 * d2
            - a4 * b2 * c3 * d1
            + a4 * b2 * c1 * d3
            - a4 * b3 * c1 * d2
            + a4 * b3 * c2 * d1
    }

    /// `aiMatrix4x4::Inverse` (all NaN when singular).
    pub fn inverse(&self) -> AiMat4 {
        let det = self.determinant();
        if det == 0.0 {
            return AiMat4 {
                m: [[f32::NAN; 4]; 4],
            };
        }
        let invdet = 1.0 / det;
        let [[a1, a2, a3, a4], [b1, b2, b3, b4], [c1, c2, c3, c4], [d1, d2, d3, d4]] = self.m;
        let mut r = [[0.0f32; 4]; 4];
        r[0][0] = invdet
            * (b2 * (c3 * d4 - c4 * d3) + b3 * (c4 * d2 - c2 * d4) + b4 * (c2 * d3 - c3 * d2));
        r[0][1] = -invdet
            * (a2 * (c3 * d4 - c4 * d3) + a3 * (c4 * d2 - c2 * d4) + a4 * (c2 * d3 - c3 * d2));
        r[0][2] = invdet
            * (a2 * (b3 * d4 - b4 * d3) + a3 * (b4 * d2 - b2 * d4) + a4 * (b2 * d3 - b3 * d2));
        r[0][3] = -invdet
            * (a2 * (b3 * c4 - b4 * c3) + a3 * (b4 * c2 - b2 * c4) + a4 * (b2 * c3 - b3 * c2));
        r[1][0] = -invdet
            * (b1 * (c3 * d4 - c4 * d3) + b3 * (c4 * d1 - c1 * d4) + b4 * (c1 * d3 - c3 * d1));
        r[1][1] = invdet
            * (a1 * (c3 * d4 - c4 * d3) + a3 * (c4 * d1 - c1 * d4) + a4 * (c1 * d3 - c3 * d1));
        r[1][2] = -invdet
            * (a1 * (b3 * d4 - b4 * d3) + a3 * (b4 * d1 - b1 * d4) + a4 * (b1 * d3 - b3 * d1));
        r[1][3] = invdet
            * (a1 * (b3 * c4 - b4 * c3) + a3 * (b4 * c1 - b1 * c4) + a4 * (b1 * c3 - b3 * c1));
        r[2][0] = invdet
            * (b1 * (c2 * d4 - c4 * d2) + b2 * (c4 * d1 - c1 * d4) + b4 * (c1 * d2 - c2 * d1));
        r[2][1] = -invdet
            * (a1 * (c2 * d4 - c4 * d2) + a2 * (c4 * d1 - c1 * d4) + a4 * (c1 * d2 - c2 * d1));
        r[2][2] = invdet
            * (a1 * (b2 * d4 - b4 * d2) + a2 * (b4 * d1 - b1 * d4) + a4 * (b1 * d2 - b2 * d1));
        r[2][3] = -invdet
            * (a1 * (b2 * c4 - b4 * c2) + a2 * (b4 * c1 - b1 * c4) + a4 * (b1 * c2 - b2 * c1));
        r[3][0] = -invdet
            * (b1 * (c2 * d3 - c3 * d2) + b2 * (c3 * d1 - c1 * d3) + b3 * (c1 * d2 - c2 * d1));
        r[3][1] = invdet
            * (a1 * (c2 * d3 - c3 * d2) + a2 * (c3 * d1 - c1 * d3) + a3 * (c1 * d2 - c2 * d1));
        r[3][2] = -invdet
            * (a1 * (b2 * d3 - b3 * d2) + a2 * (b3 * d1 - b1 * d3) + a3 * (b1 * d2 - b2 * d1));
        r[3][3] = invdet
            * (a1 * (b2 * c3 - b3 * c2) + a2 * (b3 * c1 - b1 * c3) + a3 * (b1 * c2 - b2 * c1));
        AiMat4 { m: r }
    }

    pub fn transpose(&self) -> AiMat4 {
        let mut r = [[0.0f32; 4]; 4];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = self.m[j][i];
            }
        }
        AiMat4 { m: r }
    }

    pub fn translation(v: [f32; 3]) -> AiMat4 {
        let mut m = Self::IDENTITY;
        m.m[0][3] = v[0];
        m.m[1][3] = v[1];
        m.m[2][3] = v[2];
        m
    }

    pub fn scaling(v: [f32; 3]) -> AiMat4 {
        let mut m = Self::IDENTITY;
        m.m[0][0] = v[0];
        m.m[1][1] = v[1];
        m.m[2][2] = v[2];
        m
    }

    pub fn rotation_x(a: f32) -> AiMat4 {
        let mut m = Self::IDENTITY;
        let (s, c) = (a.sin(), a.cos());
        m.m[1][1] = c;
        m.m[2][2] = c;
        m.m[1][2] = -s;
        m.m[2][1] = s;
        m
    }

    pub fn rotation_y(a: f32) -> AiMat4 {
        let mut m = Self::IDENTITY;
        let (s, c) = (a.sin(), a.cos());
        m.m[0][0] = c;
        m.m[2][2] = c;
        m.m[0][2] = s;
        m.m[2][0] = -s;
        m
    }

    pub fn rotation_z(a: f32) -> AiMat4 {
        let mut m = Self::IDENTITY;
        let (s, c) = (a.sin(), a.cos());
        m.m[0][0] = c;
        m.m[1][1] = c;
        m.m[0][1] = -s;
        m.m[1][0] = s;
        m
    }

    /// `aiMatrix4x4(const aiMatrix3x3&)`.
    pub fn from_3x3(r: &[[f32; 3]; 3]) -> AiMat4 {
        let mut m = Self::IDENTITY;
        for (dst, src) in m.m.iter_mut().zip(r) {
            dst[..3].copy_from_slice(src);
        }
        m
    }

    /// `aiMatrix4x4(scaling, rotation, position)`.
    pub fn compose(scaling: [f32; 3], rotation: AiQuat, position: [f32; 3]) -> AiMat4 {
        let r = rotation.get_matrix();
        let mut m = AiMat4::IDENTITY;
        m.m[0][0] = r[0][0] * scaling[0];
        m.m[0][1] = r[0][1] * scaling[1];
        m.m[0][2] = r[0][2] * scaling[2];
        m.m[0][3] = position[0];
        m.m[1][0] = r[1][0] * scaling[0];
        m.m[1][1] = r[1][1] * scaling[1];
        m.m[1][2] = r[1][2] * scaling[2];
        m.m[1][3] = position[1];
        m.m[2][0] = r[2][0] * scaling[0];
        m.m[2][1] = r[2][1] * scaling[1];
        m.m[2][2] = r[2][2] * scaling[2];
        m.m[2][3] = position[2];
        m
    }

    /// `aiMatrix4x4::Decompose(scaling, rotation, position)`.
    pub fn decompose(&self) -> ([f32; 3], AiQuat, [f32; 3]) {
        let m = &self.m;
        let pos = [m[0][3], m[1][3], m[2][3]];
        let mut cols = [
            [m[0][0], m[1][0], m[2][0]],
            [m[0][1], m[1][1], m[2][1]],
            [m[0][2], m[1][2], m[2][2]],
        ];
        let mut sc = [vlen(cols[0]), vlen(cols[1]), vlen(cols[2])];
        if self.determinant() < 0.0 {
            sc = [-sc[0], -sc[1], -sc[2]];
        }
        for i in 0..3 {
            if sc[i] != 0.0 {
                // aiVector3D::operator/= multiplies by the reciprocal.
                let inv = 1.0 / sc[i];
                cols[i] = [cols[i][0] * inv, cols[i][1] * inv, cols[i][2] * inv];
            }
        }
        let r = [
            [cols[0][0], cols[1][0], cols[2][0]],
            [cols[0][1], cols[1][1], cols[2][1]],
            [cols[0][2], cols[1][2], cols[2][2]],
        ];
        (sc, AiQuat::from_matrix(&r), pos)
    }

    /// `aiMatrix4x4 * aiVector3D`.
    pub fn transform_point(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.m;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2] + m[0][3],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2] + m[1][3],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2] + m[2][3],
        ]
    }

    /// `aiMatrix3x3(aiMatrix4x4) * aiVector3D`.
    pub fn transform_dir(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.m;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    /// The matrix as a column-major array (`[col][row]`) for
    /// [`crate::Mat4`].
    pub fn to_col_major(&self) -> [[f32; 4]; 4] {
        self.transpose().m
    }

    pub fn from_col_major(c: &[[f32; 4]; 4]) -> AiMat4 {
        AiMat4 { m: *c }.transpose()
    }
}

pub fn vlen(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

pub fn vsub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn vsqlen(v: [f32; 3]) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

/// `aiVector3D::Normalize` (multiplies by the reciprocal length).
pub fn vnormalize(v: [f32; 3]) -> [f32; 3] {
    let l = vlen(v);
    if l == 0.0 {
        return v;
    }
    let inv = 1.0 / l;
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

/// `aiQuaternion` (w, x, y, z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiQuat {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for AiQuat {
    fn default() -> Self {
        AiQuat {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

impl AiQuat {
    pub fn new(w: f32, x: f32, y: f32, z: f32) -> Self {
        AiQuat { w, x, y, z }
    }

    /// `aiQuaternion(const aiMatrix3x3&)`; `r[row][col]`.
    pub fn from_matrix(r: &[[f32; 3]; 3]) -> AiQuat {
        let (a1, a2, a3) = (r[0][0], r[0][1], r[0][2]);
        let (b1, b2, b3) = (r[1][0], r[1][1], r[1][2]);
        let (c1, c2, c3) = (r[2][0], r[2][1], r[2][2]);
        let t = a1 + b2 + c3;
        if t > 0.0 {
            let s = (1.0 + t).sqrt() * 2.0;
            AiQuat {
                x: (c2 - b3) / s,
                y: (a3 - c1) / s,
                z: (b1 - a2) / s,
                w: 0.25 * s,
            }
        } else if a1 > b2 && a1 > c3 {
            let s = (1.0 + a1 - b2 - c3).sqrt() * 2.0;
            AiQuat {
                x: 0.25 * s,
                y: (b1 + a2) / s,
                z: (a3 + c1) / s,
                w: (c2 - b3) / s,
            }
        } else if b2 > c3 {
            let s = (1.0 + b2 - a1 - c3).sqrt() * 2.0;
            AiQuat {
                x: (b1 + a2) / s,
                y: 0.25 * s,
                z: (c2 + b3) / s,
                w: (a3 - c1) / s,
            }
        } else {
            let s = (1.0 + c3 - a1 - b2).sqrt() * 2.0;
            AiQuat {
                x: (a3 + c1) / s,
                y: (c2 + b3) / s,
                z: 0.25 * s,
                w: (b1 - a2) / s,
            }
        }
    }

    /// `aiQuaternion::GetMatrix`; `[row][col]`.
    pub fn get_matrix(&self) -> [[f32; 3]; 3] {
        let AiQuat { w, x, y, z } = *self;
        [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - z * w),
                2.0 * (x * z + y * w),
            ],
            [
                2.0 * (x * y + z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - x * w),
            ],
            [
                2.0 * (x * z - y * w),
                2.0 * (y * z + x * w),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ]
    }

    pub fn normalize(&self) -> AiQuat {
        let mag = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if mag != 0.0 {
            let inv = 1.0 / mag;
            AiQuat {
                x: self.x * inv,
                y: self.y * inv,
                z: self.z * inv,
                w: self.w * inv,
            }
        } else {
            *self
        }
    }

    /// `aiQuaternion::operator*`.
    pub fn mul(&self, t: &AiQuat) -> AiQuat {
        AiQuat {
            w: self.w * t.w - self.x * t.x - self.y * t.y - self.z * t.z,
            x: self.w * t.x + self.x * t.w + self.y * t.z - self.z * t.y,
            y: self.w * t.y + self.y * t.w + self.z * t.x - self.x * t.z,
            z: self.w * t.z + self.z * t.w + self.x * t.y - self.y * t.x,
        }
    }

    /// `aiQuaternion::Interpolate` (slerp with the shortest path).
    pub fn interpolate(start: &AiQuat, end: &AiQuat, factor: f32) -> AiQuat {
        let mut cosom = start.x * end.x + start.y * end.y + start.z * end.z + start.w * end.w;
        let mut end = *end;
        if cosom < 0.0 {
            cosom = -cosom;
            end = AiQuat {
                x: -end.x,
                y: -end.y,
                z: -end.z,
                w: -end.w,
            };
        }
        let (sclp, sclq);
        if 1.0 - cosom > 0.0001 {
            let omega = cosom.acos();
            let sinom = omega.sin();
            sclp = ((1.0 - factor) * omega).sin() / sinom;
            sclq = (factor * omega).sin() / sinom;
        } else {
            sclp = 1.0 - factor;
            sclq = factor;
        }
        AiQuat {
            x: sclp * start.x + sclq * end.x,
            y: sclp * start.y + sclq * end.y,
            z: sclp * start.z + sclq * end.z,
            w: sclp * start.w + sclq * end.w,
        }
    }
}
