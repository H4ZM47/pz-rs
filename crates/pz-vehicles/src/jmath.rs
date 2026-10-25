//! The parts of JOML (`org.joml`), `zombie.iso.Vector2`,
//! `zombie.core.physics.Transform` and `PZMath` the vehicle code uses, with
//! the same float operations in the same order (JOML runs without
//! `joml.useMathFma`, so `Math.fma(a, b, c)` is `a * b + c`).

/// `PZMath.fastfloor(float)`.
pub fn fastfloor(val: f32) -> i32 {
    let xi = val as i32;
    if val < xi as f32 {
        xi - 1
    } else {
        xi
    }
}

/// `PZMath.fastfloor(double)`.
pub fn fastfloor_d(val: f64) -> i32 {
    let xi = val as i32;
    if val < xi as f64 {
        xi - 1
    } else {
        xi
    }
}

/// `PZMath.clamp(float, float, float)`.
pub fn clamp(val: f32, min: f32, max: f32) -> f32 {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}

/// `PZMath.clamp(int, int, int)`.
pub fn clamp_i(val: i32, min: i32, max: i32) -> i32 {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}

/// `PZMath.clamp(double, double, double)`.
pub fn clamp_d(val: f64, min: f64, max: f64) -> f64 {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}

/// `PZMath.clamp_01`.
pub fn clamp_01(val: f32) -> f32 {
    clamp(val, 0.0, 1.0)
}

/// `PZMath.lerp(float, float, float)`.
pub fn lerp(src: f32, dest: f32, alpha: f32) -> f32 {
    src + (dest - src) * alpha
}

/// `PZMath.coorddivision`.
pub fn coorddivision(value: i32, divisor: i32) -> i32 {
    fastfloor(value as f32 / divisor as f32)
}

/// `PZMath.isLeft`.
pub fn is_left(x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
}

/// `PZMath.sign(float)`.
pub fn sign(v: f32) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

/// `PZMath.roundToNearest`.
pub fn round_to_nearest(val: f32) -> f32 {
    fastfloor(val + 0.5 * sign(val) as f32) as f32
}

/// `PZMath.roundToInt`.
pub fn round_to_int(val: f32) -> i32 {
    (round_to_nearest(val) + 1.0e-4) as i32
}

/// `Math.round(float)`: `floor(x + 0.5)` as int (saturating).
pub fn java_round_f(v: f32) -> i32 {
    if v.is_nan() {
        return 0;
    }
    (v as f64 + 0.5).floor() as i32
}

/// `Math.round(double)` as long.
pub fn java_round_d(v: f64) -> i64 {
    if v.is_nan() {
        return 0;
    }
    (v + 0.5).floor() as i64
}

/// `(float) Math.sqrt(f)` / `org.joml.Math.sqrt(float)`.
pub fn jsqrt(f: f32) -> f32 {
    (f as f64).sqrt() as f32
}

/// `org.joml.Math.invsqrt(float)`.
pub fn invsqrt(f: f32) -> f32 {
    1.0f32 / jsqrt(f)
}

/// `IsoUtils.DistanceToSquared(x1, y1, x2, y2)`.
pub fn dist_sq(x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let dx = x1 - x2;
    let dy = y1 - y2;
    dx * dx + dy * dy
}

/// `IsoUtils.DistanceTo2D`.
pub fn dist_2d(x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    jsqrt(dist_sq(x1, y1, x2, y2))
}

/// `IsoUtils.DistanceManhatten`.
pub fn dist_manhatten(x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    (x1 - x2).abs() + (y1 - y2).abs()
}

/// `PZMath.intersectLineSegments`.
#[allow(clippy::too_many_arguments)]
pub fn intersect_line_segments(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
) -> Option<(f32, f32)> {
    let d = (y4 - y3) * (x2 - x1) - (x4 - x3) * (y2 - y1);
    if d == 0.0 {
        return None;
    }
    let yd = y1 - y3;
    let xd = x1 - x3;
    let ua = ((x4 - x3) * yd - (y4 - y3) * xd) / d;
    if !(0.0..=1.0).contains(&ua) {
        return None;
    }
    let ub = ((x2 - x1) * yd - (y2 - y1) * xd) / d;
    if !(0.0..=1.0).contains(&ub) {
        return None;
    }
    Some((x1 + (x2 - x1) * ua, y1 + (y2 - y1) * ua))
}

