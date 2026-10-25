//! [`VehiclePhysics`] on pz-bullet's [`PzBullet`] (the port of
//! `libPZBullet`): each native forwards to the method of the same name.

use super::{JavaCallbacks, PhysicsResult, VehiclePhysics};
use pz_bullet::api::PzBullet;

impl VehiclePhysics for PzBullet {
    fn init_world(
        &mut self,
        min_cell_x: i32,
        min_cell_y: i32,
        max_cell_x: i32,
        max_cell_y: i32,
        offset_x: i32,
        offset_y: i32,
        is_server: bool,
    ) -> PhysicsResult<()> {
        PzBullet::init_world(
            self, min_cell_x, min_cell_y, max_cell_x, max_cell_y, offset_x, offset_y, is_server,
        )
    }
    fn destroy_world(&mut self) {
        PzBullet::destroy_world(self)
    }
    fn activate_chunk_map(&mut self, i: i32, wx: i32, wy: i32, w: i32) -> PhysicsResult<()> {
        PzBullet::activate_chunk_map(self, i, wx, wy, w)
    }
    fn deactivate_chunk_map(&mut self, i: i32) -> PhysicsResult<()> {
        PzBullet::deactivate_chunk_map(self, i)
    }
    fn scroll_chunk_map(&mut self, i: i32, dir: i32) -> PhysicsResult<()> {
        PzBullet::scroll_chunk_map(self, i, dir)
    }
    fn set_chunk_min_max_level(
        &mut self,
        wx: i32,
        wy: i32,
        min: i32,
        max: i32,
    ) -> PhysicsResult<()> {
        PzBullet::set_chunk_min_max_level(self, wx, wy, min, max)
    }
    fn to_bullet(&mut self, buf: &[u8]) {
        PzBullet::to_bullet(self, buf)
    }
    fn define_vehicle_script(&mut self, name: &str, params: &[f32]) -> PhysicsResult<()> {
        PzBullet::define_vehicle_script(self, name, params)
    }
    fn define_vehicle_physics_mesh(
        &mut self,
        name: &str,
        index: i32,
        points: &[f32],
    ) -> PhysicsResult<()> {
        PzBullet::define_vehicle_physics_mesh(self, name, index, points)
    }
    fn add_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
        script: &str,
    ) -> PhysicsResult<()> {
        PzBullet::add_vehicle(self, id, x, y, z, qx, qy, qz, qw, script)
    }
    fn remove_vehicle(&mut self, id: i32) -> PhysicsResult<()> {
        PzBullet::remove_vehicle(self, id)
    }
    fn control_vehicle(
        &mut self,
        id: i32,
        engine: f32,
        brake: f32,
        steering: f32,
    ) -> PhysicsResult<()> {
        PzBullet::control_vehicle(self, id, engine, brake, steering)
    }
    fn set_vehicle_active(&mut self, id: i32, active: bool) -> PhysicsResult<()> {
        PzBullet::set_vehicle_active(self, id, active)
    }
    fn set_vehicle_static(&mut self, id: i32, is_static: bool) -> PhysicsResult<i32> {
        PzBullet::set_vehicle_static(self, id, is_static)
    }
    fn apply_central_force_to_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
    ) -> PhysicsResult<()> {
        PzBullet::apply_central_force_to_vehicle(self, id, x, y, z)
    }
    fn apply_torque_to_vehicle(&mut self, id: i32, x: f32, y: f32, z: f32) -> PhysicsResult<()> {
        PzBullet::apply_torque_to_vehicle(self, id, x, y, z)
    }
    fn teleport_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) -> PhysicsResult<()> {
        PzBullet::teleport_vehicle(self, id, x, y, z, qx, qy, qz, qw)
    }
    fn set_tire_inflation(&mut self, id: i32, wheel: i32, inflation: f32) -> PhysicsResult<()> {
        PzBullet::set_tire_inflation(self, id, wheel, inflation)
    }
    fn set_tire_removed(&mut self, id: i32, wheel: i32, removed: bool) -> PhysicsResult<()> {
        PzBullet::set_tire_removed(self, id, wheel, removed)
    }
    fn step_simulation(
        &mut self,
        time_step: f32,
        max_sub_steps: i32,
        fixed: f32,
        cb: &mut dyn JavaCallbacks,
    ) -> PhysicsResult<()> {
        PzBullet::step_simulation(self, time_step, max_sub_steps, fixed, cb)
    }
    fn get_vehicle_count(&mut self) -> PhysicsResult<i32> {
        PzBullet::get_vehicle_count(self)
    }
    fn get_vehicle_physics(&mut self, offset: i32, out: &mut [f32]) -> PhysicsResult<i32> {
        PzBullet::get_vehicle_physics(self, offset, out)
    }
    fn get_own_vehicle_physics(&mut self, id: i32, out: &mut [f32]) -> PhysicsResult<i32> {
        PzBullet::get_own_vehicle_physics(self, id, out)
    }
    fn set_own_vehicle_physics(
        &mut self,
        id: i32,
        params: &mut [f32],
        force: bool,
    ) -> PhysicsResult<i32> {
        PzBullet::set_own_vehicle_physics(self, id, params, force)
    }
    fn set_vehicle_params(&mut self, id: i32, params: &mut [f32]) -> PhysicsResult<i32> {
        PzBullet::set_vehicle_params(self, id, params)
    }
    fn set_vehicle_mass(&mut self, id: i32, mass: f32) -> PhysicsResult<i32> {
        PzBullet::set_vehicle_mass(self, id, mass)
    }
    fn get_object_physics(&mut self, out: &mut [f32]) -> PhysicsResult<i32> {
        PzBullet::get_object_physics(self, out)
    }
    fn set_vehicle_velocity_multiplier(
        &mut self,
        id: i32,
        max_speed: f32,
        multiplier: f32,
    ) -> PhysicsResult<()> {
        PzBullet::set_vehicle_velocity_multiplier(self, id, max_speed, multiplier)
    }
    fn add_6dof_constraint(&mut self, a: i32, b: i32, p: &[f32; 18]) -> PhysicsResult<i32> {
        PzBullet::add_6dof_constraint(self, a, b, p)
    }
    fn add_rope_constraint(
        &mut self,
        a: i32,
        b: i32,
        ax: f32,
        ay: f32,
        az: f32,
        bx: f32,
        by: f32,
        bz: f32,
        max_distance: f32,
    ) -> PhysicsResult<i32> {
        PzBullet::add_rope_constraint(self, a, b, ax, ay, az, bx, by, bz, max_distance)
    }
    fn set_constraint_erp(&mut self, c: i32, erp: f32, axis: i32) -> PhysicsResult<()> {
        PzBullet::set_constraint_erp(self, c, erp, axis)
    }
    fn remove_constraint(&mut self, c: i32) -> PhysicsResult<()> {
        PzBullet::remove_constraint(self, c)
    }
}
