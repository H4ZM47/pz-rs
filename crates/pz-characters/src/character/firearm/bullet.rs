//! [`PhysicsWorld`] over `pz_bullet::api::PzBullet` (the `bullet`
//! feature). Native exceptions become the value the call returns once the
//! exception is cleared (0, or nothing).

use pz_bullet::api::PzBullet;

use super::physics::PhysicsWorld;

/// `PzBullet` with the engine's physics step counter.
pub struct BulletPhysics<'a> {
    pub bullet: &'a mut PzBullet,
    /// `WorldSimulation.getBulletFrameNo()`.
    pub frame_no: i32,
}

macro_rules! forward {
    ($t:ty, $s:ident => $b:expr, $f:expr) => {
        impl PhysicsWorld for $t {
            #[allow(unused_variables)]
            fn frame_no(&self) -> i32 {
                let $s = self;
                $f
            }
            fn update_ballistics(&mut self, id: i32, x: f32, y: f32, z: f32) {
                let $s = self;
                $b.update_ballistics(id, x, y, z)
            }
            fn update_ballistics_aim_reticle_position(&mut self, id: i32, x: f32, y: f32, z: f32) {
                let $s = self;
                $b.update_ballistics_aim_reticle_position(id, x, y, z)
            }
            fn update_ballistics_muzzle_aim_direction(&mut self, id: i32, x: f32, y: f32, z: f32) {
                let $s = self;
                $b.update_ballistics_muzzle_aim_direction(id, x, y, z)
            }
            fn update_ballistics_aim_reticle_quaternion(&mut self, id: i32, x: f32, y: f32, z: f32, w: f32) {
                let $s = self;
                $b.update_ballistics_aim_reticle_quaternion(id, x, y, z, w)
            }
            fn set_ballistics_size(&mut self, id: i32, size: f32) {
                let $s = self;
                $b.set_ballistics_size(id, size)
            }
            fn set_ballistics_color(&mut self, id: i32, r: f32, g: f32, b: f32) {
                let $s = self;
                $b.set_ballistics_color(id, r, g, b)
            }
            fn set_ballistics_range(&mut self, id: i32, range: f32) {
                let $s = self;
                $b.set_ballistics_range(id, range)
            }
            fn get_ballistics_targets(&mut self, id: i32, range: f32, count: i32, out: &mut [f32]) -> i32 {
                let $s = self;
                $b.get_ballistics_targets(id, range, count, out).unwrap_or(0)
            }
            fn get_ballistics_targets_spread_data(
                &mut self,
                id: i32,
                range: f32,
                spread: f32,
                weight_center: f32,
                count: i32,
                max: i32,
                out: &mut [f32],
            ) -> i32 {
                let $s = self;
                $b.get_ballistics_targets_spread_data(id, range, spread, weight_center, count, max, out).unwrap_or(0)
            }
            fn get_ballistics_camera_targets(&mut self, id: i32, range: f32, count: i32, parts: bool, out: &mut [f32]) -> i32 {
                let $s = self;
                $b.get_ballistics_camera_targets(id, range, count, parts, out).unwrap_or(0)
            }
            fn remove_ballistics(&mut self, id: i32) {
                let $s = self;
                let _ = $b.remove_ballistics(id);
            }
            fn add_ballistics_target(&mut self, id: i32) -> i32 {
                let $s = self;
                $b.add_ballistics_target(id).unwrap_or(0)
            }
            fn remove_ballistics_target(&mut self, id: i32) -> i32 {
                let $s = self;
                $b.remove_ballistics_target(id)
            }
            fn set_ballistics_target_axis(&mut self, id: i32, x: f32, y: f32, z: f32) {
                let $s = self;
                $b.set_ballistics_target_axis(id, x, y, z)
            }
            fn update_ballistics_target(&mut self, id: i32, x: f32, y: f32, z: f32, qx: f32, qy: f32, qz: f32, qw: f32, b: bool) {
                let $s = self;
                let _ = $b.update_ballistics_target(id, x, y, z, qx, qy, qz, qw, b);
            }
            fn update_ballistics_target_skeleton(&mut self, id: i32, count: i32, data: &[f32]) {
                let $s = self;
                $b.update_ballistics_target_skeleton(id, count, data)
            }
            fn set_ballistics_target_adjusting_shape_scale(&mut self, x: f32, y: f32, z: f32) {
                let $s = self;
                $b.set_ballistics_target_adjusting_shape_scale(x, y, z)
            }
        }
    };
}

forward!(PzBullet, s => s, 0);
forward!(BulletPhysics<'_>, s => s.bullet, s.frame_no);
