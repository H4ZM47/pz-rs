//! `TileDepthTexture.clampPixelToUpperFloor` and the projection maths behind
//! it (`TileGeometryUtils.getNormalizedDepthOnPlaneAt` on the XZ plane).
//!
//! The clamp table is the normalized depth of the plane one storey up, seen
//! through the tile's isometric camera, plus a small push towards the
//! viewer. The original computes it with JOML; the matrix and vector steps
//! here follow the JOML code paths the original takes (`setOrtho`,
//! `translate`, `rotationXYZ`, `mulAffine`, `invertAffine`, `unprojectInv`,
//! `mulPosition`), all in `f32` with JOML's operation order and without fused
//! multiply-adds (JOML's `Math.fma` is a plain `a * b + c` unless
//! `joml.useMathFma` is set, which the game does not do), so the table is
//! bit-identical.

#![allow(clippy::needless_range_loop)] // index loops mirror JOML's m<col><row>

use std::sync::OnceLock;

use crate::iso::TILE_SCALE;

/// Width of the clamp table: one 2x tile.
const CLAMP_WIDTH: usize = 128;
/// Rows of the clamp table; only the top half of a tile is clamped.
pub const CLAMP_ROWS: usize = 64;
/// `top` in `initClampedPixels`: height of the upper floor plane.
const UPPER_FLOOR_Y: f32 = 2.44949;
/// `pushTopDown` in `initClampedPixels` (3/255).
const PUSH_TOP_DOWN: f32 = 0.011764706;
/// The miss value of the `getDepthOn*At` helpers.
const NO_HIT: f32 = 666.0;

/// A JOML `Matrix4f`, column-major (`m[col][row]` is JOML's `m<col><row>`).
#[derive(Clone, Copy, Debug)]
struct Mat4 {
    m: [[f32; 4]; 4],
}

#[derive(Clone, Copy, Debug, Default)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Vec3 { x, y, z }
    }

    /// `Vector3f.dot`.
    fn dot(self, v: Vec3) -> f32 {
        self.x * v.x + (self.y * v.y + self.z * v.z)
    }

    fn sub(self, v: Vec3) -> Vec3 {
        Vec3::new(self.x - v.x, self.y - v.y, self.z - v.z)
    }

    fn add(self, v: Vec3) -> Vec3 {
        Vec3::new(self.x + v.x, self.y + v.y, self.z + v.z)
    }

    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }

    /// `Vector3f.normalize`.
    fn normalize(self) -> Vec3 {
        let scalar = 1.0 / (self.x * self.x + (self.y * self.y + self.z * self.z)).sqrt();
        self.mul(scalar)
    }
}

/// JOML `Math.sin(float)`: `java.lang.Math.sin` in double precision.
fn sin(rad: f32) -> f32 {
    (rad as f64).sin() as f32
}

/// JOML `Math.cosFromSin(float, float)` (`cosFromSinInternal`).
fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = ((1.0f32 - sin * sin) as f64).sqrt() as f32;
    let two_pi = std::f64::consts::PI as f32 * 2.0;
    let a = angle + 1.570_796_4;
    let mut b = a - ((a / two_pi) as i32) as f32 * two_pi;
    if (b as f64) < 0.0 {
        b += two_pi;
    }
    if b >= std::f64::consts::PI as f32 {
        -cos
    } else {
        cos
    }
}