/// `PZMath.closestPointOnLineSegment(...)`: the squared distance and the
/// point.
#[allow(clippy::too_many_arguments)]
pub fn closest_point_on_line_segment(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    px: f32,
    py: f32,
    endpoint_snap_epsilon: f64,
) -> (f64, (f32, f32)) {
    let dx = (x2 - x1) as f64;
    let dy = (y2 - y1) as f64;
    let seg = dx * dx + dy * dy;
    if seg < 1.0e-12 {
        let ddx = (px - x1) as f64;
        let ddy = (py - y1) as f64;
        return (ddx * ddx + ddy * ddy, (x1, y1));
    }
    let u = ((px - x1) as f64 * dx + (py - y1) as f64 * dy) / seg;
    if u <= endpoint_snap_epsilon {
        let ddx = (px - x1) as f64;
        let ddy = (py - y1) as f64;
        return (ddx * ddx + ddy * ddy, (x1, y1));
    }
    if u >= 1.0 - endpoint_snap_epsilon {
        let ddx = (px - x2) as f64;
        let ddy = (py - y2) as f64;
        return (ddx * ddx + ddy * ddy, (x2, y2));
    }
    let xu = x1 as f64 + u * dx;
    let yu = y1 as f64 + u * dy;
    let ddx = px as f64 - xu;
    let ddy = py as f64 - yu;
    (ddx * ddx + ddy * ddy, (xu as f32, yu as f32))
}

/// `PZMath.closestPointsOnLineSegments`.
#[allow(clippy::too_many_arguments)]
pub fn closest_points_on_line_segments(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
) -> (f64, (f32, f32), (f32, f32)) {
    let rx = x3 - x1;
    let ry = y3 - y1;
    let ux = x2 - x1;
    let uy = y2 - y1;
    let vx = x4 - x3;
    let vy = y4 - y3;
    let ru = rx * ux + ry * uy;
    let rv = rx * vx + ry * vy;
    let uu = ux * ux + uy * uy;
    let vv = vx * vx + vy * vy;
    let uv = ux * vx + uy * vy;
    let det = uu * vv - uv * uv;
    let (s, t);
    if (det as f64) < 1.0e-6 * uu as f64 * vv as f64 {
        s = clamp_01(ru / uu);
        t = 0.0;
    } else {
        s = clamp_01((ru * vv - rv * uv) / det);
        t = clamp_01((ru * uv - rv * uu) / det);
    }
    let new_s = clamp_01((t * uv + ru) / uu);
    let new_t = clamp_01((s * uv - rv) / vv);
    let p1 = (x1 + new_s * ux, y1 + new_s * uy);
    let p2 = (x3 + new_t * vx, y3 + new_t * vy);
    (dist_sq(p1.0, p1.1, p2.0, p2.1) as f64, p1, p2)
}

/// `zombie.iso.Vector2`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

impl Vector2 {
    pub fn new(x: f32, y: f32) -> Self {
        Vector2 { x, y }
    }
    /// `getLength`.
    pub fn length(&self) -> f32 {
        jsqrt(self.x * self.x + self.y * self.y)
    }
    /// `normalize`: unchanged when the length is 0.
    pub fn normalize(&mut self) -> f32 {
        let len_sq = self.x * self.x + self.y * self.y;
        if (1.0 - len_sq).abs() < 1.0e-5 {
            return 1.0;
        }
        if len_sq == 0.0 {
            self.x = 0.0;
            self.y = 0.0;
            return 0.0;
        }
        let length = jsqrt(len_sq);
        self.x /= length;
        self.y /= length;
        length
    }
    pub fn dot(&self, x: f32, y: f32) -> f32 {
        self.x * x + self.y * y
    }
}

