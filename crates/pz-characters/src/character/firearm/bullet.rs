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
            fn initialize_ragdoll_skeleton(&mut self, n: i32, hierarchy: &[i32]) {
                let $s = self;
                $b.initialize_ragdoll_skeleton(n, hierarchy)
            }
            fn initialize_ragdoll_pose(&mut self, n: i32, data: &[f32], q: [f32; 4]) {
                let $s = self;
                let _ = $b.initialize_ragdoll_pose(n, data, q[0], q[1], q[2], q[3]);
            }
            fn set_ragdoll_mass(&mut self, mass: f32) {
                let $s = self;
                $b.set_ragdoll_mass(mass)
            }
            fn add_ragdoll(&mut self, id: i32, p: [f32; 3], q: [f32; 4]) {
                let $s = self;
                let _ = $b.add_ragdoll(id, p[0], p[1], p[2], q[0], q[1], q[2], q[3]);
            }
            fn remove_ragdoll(&mut self, id: i32) {
                let $s = self;
                let _ = $b.remove_ragdoll(id);
            }
            fn update_ragdoll(&mut self, id: i32, p: [f32; 3], q: [f32; 4]) {
                let $s = self;
                $b.update_ragdoll(id, p[0], p[1], p[2], q[0], q[1], q[2], q[3])
            }
            fn set_ragdoll_active(&mut self, id: i32, active: bool) {
                let $s = self;
                $b.set_ragdoll_active(id, active)
            }
            fn set_ragdoll_local_transform_rotation(&mut self, id: i32, q: [f32; 4]) {
                let $s = self;
                $b.set_ragdoll_local_transform_rotation(id, q[0], q[1], q[2], q[3])
            }
            fn update_ragdoll_skeleton_transforms(&mut self, id: i32, n: i32, bones: &[f32]) {
                let $s = self;
                $b.update_ragdoll_skeleton_transforms(id, n, bones)
            }
            fn update_ragdoll_skeleton_previous_transforms(&mut self, id: i32, n: i32, dt: f32, bones: &[f32]) {
                let $s = self;
                $b.update_ragdoll_skeleton_previous_transforms(id, n, dt, bones)
            }
            fn simulate_ragdoll_with_rigid_body_output(&mut self, id: i32, skeleton: &mut [f32], rigid_bodies: &mut [f32]) -> i32 {
                let $s = self;
                $b.simulate_ragdoll_with_rigid_body_output(id, skeleton, rigid_bodies).unwrap_or(-1)
            }
            fn get_ragdoll_simulation_state(&mut self, id: i32) -> i32 {
                let $s = self;
                $b.get_ragdoll_simulation_state(id)
            }
            fn apply_impulse(&mut self, id: i32, part: i32, impulse: &[f32]) {
                let $s = self;
                $b.apply_impulse(id, part, impulse)
            }
            fn set_ragdoll_body_dynamics(&mut self, id: i32, params: &[f32]) {
                let $s = self;
                let _ = $b.set_ragdoll_body_dynamics(id, Some(params));
            }
            fn reset_ragdoll_body_dynamics(&mut self, id: i32) {
                let $s = self;
                $b.reset_ragdoll_body_dynamics(id);
            }
        }
    };
}

forward!(PzBullet, s => s, 0);
forward!(BulletPhysics<'_>, s => s.bullet, s.frame_no);