impl Mat4 {
    const IDENTITY: Mat4 = Mat4 { m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]] };

    /// `setOrtho(left, right, bottom, top, zNear, zFar)` (GL depth range).
    fn ortho(left: f32, right: f32, bottom: f32, top: f32, z_near: f32, z_far: f32) -> Mat4 {
        let mut r = Mat4::IDENTITY;
        r.m[0][0] = 2.0 / (right - left);
        r.m[1][1] = 2.0 / (top - bottom);
        r.m[2][2] = 2.0 / (z_near - z_far);
        r.m[3][0] = (right + left) / (left - right);
        r.m[3][1] = (top + bottom) / (bottom - top);
        r.m[3][2] = (z_far + z_near) / (z_near - z_far);
        r
    }

    /// `translate(x, y, z)` on a non-identity matrix (`translateGeneric`).
    fn translate(mut self, x: f32, y: f32, z: f32) -> Mat4 {
        let m = self.m;
        for row in 0..4 {
            self.m[3][row] = m[0][row] * x + (m[1][row] * y + (m[2][row] * z + m[3][row]));
        }
        self
    }

    /// `rotationXYZ(angleX, angleY, angleZ)`.
    fn rotation_xyz(angle_x: f32, angle_y: f32, angle_z: f32) -> Mat4 {
        let sin_x = sin(angle_x);
        let cos_x = cos_from_sin(sin_x, angle_x);
        let sin_y = sin(angle_y);
        let cos_y = cos_from_sin(sin_y, angle_y);
        let sin_z = sin(angle_z);
        let cos_z = cos_from_sin(sin_z, angle_z);
        let mut r = Mat4::IDENTITY;
        let nm01 = -sin_x * -sin_y;
        let nm02 = cos_x * -sin_y;
        r.m[2][0] = sin_y;
        r.m[2][1] = -sin_x * cos_y;
        r.m[2][2] = cos_x * cos_y;
        r.m[0][0] = cos_y * cos_z;
        r.m[0][1] = nm01 * cos_z + cos_x * sin_z;
        r.m[0][2] = nm02 * cos_z + sin_x * sin_z;
        r.m[1][0] = cos_y * -sin_z;
        r.m[1][1] = nm01 * -sin_z + cos_x * cos_z;
        r.m[1][2] = nm02 * -sin_z + sin_x * cos_z;
        r
    }

    /// `mul(right)` with both operands affine (`mulAffine`).
    fn mul_affine(&self, r: &Mat4) -> Mat4 {
        let (a, b) = (&self.m, &r.m);
        let mut o = Mat4::IDENTITY;
        for col in 0..3 {
            for row in 0..3 {
                o.m[col][row] = a[0][row] * b[col][0] + (a[1][row] * b[col][1] + a[2][row] * b[col][2]);
            }
        }
        for row in 0..3 {
            o.m[3][row] = a[0][row] * b[3][0] + (a[1][row] * b[3][1] + (a[2][row] * b[3][2] + a[3][row]));
        }
        o.m[0][3] = a[0][3];
        o.m[1][3] = a[1][3];
        o.m[2][3] = a[2][3];
        o.m[3][3] = a[3][3];
        o
    }

    /// `invertAffine()`.
    fn invert_affine(&self) -> Mat4 {
        let [[m00, m01, m02, _], [m10, m11, m12, _], [m20, m21, m22, _], [m30, m31, m32, _]] = self.m;
        let m11m00 = m00 * m11;
        let m10m01 = m01 * m10;
        let m10m02 = m02 * m10;
        let m12m00 = m00 * m12;
        let m12m01 = m01 * m12;
        let m11m02 = m02 * m11;
        let det = (m11m00 - m10m01) * m22 + (m10m02 - m12m00) * m21 + (m12m01 - m11m02) * m20;
        let s = 1.0 / det;
        let m10m22 = m10 * m22;
        let m10m21 = m10 * m21;
        let m11m22 = m11 * m22;
        let m11m20 = m11 * m20;
        let m12m21 = m12 * m21;
        let m12m20 = m12 * m20;
        let m20m02 = m20 * m02;
        let m20m01 = m20 * m01;
        let m21m02 = m21 * m02;
        let m21m00 = m21 * m00;
        let m22m01 = m22 * m01;
        let m22m00 = m22 * m00;
        let nm31 = (m20m02 * m31 - m20m01 * m32 + m21m00 * m32 - m21m02 * m30 + m22m01 * m30 - m22m00 * m31) * s;
        let nm32 = (m11m02 * m30 - m12m01 * m30 + m12m00 * m31 - m10m02 * m31 + m10m01 * m32 - m11m00 * m32) * s;
        let nm30 = (m10m22 * m31 - m10m21 * m32 + m11m20 * m32 - m11m22 * m30 + m12m21 * m30 - m12m20 * m31) * s;
        Mat4 {
            m: [
                [(m11m22 - m12m21) * s, (m21m02 - m22m01) * s, (m12m01 - m11m02) * s, 0.0],
                [(m12m20 - m10m22) * s, (m22m00 - m20m02) * s, (m10m02 - m12m00) * s, 0.0],
                [(m10m21 - m11m20) * s, (m20m01 - m21m00) * s, (m11m00 - m10m01) * s, 0.0],
                [nm30, nm31, nm32, 1.0],
            ],
        }
    }

    /// `unprojectInv(winX, winY, winZ, viewport)` on an inverted matrix.
    fn unproject_inv(&self, win_x: f32, win_y: f32, win_z: f32, viewport: [i32; 4]) -> Vec3 {
        let m = &self.m;
        let ndc_x = (win_x - viewport[0] as f32) / viewport[2] as f32 * 2.0 - 1.0;
        let ndc_y = (win_y - viewport[1] as f32) / viewport[3] as f32 * 2.0 - 1.0;
        let ndc_z = win_z + win_z - 1.0;
        let inv_w = 1.0 / (m[0][3] * ndc_x + m[1][3] * ndc_y + m[2][3] * ndc_z + m[3][3]);
        Vec3::new(
            (m[0][0] * ndc_x + m[1][0] * ndc_y + m[2][0] * ndc_z + m[3][0]) * inv_w,
            (m[0][1] * ndc_x + m[1][1] * ndc_y + m[2][1] * ndc_z + m[3][1]) * inv_w,
            (m[0][2] * ndc_x + m[1][2] * ndc_y + m[2][2] * ndc_z + m[3][2]) * inv_w,
        )
    }

    /// `transformPosition(v).z` (`Vector3f.mulPosition`).
    fn transform_position_z(&self, v: Vec3) -> f32 {
        let m = &self.m;
        m[0][2] * v.x + (m[1][2] * v.y + (m[2][2] * v.z + m[3][2]))
    }
}

