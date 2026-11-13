use crate::mesh::Face;
use crate::{to_model, Mat4};

/// Eye height above the feet in model units (tunable).
pub const EYE_HEIGHT: f32 = 1.35;

/// A perspective camera in model space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera3D {
    /// Eye position, model space.
    pub eye: [f32; 3],
    /// World yaw: 0 looks east (+x), increasing turns toward south (+y).
    pub yaw: f32,
    /// Radians, positive looks up.
    pub pitch: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

/// Horizontal forward direction in world space for `yaw`.
pub fn forward_world(yaw: f32) -> (f32, f32) {
    (yaw.cos(), yaw.sin())
}

/// Horizontal right direction in world space for `yaw` (facing east, right is south).
pub fn right_world(yaw: f32) -> (f32, f32) {
    (-yaw.sin(), yaw.cos())
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = dot(v, v).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add_scaled(a: [f32; 3], d: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + d[0] * t, a[1] + d[1] * t, a[2] + d[2] * t]
}

/// Möller–Trumbore ray/triangle test, both sides. Returns the ray parameter `t` (> 0).
fn ray_triangle(o: [f32; 3], d: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<f32> {
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-8 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(o, a);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t > 1e-6).then_some(t)
}

/// Gap kept between the eye and the nearest face, in model units.
const WALL_MARGIN: f32 = 0.2;
/// The eye never comes closer than this to the pivot, in model units.
const MIN_EYE_DIST: f32 = 0.3;
/// The segment starts this far from the pivot, to avoid self-hits.
const PIVOT_OFFSET: f32 = 0.05;

/// Pulls a third-person eye in toward `pivot` (the first-person eye) so the segment pivot->eye does not pass through any face, leaving a 0.2 unit margin; never closer than 0.3 units to the pivot.
pub fn clamp_eye(pivot: [f32; 3], eye: [f32; 3], faces: &[Face]) -> [f32; 3] {
    let delta = sub(eye, pivot);
    let len = dot(delta, delta).sqrt();
    if len < 1e-6 {
        return eye;
    }
    let dir = [delta[0] / len, delta[1] / len, delta[2] / len];
    // Ray parameter limit, measured from the offset start point.
    let max_t = len - PIVOT_OFFSET;
    if max_t <= 0.0 {
        return eye;
    }
    let origin = add_scaled(pivot, dir, PIVOT_OFFSET);
    let mut nearest: Option<f32> = None;
    for face in faces {
        let p = face.pos;
        for [i, j, k] in [[0, 1, 2], [0, 2, 3]] {
            if let Some(t) = ray_triangle(origin, dir, p[i], p[j], p[k]) {
                if t <= max_t && nearest.map_or(true, |n| t < n) {
                    nearest = Some(t);
                }
            }
        }
    }
    match nearest {
        None => eye,
        Some(t) => {
            let hit_dist = PIVOT_OFFSET + t;
            let dist = (hit_dist - WALL_MARGIN).max(MIN_EYE_DIST).min(len);
            add_scaled(pivot, dir, dist)
        }
    }
}

/// How close the first-person eye may come to a wall, in model units.
pub const FP_WALL_GAP: f32 = 0.22;

/// Pushes an eye out of any wall (a vertical, axis-aligned face) that is
/// closer than `gap`, so standing against a wall never puts the near plane
/// inside it. Diagonal cards and flat faces are ignored.
pub fn keep_eye_off_walls(eye: [f32; 3], faces: &[Face], gap: f32) -> [f32; 3] {
    let mut e = eye;
    // Two passes settle corners, where two walls push in turn.
    for _ in 0..2 {
        for face in faces {
            let p = face.pos;
            let n = cross(sub(p[1], p[0]), sub(p[3], p[0]));
            let len = dot(n, n).sqrt();
            if len < 1e-6 {
                continue;
            }
            let n = [n[0] / len, n[1] / len, n[2] / len];
            // Walls only: horizontal normal along model x or z.
            let axis = if n[0].abs() > 0.99 {
                0
            } else if n[2].abs() > 0.99 {
                2
            } else {
                continue;
            };
            let other = 2 - axis;
            let (lo_o, hi_o) = span(&p, other);
            let (lo_y, hi_y) = span(&p, 1);
            if e[other] < lo_o - gap * 0.5
                || e[other] > hi_o + gap * 0.5
                || e[1] < lo_y
                || e[1] > hi_y
            {
                continue;
            }
            let d = e[axis] - p[0][axis];
            if d.abs() < gap {
                let side = if d >= 0.0 { 1.0 } else { -1.0 };
                e[axis] = p[0][axis] + side * gap;
            }
        }
    }
    e
}