/// `org.joml.Vector3f`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vector3f {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Vector3f { x, y, z }
    }
    pub fn set(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.x = x;
        self.y = y;
        self.z = z;
        self
    }
    pub fn add(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.x += x;
        self.y += y;
        self.z += z;
        self
    }
    pub fn sub(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.x -= x;
        self.y -= y;
        self.z -= z;
        self
    }
    pub fn mul(&mut self, s: f32) -> &mut Self {
        self.x *= s;
        self.y *= s;
        self.z *= s;
        self
    }
    pub fn length_squared(&self) -> f32 {
        self.x * self.x + (self.y * self.y + self.z * self.z)
    }
    pub fn length(&self) -> f32 {
        jsqrt(self.length_squared())
    }
    pub fn normalize(&mut self) -> &mut Self {
        let scalar = invsqrt(self.length_squared());
        self.x *= scalar;
        self.y *= scalar;
        self.z *= scalar;
        self
    }
    pub fn dot(&self, v: &Vector3f) -> f32 {
        self.x * v.x + (self.y * v.y + self.z * v.z)
    }
    pub fn dot3(&self, x: f32, y: f32, z: f32) -> f32 {
        self.x * x + (self.y * y + self.z * z)
    }
    pub fn cross(&self, v: &Vector3f) -> Vector3f {
        let rx = self.y * v.z + -self.z * v.y;
        let ry = self.z * v.x + -self.x * v.z;
        let rz = self.x * v.y + -self.y * v.x;
        Vector3f::new(rx, ry, rz)
    }
    pub fn get(&self, i: usize) -> f32 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }
    /// `distance`.
    pub fn distance(&self, v: &Vector3f) -> f32 {
        let dx = self.x - v.x;
        let dy = self.y - v.y;
        let dz = self.z - v.z;
        jsqrt(dx * dx + (dy * dy + dz * dz))
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

impl Default for Quaternionf {
    fn default() -> Self {
        Quaternionf {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        }
    }
}

/// `org.joml.Math.cosFromSin(float, float)` (no fast math).
pub fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = jsqrt(1.0 - sin * sin);
    let a = angle + 1.570_796_4;
    let two_pi = std::f32::consts::PI * 2.0;
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

/// `org.joml.Math.sin(float)`.
pub fn jsin(rad: f32) -> f32 {
    (rad as f64).sin() as f32
}

/// `org.joml.Math.atan2(float, float)`.
pub fn jatan2(y: f32, x: f32) -> f32 {
    (y as f64).atan2(x as f64) as f32
}

/// `org.joml.Math.safeAsin(float)`.
pub fn safe_asin(r: f32) -> f32 {
    if r <= -1.0 {
        -std::f32::consts::FRAC_PI_2
    } else if r >= 1.0 {
        std::f32::consts::FRAC_PI_2
    } else {
        (r as f64).asin() as f32
    }
}

impl Quaternionf {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Quaternionf { x, y, z, w }
    }
    /// `rotationXYZ`.
    pub fn rotation_xyz(angle_x: f32, angle_y: f32, angle_z: f32) -> Self {
        let sx = jsin(angle_x * 0.5);
        let cx = cos_from_sin(sx, angle_x * 0.5);
        let sy = jsin(angle_y * 0.5);
        let cy = cos_from_sin(sy, angle_y * 0.5);
        let sz = jsin(angle_z * 0.5);
        let cz = cos_from_sin(sz, angle_z * 0.5);
        let cycz = cy * cz;
        let sysz = sy * sz;
        let sycz = sy * cz;
        let cysz = cy * sz;
        Quaternionf {
            w: cx * cycz - sx * sysz,
            x: sx * cycz + cx * sysz,
            y: cx * sycz - sx * cysz,
            z: cx * cysz + sx * sycz,
        }
    }
    /// `setAngleAxis(float, x, y, z)`.
    pub fn angle_axis(angle: f32, x: f32, y: f32, z: f32) -> Self {
        let s = jsin(angle * 0.5);
        Quaternionf {
            x: x * s,
            y: y * s,
            z: z * s,
            w: cos_from_sin(s, angle * 0.5),
        }
    }
    /// `getEulerAnglesXYZ`.
    pub fn euler_angles_xyz(&self) -> Vector3f {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        Vector3f::new(
            jatan2(2.0 * (x * w - y * z), 1.0 - 2.0 * (x * x + y * y)),
            safe_asin(2.0 * (x * z + y * w)),
            jatan2(2.0 * (z * w - x * y), 1.0 - 2.0 * (y * y + z * z)),
        )
    }
}