/// `TileGeometryUtils.calcMatricesForSquare`: the projection and model-view
/// of one tile (2 units wide, 4 tall, 30/315 degree isometric view).
fn matrices_for_square() -> (Mat4, Mat4) {
    let s = 2.0f64.sqrt() as f32;
    // `-1 * s` and `1 * s` are exact.
    let projection = Mat4::ortho(-s / 2.0, s / 2.0, -2.0 * s / 2.0, 2.0 * s / 2.0, -2.0, 2.0).translate(0.0, -2.0 * s * 0.375, 0.0);
    let deg = std::f64::consts::PI as f32 / 180.0;
    let model_view = Mat4::rotation_xyz(30.0 * deg, 315.0 * deg, 0.0 * deg);
    (projection, model_view)
}

/// `TileGeometryUtils.getDepthOfScenePoint`.
fn depth_of_scene_point(p: Vec3) -> f32 {
    let (projection, model_view) = matrices_for_square();
    projection.mul_affine(&model_view).transform_position_z(p)
}

/// `TileGeometryUtils.getNormalizedDepth`: scene depth mapped so the
/// tile's north-west corner on the floor is 0.5 and its centre 0.75.
fn normalized_depth(depth: f32) -> f32 {
    let depth_nw = depth_of_scene_point(Vec3::new(-0.5, 0.0, -0.5)).abs();
    let mut scale = 1.0 / depth_nw;
    scale *= 0.25;
    depth * scale + 0.75
}

/// `UI3DScene.intersect_ray_plane`: the hit point when the ray's first 100
/// units cross the plane.
fn intersect_ray_plane(point: Vec3, normal: Vec3, origin: Vec3, direction: Vec3) -> Option<Vec3> {
    let u = direction.mul(100.0);
    let w = origin.sub(point);
    let d = normal.dot(u);
    let n = -normal.dot(w);
    if d.abs() < 1.0e-8 {
        return None;
    }
    let s = n / d;
    #[allow(clippy::manual_range_contains)] // NaN passes, as in the original
    if s < 0.0 || s > 1.0 {
        return None;
    }
    Some(origin.add(u.mul(s)))
}

/// `TileGeometryUtils.getDepthOnPlaneAt(tileX, tileY, planePoint,
/// planeNormal)`: depth of the plane under tile pixel (`tile_x`, `tile_y`),
/// or 666 when the camera ray misses it.
fn depth_on_plane_at(tile_x: f32, tile_y: f32, point: Vec3, normal: Vec3) -> f32 {
    let (projection, model_view) = matrices_for_square();
    let tile_x = tile_x * (1.0 / (64.0 * TILE_SCALE as f32));
    let tile_y = tile_y * (2.0 / (128.0 * TILE_SCALE as f32));
    // `getCameraRay(uiX, uiY, projection, modelView, 1, 2)`.
    let inverse = projection.mul_affine(&model_view).invert_affine();
    let viewport = [0, 0, 1, 2];
    let start = inverse.unproject_inv(tile_x, 2.0 - tile_y, 0.0, viewport);
    let end = inverse.unproject_inv(tile_x, 2.0 - tile_y, 1.0, viewport);
    let direction = end.sub(start).normalize();
    match intersect_ray_plane(point, normal, start, direction) {
        Some(scene) => projection.mul_affine(&model_view).transform_position_z(scene),
        None => NO_HIT,
    }
}

