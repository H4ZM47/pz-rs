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
}