/// `org.joml.Matrix3f` (column-major fields `mCR`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix3f {
    pub m00: f32,
    pub m01: f32,
    pub m02: f32,
    pub m10: f32,
    pub m11: f32,
    pub m12: f32,
    pub m20: f32,
    pub m21: f32,
    pub m22: f32,
}

impl Default for Matrix3f {
    fn default() -> Self {
        Matrix3f::IDENTITY
    }
}

impl Matrix3f {
    pub const IDENTITY: Matrix3f = Matrix3f {
        m00: 1.0,
        m01: 0.0,
        m02: 0.0,
        m10: 0.0,
        m11: 1.0,
        m12: 0.0,
        m20: 0.0,
        m21: 0.0,
        m22: 1.0,
    };

    /// `rotation(Quaternionfc)` / `set(Quaternionfc)`.
    pub fn from_quat(q: &Quaternionf) -> Self {
        let w2 = q.w * q.w;
        let x2 = q.x * q.x;
        let y2 = q.y * q.y;
        let z2 = q.z * q.z;
        let zw = q.z * q.w;
        let dzw = zw + zw;
        let xy = q.x * q.y;
        let dxy = xy + xy;
        let xz = q.x * q.z;
        let dxz = xz + xz;
        let yw = q.y * q.w;
        let dyw = yw + yw;
        let yz = q.y * q.z;
        let dyz = yz + yz;
        let xw = q.x * q.w;
        let dxw = xw + xw;
        Matrix3f {
            m00: w2 + x2 - z2 - y2,
            m01: dxy + dzw,
            m02: dxz - dyw,
            m10: -dzw + dxy,
            m11: y2 - z2 + w2 - x2,
            m12: dyz + dxw,
            m20: dyw + dxz,
            m21: dyz - dxw,
            m22: z2 - y2 - x2 + w2,
        }
    }

    /// `transform(Vector3f)` (`v.mul(this)`).
    pub fn transform(&self, v: &mut Vector3f) {
        let (lx, ly, lz) = (v.x, v.y, v.z);
        v.x = self.m00 * lx + (self.m10 * ly + self.m20 * lz);
        v.y = self.m01 * lx + (self.m11 * ly + self.m21 * lz);
        v.z = self.m02 * lx + (self.m12 * ly + self.m22 * lz);
    }

    /// `transpose()`.
    pub fn transposed(&self) -> Self {
        Matrix3f {
            m00: self.m00,
            m01: self.m10,
            m02: self.m20,
            m10: self.m01,
            m11: self.m11,
            m12: self.m21,
            m20: self.m02,
            m21: self.m12,
            m22: self.m22,
        }
    }

    /// `getColumn(column, dest)`.
    pub fn column(&self, column: usize) -> Vector3f {
        match column {
            0 => Vector3f::new(self.m00, self.m01, self.m02),
            1 => Vector3f::new(self.m10, self.m11, self.m12),
            _ => Vector3f::new(self.m20, self.m21, self.m22),
        }
    }

