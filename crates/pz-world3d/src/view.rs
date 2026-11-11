use crate::camera::{forward_world, right_world, Camera3D};
use crate::mesh::Face;
use crate::ViewMode;
use std::f32::consts::{PI, TAU};

/// The view mode and the free-look angles the player controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewState {
    pub mode: ViewMode,
    /// World yaw, see [`Camera3D::yaw`].
    pub yaw: f32,
    /// Radians, positive looks up.
    pub pitch: f32,
    /// Radians per mouse count.
    pub sensitivity: f32,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            mode: ViewMode::Iso,
            yaw: 0.0,
            pitch: 0.0,
            sensitivity: 0.0025,
        }
    }
}

impl ViewState {
    /// O key: Iso enters first person facing `facing_world`; any 3D mode returns to Iso.
    pub fn toggle_first_person(&mut self, facing_world: (f32, f32)) {
        match self.mode {
            ViewMode::Iso => {
                self.mode = ViewMode::FirstPerson;
                self.yaw = facing_world.1.atan2(facing_world.0);
                self.pitch = 0.0;
            }
            _ => self.mode = ViewMode::Iso,
        }
    }

    /// Shift+O: Iso enters third person facing `facing_world`; otherwise switches between first and third person.
    pub fn swap_person(&mut self, facing_world: (f32, f32)) {
        match self.mode {
            ViewMode::Iso => {
                self.mode = ViewMode::ThirdPerson;
                self.yaw = facing_world.1.atan2(facing_world.0);
                self.pitch = 0.0;
            }
            ViewMode::FirstPerson => self.mode = ViewMode::ThirdPerson,
            ViewMode::ThirdPerson => self.mode = ViewMode::FirstPerson,
        }
    }

    /// Mouse look. Right turns toward +yaw, down looks down. No-op in Iso.
    pub fn apply_mouse(&mut self, dx: f64, dy: f64) {
        if !self.is_3d() {
            return;
        }
        let limit = 89.0_f32.to_radians();
        self.yaw += dx as f32 * self.sensitivity;
        self.pitch = (self.pitch - dy as f32 * self.sensitivity).clamp(-limit, limit);
        // Wrap into (-PI, PI].
        let y = self.yaw.rem_euclid(TAU);
        self.yaw = if y > PI { y - TAU } else { y };
    }

    /// Converts the game's move vector (x right, y = -forward) to world space.
    pub fn move_world(&self, input: (f32, f32)) -> (f32, f32) {
        let (rx, ry) = right_world(self.yaw);
        let (fx, fy) = forward_world(self.yaw);
        let (x, f) = (input.0, -input.1);
        (x * rx + f * fx, x * ry + f * fy)
    }

    /// True in first or third person.
    pub fn is_3d(&self) -> bool {
        self.mode != ViewMode::Iso
    }

    /// The camera for the current mode, or None in Iso.
    pub fn camera(&self, feet_world: [f32; 3], origin_world: [f32; 3]) -> Option<Camera3D> {
        match self.mode {
            ViewMode::Iso => None,
            ViewMode::FirstPerson => Some(Camera3D::first_person(
                feet_world,
                origin_world,
                self.yaw,
                self.pitch,
            )),
            ViewMode::ThirdPerson => Some(Camera3D::third_person(
                feet_world,
                origin_world,
                self.yaw,
                self.pitch,
            )),
        }
    }