/// `getNormalizedDepthOnPlaneAt(tileX, tileY, GridPlane.XZ, planePoint)`:
/// -1 where the plane is not hit.
fn normalized_depth_on_xz_plane_at(tile_x: f32, tile_y: f32, point: Vec3) -> f32 {
    let depth = depth_on_plane_at(tile_x, tile_y, point, Vec3::new(0.0, 1.0, 0.0));
    if depth == NO_HIT {
        -1.0
    } else {
        normalized_depth(depth)
    }
}

/// `TileDepthTexture.s_clampedPixels` (`initClampedPixels`): 128 x 64 values,
/// row-major.
pub fn upper_floor_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let plane_point = Vec3::new(0.0, UPPER_FLOOR_Y, 0.0);
        let mut t = vec![0.0f32; CLAMP_WIDTH * CLAMP_ROWS];
        for y in 0..CLAMP_ROWS {
            for x in 0..CLAMP_WIDTH {
                let mut pixel = normalized_depth_on_xz_plane_at(x as f32 + 0.5, y as f32 + 0.5, plane_point);
                if pixel >= 0.0 {
                    pixel += PUSH_TOP_DOWN;
                }
                t[x + y * CLAMP_WIDTH] = pixel;
            }
        }
        t
    })
}

/// `clampPixelToUpperFloor(x, y, pixel)`: keeps a depth value from reaching
/// in front of the floor above. Only valid for `y < 64` and `x < 128`.
pub fn clamp_pixel_to_upper_floor(x: usize, y: usize, pixel: f32) -> f32 {
    let c = upper_floor_table()[x + y * CLAMP_WIDTH];
    // `PZMath.max`.
    if pixel > c {
        pixel
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_depth_anchors() {
        // The tile centre on the floor is the middle of the 0.5..1 floor
        // range; the north-west corner is its far end (1), south-east the
        // near one.
        assert_eq!(normalized_depth(depth_of_scene_point(Vec3::new(0.0, 0.0, 0.0))), 0.75);
        let nw = normalized_depth(depth_of_scene_point(Vec3::new(-0.5, 0.0, -0.5)));
        assert!((nw - 1.0).abs() < 1e-6, "{nw}");
        let se = normalized_depth(depth_of_scene_point(Vec3::new(0.5, 0.0, 0.5)));
        assert!((se - 0.5).abs() < 1e-6, "{se}");
    }

    #[test]
    fn affine_inverse_round_trips() {
        let (p, mv) = matrices_for_square();
        let m = p.mul_affine(&mv);
        let i = m.invert_affine().mul_affine(&m);
        for c in 0..4 {
            for r in 0..4 {
                let want = if c == r { 1.0 } else { 0.0 };
                assert!((i.m[c][r] - want).abs() < 1e-5, "{c}{r}: {}", i.m[c][r]);
            }
        }
    }

    #[test]
    fn floor_plane_under_tile_centre() {
        // The floor diamond fills the bottom 64 rows of a 128x256 tile, so
        // its centre is drawn at (64, 224): the ray there meets the floor at
        // depth 0.75, its top corner (64, 192) at the far end.
        let d = normalized_depth_on_xz_plane_at(64.0, 224.0, Vec3::new(0.0, 0.0, 0.0));
        assert!((d - 0.75).abs() < 1e-5, "{d}");
        let d = normalized_depth_on_xz_plane_at(64.0, 192.0, Vec3::new(0.0, 0.0, 0.0));
        assert!((d - 1.0).abs() < 1e-5, "{d}");
    }

    #[test]
    fn clamp_table_shape() {
        let t = upper_floor_table();
        assert_eq!(t.len(), 128 * 64);
        // Every value is either a miss (-1) or a pushed normalized depth.
        assert!(t.iter().all(|&v| v == -1.0 || v >= PUSH_TOP_DOWN - 1.0));
        // The upper floor's diamond covers the middle of the top rows.
        assert!(t[64 + 32 * 128] > 0.0);
        // Clamping only ever raises a value.
        for (i, &c) in t.iter().enumerate() {
            let (x, y) = (i % 128, i / 128);
            assert_eq!(clamp_pixel_to_upper_floor(x, y, 2.0), 2.0);
            assert_eq!(clamp_pixel_to_upper_floor(x, y, -5.0), if c > -5.0 { c } else { -5.0 });
        }
    }
}