    /// `getUnnormalizedRotation` (`Quaternionf.setFromUnnormalized`).
    pub fn unnormalized_rotation(&self) -> Quaternionf {
        let lx = invsqrt(self.m00 * self.m00 + self.m01 * self.m01 + self.m02 * self.m02);
        let ly = invsqrt(self.m10 * self.m10 + self.m11 * self.m11 + self.m12 * self.m12);
        let lz = invsqrt(self.m20 * self.m20 + self.m21 * self.m21 + self.m22 * self.m22);
        let (m00, m01, m02) = (self.m00 * lx, self.m01 * lx, self.m02 * lx);
        let (m10, m11, m12) = (self.m10 * ly, self.m11 * ly, self.m12 * ly);
        let (m20, m21, m22) = (self.m20 * lz, self.m21 * lz, self.m22 * lz);
        let tr = m00 + m11 + m22;
        let (ox, oy, oz, ow);
        if tr >= 0.0 {
            let t = jsqrt(tr + 1.0);
            ow = t * 0.5;
            let t = 0.5 / t;
            ox = (m12 - m21) * t;
            oy = (m20 - m02) * t;
            oz = (m01 - m10) * t;
        } else if m00 >= m11 && m00 >= m22 {
            let t = jsqrt(m00 - (m11 + m22) + 1.0);
            ox = t * 0.5;
            let t = 0.5 / t;
            oy = (m10 + m01) * t;
            oz = (m02 + m20) * t;
            ow = (m12 - m21) * t;
        } else if m11 > m22 {
            let t = jsqrt(m11 - (m22 + m00) + 1.0);
            oy = t * 0.5;
            let t = 0.5 / t;
            oz = (m21 + m12) * t;
            ox = (m10 + m01) * t;
            ow = (m20 - m02) * t;
        } else {
            let t = jsqrt(m22 - (m00 + m11) + 1.0);
            oz = t * 0.5;
            let t = 0.5 / t;
            ox = (m02 + m20) * t;
            oy = (m21 + m12) * t;
            ow = (m01 - m10) * t;
        }
        Quaternionf::new(ox, oy, oz, ow)
    }
}

/// `zombie.core.physics.Transform`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transform {
    pub basis: Matrix3f,
    pub origin: Vector3f,
}

impl Transform {
    /// `setIdentity`.
    pub fn set_identity(&mut self) {
        self.basis = Matrix3f::IDENTITY;
        self.origin = Vector3f::default();
    }
    /// `transform(Vector3f)`.
    pub fn transform(&self, v: &mut Vector3f) {
        self.basis.transform(v);
        v.add(self.origin.x, self.origin.y, self.origin.z);
    }
    /// `inverse()`.
    pub fn inverse(&mut self) {
        self.basis = self.basis.transposed();
        self.origin = Vector3f::new(-self.origin.x, -self.origin.y, -self.origin.z);
        let b = self.basis;
        b.transform(&mut self.origin);
    }
    /// `getRotation(out)`.
    pub fn rotation(&self) -> Quaternionf {
        self.basis.unnormalized_rotation()
    }
    /// `setRotation(q)`.
    pub fn set_rotation(&mut self, q: &Quaternionf) {
        self.basis = Matrix3f::from_quat(q);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_round_trip_matches_world_helper() {
        let q = Quaternionf::new(0.01, 0.7, -0.02, 0.71);
        let t = Transform {
            basis: Matrix3f::from_quat(&q),
            origin: Vector3f::default(),
        };
        let r = t.rotation();
        let w = pz_world::save::vehicle::rotation_round_trip([q.x, q.y, q.z, q.w]);
        assert_eq!([r.x, r.y, r.z, r.w], w);
    }

    #[test]
    fn inverse_undoes_transform() {
        let q = Quaternionf::rotation_xyz(0.0, 1.2, 0.0);
        let mut t = Transform {
            basis: Matrix3f::from_quat(&q),
            origin: Vector3f::new(3.0, 1.0, -2.0),
        };
        let mut v = Vector3f::new(1.0, 2.0, 3.0);
        t.transform(&mut v);
        t.inverse();
        t.transform(&mut v);
        assert!((v.x - 1.0).abs() < 1e-5 && (v.y - 2.0).abs() < 1e-5 && (v.z - 3.0).abs() < 1e-5);
    }

    #[test]
    fn java_round() {
        assert_eq!(java_round_f(2.5), 3);
        assert_eq!(java_round_f(-2.5), -2);
        assert_eq!(round_to_int(2.4), 2);
    }
}