fn span(p: &[[f32; 3]; 4], k: usize) -> (f32, f32) {
    let lo = p.iter().map(|v| v[k]).fold(f32::INFINITY, f32::min);
    let hi = p.iter().map(|v| v[k]).fold(f32::NEG_INFINITY, f32::max);
    (lo, hi)
}

impl Camera3D {
    /// Unit look direction in model space, including pitch.
    pub fn forward_model(&self) -> [f32; 3] {
        let (fx, fy) = forward_world(self.yaw);
        let (sp, cp) = self.pitch.sin_cos();
        // World (fx, fy) maps to model (-fx, 0, -fy); pitch lifts along +Y.
        normalize([-fx * cp, sp, -fy * cp])
    }

    /// Right-handed look-at matrix (gluLookAt), up = +Y.
    pub fn view(&self) -> Mat4 {
        let e = self.eye;
        let f = self.forward_model();
        let s = normalize(cross(f, [0.0, 1.0, 0.0]));
        let u = cross(s, f);
        Mat4([
            [s[0], u[0], -f[0], 0.0],
            [s[1], u[1], -f[1], 0.0],
            [s[2], u[2], -f[2], 0.0],
            [-dot(s, e), -dot(u, e), dot(f, e), 1.0],
        ])
    }

    /// GL perspective projection (depth -1..1), as gluPerspective.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        let f = 1.0 / (self.fov_y * 0.5).tan();
        let (n, far) = (self.near, self.far);
        let mut m = [[0.0f32; 4]; 4];
        m[0][0] = f / aspect;
        m[1][1] = f;
        m[2][2] = (far + n) / (n - far);
        m[2][3] = -1.0;
        m[3][2] = 2.0 * far * n / (n - far);
        Mat4(m)
    }

    /// The model-space ray through normalized device point (`ndc_x`,
    /// `ndc_y`) (-1..1, y up) of a view with this `aspect`: the eye and a
    /// unit direction.
    pub fn ray(&self, aspect: f32, ndc_x: f32, ndc_y: f32) -> ([f32; 3], [f32; 3]) {
        let f = self.forward_model();
        let s = normalize(cross(f, [0.0, 1.0, 0.0]));
        let u = cross(s, f);
        let t = (self.fov_y * 0.5).tan();
        let (kx, ky) = (ndc_x * t * aspect, ndc_y * t);
        let d = [
            f[0] + s[0] * kx + u[0] * ky,
            f[1] + s[1] * kx + u[1] * ky,
            f[2] + s[2] * kx + u[2] * ky,
        ];
        (self.eye, normalize(d))
    }

    /// The nearest face point along the ray through (`ndc_x`, `ndc_y`),
    /// in model space, within the far plane.
    pub fn pick(&self, aspect: f32, ndc_x: f32, ndc_y: f32, faces: &[Face]) -> Option<[f32; 3]> {
        let (o, d) = self.ray(aspect, ndc_x, ndc_y);
        let mut best: Option<f32> = None;
        for face in faces {
            let p = face.pos;
            for [i, j, k] in [[0, 1, 2], [0, 2, 3]] {
                if let Some(t) = ray_triangle(o, d, p[i], p[j], p[k]) {
                    if t >= self.near && t <= self.far && best.map_or(true, |b| t < b) {
                        best = Some(t);
                    }
                }
            }
        }
        best.map(|t| add_scaled(o, d, t))
    }

    /// `projection * view`.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect).mul_mat(&self.view())
    }

    /// First-person camera with eyes `EYE_HEIGHT` above the feet.
    pub fn first_person(
        feet_world: [f32; 3],
        origin_world: [f32; 3],
        yaw: f32,
        pitch: f32,
    ) -> Camera3D {
        let p = to_model(feet_world, origin_world);
        Camera3D {
            eye: [p[0], p[1] + EYE_HEIGHT, p[2]],
            yaw,
            pitch,
            fov_y: 70.0_f32.to_radians(),
            near: 0.05,
            far: 120.0,
        }
    }

    /// Third-person camera: 3 units behind the first-person eye (horizontal), 0.6 higher.
    pub fn third_person(
        feet_world: [f32; 3],
        origin_world: [f32; 3],
        yaw: f32,
        pitch: f32,
    ) -> Camera3D {
        let mut c = Camera3D::first_person(feet_world, origin_world, yaw, pitch);
        let (fx, fy) = forward_world(yaw);
        // Behind means against the horizontal look direction; model x and z flip.
        c.eye = [c.eye[0] + 3.0 * fx, c.eye[1] + 0.6, c.eye[2] + 3.0 * fy];
        c
    }

    /// First-person camera whose eye is kept [`FP_WALL_GAP`] off walls.
    pub fn first_person_clamped(
        feet_world: [f32; 3],
        origin_world: [f32; 3],
        yaw: f32,
        pitch: f32,
        faces: &[Face],
    ) -> Camera3D {
        let mut c = Camera3D::first_person(feet_world, origin_world, yaw, pitch);
        c.eye = keep_eye_off_walls(c.eye, faces, FP_WALL_GAP);
        c
    }

    /// Third-person camera whose eye is pulled in by [`clamp_eye`] so walls do not block the view.
    pub fn third_person_clamped(
        feet_world: [f32; 3],
        origin_world: [f32; 3],
        yaw: f32,
        pitch: f32,
        faces: &[Face],
    ) -> Camera3D {
        let pivot = Camera3D::first_person_clamped(feet_world, origin_world, yaw, pitch, faces).eye;
        let mut c = Camera3D::third_person(feet_world, origin_world, yaw, pitch);
        c.eye = clamp_eye(pivot, c.eye, faces);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    const ORIGIN: [f32; 3] = [0.0, 0.0, 0.0];

    fn ndc(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
        let c = m.transform(p);
        [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
    }

    fn north_cam() -> Camera3D {
        Camera3D::first_person(ORIGIN, ORIGIN, -FRAC_PI_2, 0.0)
    }

    #[test]
    fn world_directions() {
        let (fx, fy) = forward_world(-FRAC_PI_2);
        assert!(fx.abs() < 1e-6 && (fy + 1.0).abs() < 1e-6);
        let (rx, ry) = right_world(-FRAC_PI_2);
        assert!((rx - 1.0).abs() < 1e-6 && ry.abs() < 1e-6);
        let (fx, fy) = forward_world(0.0);
        assert!((fx - 1.0).abs() < 1e-6 && fy.abs() < 1e-6);
    }

    #[test]
    fn forward_model_pitch_and_axes() {
        let f = north_cam().forward_model();
        assert!(f[0].abs() < 1e-6 && f[1].abs() < 1e-6 && (f[2] - 1.0).abs() < 1e-6);
        let up = Camera3D {
            pitch: 0.5,
            ..north_cam()
        }
        .forward_model();
        assert!(up[1] > 0.0);
        assert!((dot(up, up) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn east_is_on_the_right_when_facing_north() {
        let m = north_cam().view_proj(1.0);
        let east = ndc(&m, to_model([1.0, -5.0, 0.0], ORIGIN));
        let west = ndc(&m, to_model([-1.0, -5.0, 0.0], ORIGIN));
        assert!(east[0] > 0.0);
        assert!(west[0] < 0.0);
    }

    #[test]
    fn point_ahead_is_centred() {
        let m = north_cam().view_proj(1.0);
        let p = ndc(&m, [0.0, EYE_HEIGHT, 5.0]);
        assert!(p[0].abs() < 1e-5 && p[1].abs() < 1e-5);
        assert!(p[2] > -1.0 && p[2] < 1.0);
    }

    #[test]
    fn point_above_eye_projects_up() {
        let m = north_cam().view_proj(1.0);
        let p = ndc(&m, [0.0, EYE_HEIGHT + 1.0, 5.0]);
        assert!(p[1] > 0.0);
    }

    #[test]
    fn point_behind_has_negative_w() {
        let m = north_cam().view_proj(1.0);
        let c = m.transform([0.0, EYE_HEIGHT, -5.0]);
        assert!(c[3] < 0.0);
    }

    #[test]
    fn first_person_eye_height() {
        let c = Camera3D::first_person([3.0, 4.0, 1.0], ORIGIN, 0.0, 0.0);
        let p = to_model([3.0, 4.0, 1.0], ORIGIN);
        assert_eq!(c.eye, [p[0], p[1] + EYE_HEIGHT, p[2]]);
        assert_eq!(c.far, 120.0);
    }

    #[test]
    fn third_person_is_behind_and_above() {
        let fp = Camera3D::first_person(ORIGIN, ORIGIN, -FRAC_PI_2, 0.0);
        let tp = Camera3D::third_person(ORIGIN, ORIGIN, -FRAC_PI_2, 0.0);
        // Facing north, behind is south, which is model -z.
        assert!((tp.eye[2] - (fp.eye[2] - 3.0)).abs() < 1e-5);
        assert!((tp.eye[1] - (fp.eye[1] + 0.6)).abs() < 1e-5);
        assert!((tp.eye[0] - fp.eye[0]).abs() < 1e-5);
    }

    fn quad_at_z(z: f32) -> Face {
        Face {
            texture: pz_render::TextureManager::default().white(),
            pos: [
                [-1.0, -1.0, z],
                [1.0, -1.0, z],
                [1.0, 1.0, z],
                [-1.0, 1.0, z],
            ],
            uv: [[0.0; 2]; 4],
            color: [0xFFFF_FFFF; 4],
            translucent: false,
        }
    }

    #[test]
    fn centre_ray_is_the_look_direction_and_picks_the_nearest_face() {
        let c = Camera3D {
            eye: [0.0, 0.0, 0.0],
            ..north_cam()
        };
        let (o, d) = c.ray(1.5, 0.0, 0.0);
        assert_eq!(o, c.eye);
        let f = c.forward_model();
        assert!((0..3).all(|i| (d[i] - f[i]).abs() < 1e-6));
        // North is model +z: walls at z = 2 and z = 4, the nearer wins.
        let hit = c.pick(1.5, 0.0, 0.0, &[quad_at_z(4.0), quad_at_z(2.0)]).unwrap();
        assert!((hit[2] - 2.0).abs() < 1e-5, "{hit:?}");
        // Nothing behind the eye is picked.
        assert!(c.pick(1.5, 0.0, 0.0, &[quad_at_z(-2.0)]).is_none());
    }

    #[test]
    fn edge_ray_spans_half_the_field_of_view() {
        let c = north_cam();
        let (_, d) = c.ray(1.0, 0.0, 1.0);
        let up = d[1].atan2(d[2]);
        assert!((up - c.fov_y * 0.5).abs() < 1e-5, "{up}");
    }

    #[test]
    fn eye_is_pushed_off_a_close_wall() {
        // Wall at z = -0.1, eye on the +z side: pushed to z = -0.1 + gap.
        let e = keep_eye_off_walls([0.0, 0.0, 0.0], &[quad_at_z(-0.1)], 0.22);
        assert!((e[2] - 0.12).abs() < 1e-5, "{e:?}");
        // From the other side it is pushed the other way.
        let e = keep_eye_off_walls([0.0, 0.0, -0.15], &[quad_at_z(-0.1)], 0.22);
        assert!((e[2] + 0.32).abs() < 1e-5, "{e:?}");
    }

    #[test]
    fn far_or_offset_walls_leave_the_eye_alone() {
        let eye = [0.0, 0.0, 0.0];
        assert_eq!(keep_eye_off_walls(eye, &[quad_at_z(-0.5)], 0.22), eye);
        // Beside the wall's end, not in front of it.
        let eye = [3.0, 0.0, 0.0];
        assert_eq!(keep_eye_off_walls(eye, &[quad_at_z(-0.1)], 0.22), eye);
        // Above the wall's top.
        let eye = [0.0, 2.0, 0.0];
        assert_eq!(keep_eye_off_walls(eye, &[quad_at_z(-0.1)], 0.22), eye);
    }

    #[test]
    fn first_person_clamped_without_faces_matches_first_person() {
        let a = Camera3D::first_person([0.0; 3], [0.0; 3], 0.3, 0.1);
        let b = Camera3D::first_person_clamped([0.0; 3], [0.0; 3], 0.3, 0.1, &[]);
        assert_eq!(a.eye, b.eye);
    }

    #[test]
    fn clamp_eye_pulls_in_before_wall() {
        let pivot = [0.0, 0.0, 0.0];
        let eye = [0.0, 0.0, -5.0];
        // Wall at distance 3 along the segment.
        let clamped = clamp_eye(pivot, eye, &[quad_at_z(-3.0)]);
        let d = dot(sub(clamped, pivot), sub(clamped, pivot)).sqrt();
        assert!(d < 3.0 && d > 3.0 - 0.3, "distance {d}");
        // Still on the same ray.
        assert!(clamped[0].abs() < 1e-6 && clamped[1].abs() < 1e-6);
    }

    #[test]
    fn clamp_eye_without_faces_is_unchanged() {
        let eye = [0.5, 1.0, -5.0];
        assert_eq!(clamp_eye([0.0; 3], eye, &[]), eye);
    }

    #[test]
    fn clamp_eye_ignores_face_behind_pivot() {
        let eye = [0.0, 0.0, -5.0];
        let behind = [quad_at_z(2.0)];
        assert_eq!(clamp_eye([0.0; 3], eye, &behind), eye);
    }

    #[test]
    fn clamp_eye_never_closer_than_min() {
        let pivot = [0.0, 0.0, 0.0];
        let eye = [0.0, 0.0, -5.0];
        // Wall at 0.4: the margin would put the eye at 0.2, so it stops at 0.3.
        let clamped = clamp_eye(pivot, eye, &[quad_at_z(-0.4)]);
        let d = dot(sub(clamped, pivot), sub(clamped, pivot)).sqrt();
        assert!((d - 0.3).abs() < 1e-5, "distance {d}");
    }

    #[test]
    fn third_person_clamped_without_faces_matches_third_person() {
        let a = Camera3D::third_person([0.0; 3], [0.0; 3], -FRAC_PI_2, 0.0);
        let b = Camera3D::third_person_clamped([0.0; 3], [0.0; 3], -FRAC_PI_2, 0.0, &[]);
        assert_eq!(a, b);
    }
}