    /// Like [`camera`](Self::camera), but the third-person eye is kept out of `faces` (walls).
    pub fn camera_with(
        &self,
        feet_world: [f32; 3],
        origin_world: [f32; 3],
        faces: &[Face],
    ) -> Option<Camera3D> {
        match self.mode {
            ViewMode::ThirdPerson => Some(Camera3D::third_person_clamped(
                feet_world,
                origin_world,
                self.yaw,
                self.pitch,
                faces,
            )),
            _ => self.camera(feet_world, origin_world),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::EYE_HEIGHT;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn default_is_iso() {
        let v = ViewState::default();
        assert_eq!(v.mode, ViewMode::Iso);
        assert_eq!((v.yaw, v.pitch, v.sensitivity), (0.0, 0.0, 0.0025));
        assert!(!v.is_3d());
    }

    #[test]
    fn toggle_enters_and_leaves_first_person() {
        let mut v = ViewState {
            pitch: 0.3,
            ..Default::default()
        };
        v.toggle_first_person((0.0, -1.0));
        assert_eq!(v.mode, ViewMode::FirstPerson);
        assert!(close(v.yaw, -FRAC_PI_2));
        assert_eq!(v.pitch, 0.0);
        assert!(v.is_3d());
        v.swap_person((0.0, -1.0));
        v.toggle_first_person((1.0, 0.0));
        assert_eq!(v.mode, ViewMode::Iso);
        assert!(!v.is_3d());
    }

    #[test]
    fn swap_person_from_iso_enters_third_person_facing() {
        let mut v = ViewState {
            pitch: 0.3,
            ..Default::default()
        };
        v.swap_person((0.0, -1.0));
        assert_eq!(v.mode, ViewMode::ThirdPerson);
        assert!(close(v.yaw, -FRAC_PI_2));
        assert_eq!(v.pitch, 0.0);
    }

    #[test]
    fn swap_person_toggles_between_3d_modes() {
        let mut v = ViewState::default();
        v.toggle_first_person((1.0, 0.0));
        v.swap_person((0.0, -1.0));
        assert_eq!(v.mode, ViewMode::ThirdPerson);
        // Facing is kept when switching between first and third person.
        assert!(close(v.yaw, 0.0));
        v.swap_person((0.0, -1.0));
        assert_eq!(v.mode, ViewMode::FirstPerson);
    }

    #[test]
    fn mouse_ignored_in_iso() {
        let mut v = ViewState::default();
        v.apply_mouse(100.0, 100.0);
        assert_eq!((v.yaw, v.pitch), (0.0, 0.0));
    }

    #[test]
    fn mouse_turns_and_looks() {
        let mut v = ViewState {
            mode: ViewMode::FirstPerson,
            ..Default::default()
        };
        v.apply_mouse(100.0, 0.0);
        assert!(close(v.yaw, 0.25));
        v.apply_mouse(0.0, 100.0);
        assert!(close(v.pitch, -0.25));
        v.apply_mouse(0.0, -200.0);
        assert!(close(v.pitch, 0.25));
    }

    #[test]
    fn mouse_clamps_pitch() {
        let mut v = ViewState {
            mode: ViewMode::FirstPerson,
            ..Default::default()
        };
        v.apply_mouse(0.0, -1e6);
        assert!(close(v.pitch, 89.0_f32.to_radians()));
        v.apply_mouse(0.0, 1e6);
        assert!(close(v.pitch, -89.0_f32.to_radians()));
    }

    #[test]
    fn mouse_wraps_yaw() {
        let mut v = ViewState {
            mode: ViewMode::FirstPerson,
            yaw: 3.0,
            ..Default::default()
        };
        v.apply_mouse(100.0, 0.0);
        // 3.25 wraps to 3.25 - 2*PI, which is in (-PI, PI].
        assert!(close(v.yaw, 3.25 - TAU));
        let mut w = ViewState {
            mode: ViewMode::FirstPerson,
            yaw: -3.0,
            ..Default::default()
        };
        w.apply_mouse(-100.0, 0.0);
        assert!(close(w.yaw, -3.25 + TAU));
        assert!(w.yaw > -PI && w.yaw <= PI);
    }

    #[test]
    fn move_world_follows_yaw() {
        let mut v = ViewState {
            mode: ViewMode::FirstPerson,
            yaw: -FRAC_PI_2,
            ..Default::default()
        };
        // Facing north: forward is (0, -1), right is east (1, 0).
        let f = v.move_world((0.0, -1.0));
        assert!(close(f.0, 0.0) && close(f.1, -1.0));
        let r = v.move_world((1.0, 0.0));
        assert!(close(r.0, 1.0) && close(r.1, 0.0));
        v.yaw = 0.0;
        // Facing east: forward is +x, right is south (+y).
        let f = v.move_world((0.0, -1.0));
        assert!(close(f.0, 1.0) && close(f.1, 0.0));
        let r = v.move_world((1.0, 0.0));
        assert!(close(r.0, 0.0) && close(r.1, 1.0));
        // Length is preserved.
        let d = v.move_world((0.6, -0.8));
        assert!(close((d.0 * d.0 + d.1 * d.1).sqrt(), 1.0));
    }

    #[test]
    fn camera_none_in_iso() {
        assert!(ViewState::default().camera([0.0; 3], [0.0; 3]).is_none());
    }

    #[test]
    fn camera_first_and_third_person() {
        let mut v = ViewState::default();
        v.toggle_first_person((0.0, -1.0));
        let fp = v.camera([0.0; 3], [0.0; 3]).unwrap();
        assert!(close(fp.eye[1], EYE_HEIGHT));
        assert!(close(fp.yaw, -FRAC_PI_2));
        v.swap_person((0.0, -1.0));
        let tp = v.camera([0.0; 3], [0.0; 3]).unwrap();
        // Facing north, third person sits 3 units south (model +z is north, so -z).
        assert!(close(tp.eye[2], fp.eye[2] - 3.0));
        assert!(close(tp.eye[1], fp.eye[1] + 0.6));
    }
}
