//! `zombie.core.physics.CarController`: driver input to engine force,
//! braking force and steering, gears and engine speed, sent to the physics
//! with `Bullet.controlVehicle`.
//!
//! The controller's data lives in the vehicle's state
//! ([`crate::vehicle::VehicleState::ctrl`]); its methods are vehicle
//! methods so that they never hold a borrow while calling back into the
//! vehicle.

use crate::engine::{EngineState, Transmission};
use crate::env;
use crate::jmath::{self, fastfloor, Vector2};
use crate::physics::Bullet;
use crate::rand;
use crate::vehicle::{BaseVehicle, LEVEL_HEIGHT};

/// `CarController.ClientControls`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientControls {
    pub steering: f32,
    pub forward: bool,
    pub backward: bool,
    pub brake: bool,
    pub shift: bool,
    pub was_using_parking_brakes: bool,
    pub force_brake: i64,
}

impl ClientControls {
    pub fn reset(&mut self) {
        *self = ClientControls::default();
    }
}

/// `CarController` (its fields).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarController {
    pub client_force: f32,
    pub engine_force: f32,
    pub braking_force: f32,
    pub vehicle_steering: f32,
    pub is_gas: bool,
    pub is_gas_r: bool,
    pub is_break: bool,
    pub at_rest_timer: f32,
    pub regulator_timer: f32,
    pub is_enable: bool,
    pub accelerator_on: bool,
    pub brake_on: bool,
    pub speed: f32,
    pub client_controls: ClientControls,
    pub engine_starting_from_keyboard: bool,
    pub drunk_delay_command_timer: f32,
    pub was_breaking: bool,
    pub was_gas: bool,
    pub was_gas_r: bool,
    pub was_steering: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ControlState {
    NoControl,
    Braking,
    Forward,
    Reverse,
}

impl CarController {
    /// `new CarController(vehicle)`: the physics height of a vehicle without
    /// a saved one, and `Bullet.addVehicle`.
    pub fn new(v: &BaseVehicle) -> CarController {
        let script = v.script().expect("CarController without a script");
        let s = script.borrow();
        let mut physics_z = v.st().saved_physics_z;
        if physics_z.is_nan() {
            let mut wheel_bottom = 0.0f32;
            if s.wheel_count() > 0 {
                let mo = s.model_offset().unwrap_or_default();
                wheel_bottom += mo.y;
                wheel_bottom += s.wheel(0).offset.y - s.wheel(0).radius;
            }
            let chassis_bottom = s.center_of_mass_offset().y - s.extents().y / 2.0;
            let level = (fastfloor(v.z()) * 3) as f32 * 0.816_496_7;
            physics_z = level - wheel_bottom.min(chassis_bottom);
            if s.wheel_count() == 0 {
                physics_z = physics_z.max(level + 0.1);
            }
            v.st_mut().jni_transform.origin.y = physics_z;
        }
        let full = s.full_name();
        drop(s);
        let (id, x, y, q) = {
            let st = v.st();
            (st.vehicle_id as i32, st.x, st.y, st.saved_rot)
        };
        Bullet::with(|b| b.add_vehicle(id, x, y, physics_z, q.x, q.y, q.z, q.w, &full));
        CarController {
            client_force: 0.0,
            engine_force: 0.0,
            braking_force: 0.0,
            vehicle_steering: 0.0,
            is_gas: false,
            is_gas_r: false,
            is_break: false,
            at_rest_timer: -1.0,
            regulator_timer: 0.0,
            is_enable: false,
            accelerator_on: false,
            brake_on: false,
            speed: 0.0,
            client_controls: ClientControls::default(),
            engine_starting_from_keyboard: false,
            drunk_delay_command_timer: 0.0,
            was_breaking: false,
            was_gas: false,
            was_gas_r: false,
            was_steering: false,
        }
    }
}

impl BaseVehicle {
    fn ctrl(&self) -> CarController {
        self.st().ctrl.expect("vehicle without CarController")
    }

    fn ctrl_mut<R>(&self, f: impl FnOnce(&mut CarController) -> R) -> R {
        f(self
            .st_mut()
            .ctrl
            .as_mut()
            .expect("vehicle without CarController"))
    }

    /// `getController().clientControls` (a copy).
    pub fn client_controls(&self) -> Option<ClientControls> {
        self.st().ctrl.map(|c| c.client_controls)
    }

    /// Sets the driver input (what `updateControls` reads from the keyboard
    /// or joypad).
    pub fn set_client_controls(&self, controls: ClientControls) {
        if let Some(c) = self.st_mut().ctrl.as_mut() {
            c.client_controls = controls;
        }
    }

    /// `updatePhysics()` = `CarController.update()`.
    pub fn update_physics(&self) {
        if self.st().ctrl.is_some() {
            self.controller_update();
        }
    }

    /// `CarController.update()`.
    pub fn controller_update(&self) {
        if self.vehicle_towed_by().is_some() {
            return;
        }
        let Some(script) = self.script() else { return };
        let speed_kmh = self.current_speed_kmh();
        self.ctrl_mut(|c| c.speed = speed_kmh);
        let drunk = self.driver().is_some_and(|d| d.drunk_level() > 1);
        let mut dot = 0.0f32;
        let mut velocity = self.linear_velocity();
        velocity.y = 0.0;
        if (velocity.length() as f64) > 0.5 {
            velocity.normalize();
            let fwd = self.forward_vector();
            dot = velocity.dot(&fwd);
        }
        let limit_mod = 1.0f32;
        let speed_limited = self.current_speed_kmh() * limit_mod;
        let cc = self.ctrl().client_controls;
        let (mut is_gas, mut is_gas_r, mut is_break) = (false, false, false);
        if cc.forward {
            if dot < 0.0 {
                is_break = true;
            }
            if dot >= 0.0 {
                is_gas = true;
            }
            is_gas_r = false;
        }
        if cc.backward {
            if dot > 0.0 {
                is_break = true;
            }
            if dot <= 0.0 {
                is_gas_r = true;
            }
            is_gas = false;
        }
        if cc.brake {
            is_break = true;
            is_gas = false;
            is_gas_r = false;
        }
        if cc.forward && cc.backward {
            is_break = true;
            is_gas = false;
            is_gas_r = false;
        }
        self.ctrl_mut(|c| {
            c.is_gas = is_gas;
            c.is_gas_r = is_gas_r;
            c.is_break = is_break;
        });
        if drunk && self.engine_state() != EngineState::Idle {
            let c = self.ctrl();
            if c.is_break && !c.was_breaking {
                let r = self.delay_command_while_drunk();
                self.ctrl_mut(|c| c.is_break = r);
            }
            let c = self.ctrl();
            if c.is_gas && !c.was_gas {
                let r = self.delay_command_while_drunk();
                self.ctrl_mut(|c| c.is_gas = r);
            }
            let c = self.ctrl();
            if c.is_gas_r && !c.was_gas_r {
                let r = self.delay_command_while_drunk();
                self.ctrl_mut(|c| c.is_gas_r = r);
            }
            let c = self.ctrl();
            if c.client_controls.steering != 0.0 && !c.was_steering {
                let ok = self.delay_command_while_drunk();
                self.ctrl_mut(|c| {
                    if !ok {
                        c.client_controls.steering = 0.0;
                    }
                });
            }
        }
        self.update_regulator();
        self.ctrl_mut(|c| {
            c.was_breaking = c.is_break;
            c.was_gas = c.is_gas;
            c.was_gas_r = c.is_gas_r;
            c.was_steering = c.client_controls.steering != 0.0;
        });
        let c = self.ctrl();
        if !c.is_gas_r && self.is_invalid_chunk_ahead() {
            self.ctrl_mut(|c| {
                c.is_break = true;
                c.is_gas = false;
                c.is_gas_r = false;
            });
        } else if !c.is_gas && self.is_invalid_chunk_behind() {
            self.ctrl_mut(|c| {
                c.is_break = true;
                c.is_gas = false;
                c.is_gas_r = false;
            });
        }
        if self.ctrl().client_controls.shift {
            self.ctrl_mut(|c| {
                c.is_gas = false;
                c.is_break = false;
                c.is_gas_r = false;
                c.client_controls.was_using_parking_brakes = false;
            });
        }
        let t = env::time();
        let mut throttle = self.st().throttle;
        let c = self.ctrl();
        if c.is_gas || c.is_gas_r {
            throttle += t.multiplier / 30.0;
        } else {
            throttle -= t.multiplier / 30.0;
        }
        throttle = throttle.clamp(0.0, 1.0).max(0.0);
        if self.is_regulator() && !c.is_gas && !c.is_gas_r {
            throttle = 0.5;
            if speed_limited < self.regulator_speed() {
                self.ctrl_mut(|c| c.is_gas = true);
            }
        }
        self.st_mut().throttle = throttle;
        let fps_mod = t.multiplier / 0.8;
        let c = self.ctrl();
        let state = if c.is_break {
            ControlState::Braking
        } else if c.is_gas && !c.is_gas_r {
            ControlState::Forward
        } else if !c.is_gas && c.is_gas_r {
            ControlState::Reverse
        } else {
            ControlState::NoControl
        };
        if state != ControlState::NoControl {
            env::events().event("SetCurrentGameSpeed 1");
        }
        match state {
            ControlState::NoControl => self.control_no_control(),
            ControlState::Reverse => self.control_reverse(speed_limited),
            ControlState::Forward => self.control_forward_new(speed_limited),
            ControlState::Braking => {}
        }
        self.update_back_signal();
        if state == ControlState::Braking {
            self.control_braking();
        }
        self.update_brake_lights();
        self.update_ramming_sound(speed_limited);
        let c = self.ctrl();
        let mut steering = c.vehicle_steering;
        if c.client_controls.steering.abs() > 0.1 {
            let mut delta = 1.0 - c.speed / self.max_speed();
            if delta < 0.1 {
                delta = 0.1;
            }
            steering -= (c.client_controls.steering + steering) * 0.06 * fps_mod * delta;
        } else if (steering.abs() as f64) <= 0.04 {
            steering = 0.0;
        } else if steering > 0.0 {
            steering -= 0.04 * fps_mod;
            steering = steering.max(0.0);
        } else {
            steering += 0.04 * fps_mod;
            steering = steering.min(0.0);
        }
        let clamp = script.borrow().steering_clamp(c.speed);
        steering = jmath::clamp(steering, -clamp, clamp);
        self.ctrl_mut(|c| c.vehicle_steering = steering);
        self.check_tire();
        if self.is_doing_offroad() {
            let mut n = self.transmission_number();
            if n <= 0 {
                n = 1;
            }
            let mult1 = if self.vehicle_towing().is_none() {
                0.6
            } else {
                0.8
            };
            let eff = script.borrow().offroad_efficiency();
            let mult = jmath::lerp(0.0, 1.0, 1.0 - (n - 1).clamp(0, 7) as f32 / 15.0) * mult1 * eff;
            self.ctrl_mut(|c| c.engine_force *= mult);
        }
        let c = self.ctrl();
        self.set_current_steering(c.vehicle_steering);
        self.set_braking(c.is_break);
        self.controller_check_should_be_active();
        let c = self.ctrl();
        let id = self.id() as i32;
        Bullet::with(|b| {
            b.control_vehicle(id, c.engine_force, c.braking_force, c.vehicle_steering)
        });
        if c.engine_force > 0.0
            && self.engine_state() == EngineState::Idle
            && !c.engine_starting_from_keyboard
        {
            self.ctrl_mut(|c| c.engine_starting_from_keyboard = true);
            let have_key = self
                .driver()
                .and_then(|d| d.have_key(self.key_id()))
                .is_some();
            self.try_start_engine(have_key);
        }
        let c = self.ctrl();
        if c.engine_starting_from_keyboard && c.engine_force == 0.0 {
            self.ctrl_mut(|c| c.engine_starting_from_keyboard = false);
        }
        if self.engine_state() != EngineState::Running {
            self.ctrl_mut(|c| c.accelerator_on = false);
            let wheels = script.borrow().wheel_count();
            if self.current_speed_kmh() > 5.0 && wheels > 0 {
                let c = self.ctrl();
                Bullet::with(|b| b.control_vehicle(id, 0.0, c.braking_force, c.vehicle_steering));
            } else {
                self.controller_park();
            }
        }
    }

    /// `CarController.updateTrailer()`.
    pub fn controller_update_trailer(&self) {
        let Some(by) = self.vehicle_towed_by() else {
            return;
        };
        let speed = self.current_speed_kmh();
        self.ctrl_mut(|c| {
            c.speed = speed;
            c.is_gas = false;
            c.is_gas_r = false;
            c.is_break = false;
            c.was_gas = false;
            c.was_gas_r = false;
            c.was_breaking = false;
        });
        self.st_mut().throttle = 0.0;
        if by.driver().is_none() && self.driver().is_some() {
            env::events().event("addPointConstraint");
            return;
        }
        self.controller_check_should_be_active();
        let towed_force = by.st().ctrl.map_or(0.0, |c| c.engine_force);
        self.ctrl_mut(|c| {
            c.engine_force = if towed_force > 0.0 { 1.0e-4 } else { 0.0 };
            c.braking_force = 0.0;
            c.vehicle_steering = 0.0;
        });
        if !self.is_burnt() && self.engine_part().is_some() {
            self.steer_in_direction_of_towing();
        }
        let steering = self.ctrl().vehicle_steering;
        self.set_current_steering(steering);
        if !self.script_name().contains("Trailer") {
            self.ctrl_mut(|c| c.braking_force = 10.0);
        }
        let c = self.ctrl();
        let id = self.id() as i32;
        Bullet::with(|b| {
            b.control_vehicle(id, c.engine_force, c.braking_force, c.vehicle_steering)
        });
    }

    /// `steerInDirectionOfTowing()`.
    fn steer_in_direction_of_towing(&self) {
        let Some(by) = self.vehicle_towed_by() else {
            return;
        };
        let att = self.st().tow_attachment_self.clone().unwrap_or_default();
        let Some(p) = self.attachment_world_pos(&att) else {
            return;
        };
        let mut dir = Vector2::new(by.x() - p.x, by.y() - p.y);
        dir.normalize();
        let f = self.forward_vector();
        let mut fwd = Vector2::new(f.x, f.z);
        if self.is_being_towed_backwards() {
            fwd.x *= -1.0;
            fwd.y *= -1.0;
        }
        let angle_between = {
            let d = dir.x * fwd.x + dir.y * fwd.y;
            let len = dir.length() * fwd.length();
            let c = jmath::clamp(d / len, -1.0, 1.0);
            ((c as f64).acos()) as f32
        };
        let mut angle = jmath::clamp(angle_between * 57.295776 / 2.0, 0.0, 45.0) / 45.0;
        let (p0x, p0y) = (self.x(), self.y());
        if jmath::is_left(p0x, p0y, p0x + f.x, p0y + f.z, by.x(), by.y()) > 0.0 {
            angle *= -1.0;
        }
        self.ctrl_mut(|c| c.vehicle_steering = angle);
    }

    /// `updateRegulator()`.
    fn update_regulator(&self) {
        let t = env::time();
        if self.ctrl().regulator_timer > 0.0 {
            self.ctrl_mut(|c| c.regulator_timer -= t.thirty_fps_multiplier);
        }
        let c = self.ctrl();
        let cc = c.client_controls;
        if cc.shift {
            if cc.forward && c.regulator_timer <= 0.0 {
                let rs = self.regulator_speed();
                if rs < self.max_speed() + 20.0
                    && ((!self.is_regulator() && rs == 0.0) || self.is_regulator())
                {
                    let cur = self.current_speed_for_regulator();
                    if rs == 0.0 && cur != rs {
                        self.set_regulator_speed(cur);
                    } else {
                        self.set_regulator_speed(rs + 5.0);
                    }
                }
                self.set_regulator(true);
                self.ctrl_mut(|c| c.regulator_timer = 20.0);
            } else if cc.backward && c.regulator_timer <= 0.0 {
                self.ctrl_mut(|c| c.regulator_timer = 20.0);
                let rs = self.regulator_speed();
                if rs >= 5.0 && ((!self.is_regulator() && rs == 0.0) || self.is_regulator()) {
                    self.set_regulator_speed(rs - 5.0);
                }
                self.set_regulator(true);
                if self.regulator_speed() <= 0.0 {
                    self.set_regulator_speed(0.0);
                    self.set_regulator(false);
                }
            }
        } else if c.is_gas_r || c.is_break {
            self.set_regulator(false);
        }
    }

    /// `control_NoControl()`.
    fn control_no_control(&self) {
        if self.engine_part().is_none() {
            self.set_transmission(Transmission::N);
            self.ctrl_mut(|c| {
                c.engine_force = 0.0;
                c.braking_force = 10.0;
            });
            return;
        }
        let fps = env::time().multiplier / 0.8;
        if !self.is_engine_running() {
            if self.engine_speed() > 0.0 {
                self.set_engine_speed((self.engine_speed() - (50.0 * fps) as f64).max(0.0));
            }
        } else if self.engine_speed() > self.script().unwrap().borrow().engine_idle_speed() as f64 {
            if !self.is_regulator() {
                self.add_engine_speed((-20.0 * fps) as f64);
            }
        } else {
            self.add_engine_speed((20.0 * fps) as f64);
        }
        if !self.is_regulator() {
            self.set_transmission(Transmission::N);
        }
        let es = self.engine_speed();
        self.ctrl_mut(|c| {
            c.engine_force = 0.0;
            c.braking_force = if es > 1000.0 { 15.0 } else { 10.0 };
        });
    }

    /// `control_Braking()`.
    fn control_braking(&self) {
        let fps = env::time().multiplier / 0.8;
        let idle = self.script().unwrap().borrow().engine_idle_speed() as f64;
        if self.engine_speed() > idle {
            self.add_engine_speed((-(rand::next_range(10, 30)) as f32 * fps) as f64);
        } else {
            self.add_engine_speed((rand::next(20) as f32 * fps) as f64);
        }
        self.set_transmission(Transmission::N);
        let bf = self.braking_force();
        self.ctrl_mut(|c| {
            c.engine_force = 0.0;
            c.braking_force = bf;
            if c.client_controls.brake {
                c.braking_force *= 13.0;
            }
        });
    }

    /// `control_ForwardNew(speed)`.
    fn control_forward_new(&self, speed: f32) {
        let fps = env::time().multiplier / 0.8;
        let driver = self.driver();
        let fast = driver.as_ref().is_some_and(|d| d.has_trait("SPEED_DEMON"));
        let slow = driver
            .as_ref()
            .is_some_and(|d| d.has_trait("SUNDAY_DRIVER"));
        let gear_count = self.script().unwrap().borrow().gear_ratio_count;
        let rpm = self.engine_rpm_data();
        let max_speed = self.max_speed();
        let speed_per_gear = max_speed / gear_count as f32;
        let speed_clamped = jmath::clamp(speed, 0.0, max_speed);
        let mut gear_for_speed = fastfloor(speed_clamped / speed_per_gear) + 1;
        gear_for_speed = gear_for_speed.min(gear_count);
        let rpm_at = |i: i32| rpm.get(i as usize).copied().flatten().unwrap_or_default();
        let engine_speed_calc = rpm_at(gear_for_speed - 1).gear_change;
        let target = if (1..=8).contains(&gear_for_speed) {
            Transmission::from_index(gear_for_speed)
        } else {
            Transmission::Speed1
        };
        let tn = self.transmission();
        if tn == Transmission::N {
            self.set_transmission(target);
        } else if tn.index() > 0
            && tn.index() < target.index()
            && self.engine_speed() >= rpm_at(tn.index() - 1).gear_change as f64
            && speed >= speed_per_gear * tn.index() as f32
        {
            self.set_transmission(target);
            let after = rpm_at(target.index() - 1).after_gear_change;
            self.set_engine_speed(after as f64);
        }
        let tn = self.transmission();
        if tn.index() < gear_count && tn.index() > 0 {
            let cap = (rpm_at(tn.index() - 1).gear_change + 100.0) as f64;
            self.set_engine_speed(self.engine_speed().min(cap));
        }
        if self.engine_speed() > engine_speed_calc as f64 {
            self.add_engine_speed(
                -(0.5 * (self.engine_speed() - engine_speed_calc as f64)).min(10.0) * fps as f64,
            );
        } else {
            let inc = match tn {
                Transmission::Speed1 => 10.0f32,
                Transmission::Speed2 => 8.0,
                Transmission::Speed3 => 7.0,
                Transmission::Speed4 => 6.0,
                Transmission::Speed5 => 5.0,
                _ => 4.0,
            };
            self.add_engine_speed((inc * fps) as f64);
        }
        let mut power = self.engine_power() as f32;
        power *= if tn == Transmission::Speed1 { 1.5 } else { 1.0 };
        let mut force = (power as f64 * (0.3f32 as f64 + self.engine_speed() / 30000.0)) as f32;
        force -= force * (speed / 200.0);
        if speed < 15.0 {
            if let Some(t) = self.vehicle_towing() {
                if t.is_burnt() {
                    force *= 1.2f32.min(t.mass() / 500.0);
                    force *= 2.0;
                }
            }
        }
        if self.engine_speed() > 6000.0 {
            force *= (7000.0 - self.engine_speed() as f32) / 1000.0;
        }
        if slow {
            force *= 0.75;
            if speed > max_speed * 0.6 {
                force *= (max_speed * 0.75 + 20.0 - speed) / 20.0;
            }
        }
        if fast {
            if speed > max_speed * 1.15 {
                force *= (max_speed * 1.15 + 20.0 - speed) / 20.0;
            }
        } else if speed > max_speed {
            force *= (max_speed + 20.0 - speed) / 20.0;
        }
        self.ctrl_mut(|c| {
            c.engine_force = force;
            c.braking_force = 0.0;
            if c.client_controls.was_using_parking_brakes {
                c.client_controls.was_using_parking_brakes = false;
                c.engine_force *= 8.0;
            }
        });
    }

    /// `control_Reverse(speed)`.
    fn control_reverse(&self, speed: f32) {
        let fps = env::time().multiplier / 0.8;
        let driver = self.driver();
        let fast = driver.as_ref().is_some_and(|d| d.has_trait("SPEED_DEMON"));
        let slow = driver
            .as_ref()
            .is_some_and(|d| d.has_trait("SUNDAY_DRIVER"));
        self.set_transmission(Transmission::R);
        let speed = speed * 1.5;
        let calc = 1000.0 * speed / 30.0;
        self.add_engine_speed(-(0.5 * (self.engine_speed() - calc as f64)).min(100.0) * fps as f64);
        let k = if fast { 0.06 } else { 0.02 };
        self.add_engine_speed(
            -(k * (self.engine_speed() - 7000.0)).min((30.0 - speed) as f64) * fps as f64,
        );
        let mut force =
            (-(self.engine_power() as f32) as f64 * (0.75 + self.engine_speed() / 24000.0)) as f32;
        if self.engine_speed() > 6000.0 {
            force = (force as f64 * ((7000.0 - self.engine_speed()) / 1000.0)) as f32;
        }
        if slow {
            force *= 0.7;
            if speed < -5.0 {
                force *= (15.0 + speed) / 10.0;
            }
        }
        if speed < -self.script().unwrap().borrow().max_speed_reverse {
            force = 0.0;
        }
        self.ctrl_mut(|c| {
            c.engine_force = force;
            c.braking_force = 0.0;
        });
    }

    /// `updateRammingSound(speed)`.
    fn update_ramming_sound(&self, speed: f32) {
        let idle = self.script().unwrap().borrow().engine_idle_speed();
        let ef = self.ctrl().engine_force;
        if self.is_engine_running()
            && ((speed < 1.0 && ef > idle * 2.0) || (speed > -0.5 && ef < idle * -2.0))
        {
            if self.st().ram_sound == 0 {
                let h = env::events().play_sound("VehicleSkid");
                self.st_mut().ram_sound = h;
                let t = env::now_ms() + 1000 + rand::next(2000) as i64;
                self.st_mut().ram_sound_time = t;
            }
            if self.st().ram_sound != 0 && self.st().ram_sound_time < env::now_ms() {
                self.st_mut().ram_sound = 0;
            }
        } else if self.st().ram_sound != 0 {
            self.st_mut().ram_sound = 0;
        }
    }

    /// `updateBackSignal()`.
    fn update_back_signal(&self) {
        let c = self.ctrl();
        let has = self
            .script()
            .is_some_and(|s| s.borrow().sounds().back_signal_enable);
        if c.is_gas_r && self.is_engine_running() && has && !self.st().sound_back_move_on {
            self.st_mut().sound_back_move_on = true;
        }
        if !c.is_gas_r && self.st().sound_back_move_on {
            self.st_mut().sound_back_move_on = false;
        }
    }

    /// `updateBrakeLights()`.
    fn update_brake_lights(&self) {
        let b = self.ctrl().is_break;
        if b != self.stoplights_on() {
            self.set_stoplights_on(b);
        }
    }

    /// `delayCommandWhileDrunk`.
    fn delay_command_while_drunk(&self) -> bool {
        let m = env::time().multiplier;
        self.ctrl_mut(|c| c.drunk_delay_command_timer += m);
        let level = self.driver().map_or(0, |d| d.drunk_level());
        let t = self.ctrl().drunk_delay_command_timer;
        if (rand::adjust_for_framerate(4 * level) as f32) < t {
            self.ctrl_mut(|c| c.drunk_delay_command_timer = 0.0);
            return true;
        }
        false
    }

    /// `checkTire(bv)`.
    fn check_tire(&self) {
        let missing = |id: &str| self.part_by_id(id).is_none_or(|p| !p.has_item());
        let (fl, fr, rl, rr) = (
            missing("TireFrontLeft"),
            missing("TireFrontRight"),
            missing("TireRearLeft"),
            missing("TireRearRight"),
        );
        self.ctrl_mut(|c| {
            for (m, d) in [(fl, 1.2f64), (fr, 1.2), (rl, 1.3), (rr, 1.3)] {
                if m {
                    c.braking_force = (c.braking_force as f64 / d) as f32;
                    c.engine_force = (c.engine_force as f64 / d) as f32;
                }
            }
        });
    }

    /// `park()`.
    pub fn controller_park(&self) {
        if self.st().ctrl.is_none() {
            return;
        }
        if self.script().is_some_and(|s| s.borrow().wheel_count() > 0) {
            let id = self.id() as i32;
            let bf = self.braking_force();
            Bullet::with(|b| b.control_vehicle(id, 0.0, bf, 0.0));
        }
        self.ctrl_mut(|c| {
            c.was_gas = false;
            c.is_gas = false;
            c.was_gas_r = false;
            c.is_gas_r = false;
            c.client_controls.reset();
        });
        self.set_transmission(Transmission::N);
        if let Some(t) = self.vehicle_towing() {
            t.controller_park();
        }
    }

    /// `shouldBeActive()`.
    fn should_be_active(&self) -> bool {
        if self.st().physic_active_check != -1 {
            return true;
        }
        if !self.is_at_rest() {
            return true;
        }
        if self.is_player_driven_vehicle_nearby() {
            return true;
        }
        match self.vehicle_towed_by() {
            None => {
                let ef = if self.is_engine_running() {
                    self.ctrl().engine_force
                } else {
                    0.0
                };
                ef.abs() > 0.01
            }
            Some(by) => by.st().ctrl.is_some() && by.should_be_active(),
        }
    }

    /// `checkShouldBeActive()`.
    pub fn controller_check_should_be_active(&self) {
        if self.should_be_active() {
            if !self.ctrl().is_enable {
                self.set_physics_active(true, true);
            }
            self.ctrl_mut(|c| c.at_rest_timer = 1.0);
        } else if self.ctrl().is_enable && self.is_at_rest() {
            let td = env::time().time_delta;
            if self.ctrl().at_rest_timer > 0.0 {
                self.ctrl_mut(|c| c.at_rest_timer -= td);
            }
            if self.ctrl().at_rest_timer <= 0.0 {
                self.set_physics_active(false, true);
            }
        }
    }

    /// `isPlayerDrivenVehicleNearby()`.
    fn is_player_driven_vehicle_nearby(&self) -> bool {
        let cx = jmath::coorddivision(self.xi(), 8);
        let cy = jmath::coorddivision(self.yi(), 8);
        for v in crate::manager::cell_vehicles() {
            if std::ptr::eq(v.as_ref(), self) || v.driver().is_none() {
                continue;
            }
            let Some((wx, wy)) = v.st().chunk else {
                continue;
            };
            if (wx - cx).abs() > 1 || (wy - cy).abs() > 1 {
                continue;
            }
            let (d, _, _) = self.closest_point_on_poly_vehicle(&v);
            if d < 9.0 {
                return true;
            }
        }
        false
    }

    /// `isInvalidChunkAround(w, e, n, s)`.
    fn is_invalid_chunk_around(&self, w: bool, e: bool, n: bool, s: bool) -> bool {
        let Some((wx, wy)) = self.st().chunk else {
            return false;
        };
        let world = env::world();
        let null = |x: i32, y: i32| world.is_valid_chunk(x, y) && !world.chunk_loaded(x, y);
        let gw = world.chunk_grid_width();
        if gw > 7 {
            if e && (null(wx + 1, wy) || null(wx + 2, wy)) {
                return true;
            }
            if w && (null(wx - 1, wy) || null(wx - 2, wy)) {
                return true;
            }
            if s && (null(wx, wy + 1) || null(wx, wy + 2)) {
                return true;
            }
            if n && (null(wx, wy - 1) || null(wx, wy - 2)) {
                return true;
            }
        } else if gw > 4 {
            if e && null(wx + 1, wy) {
                return true;
            }
            if w && null(wx - 1, wy) {
                return true;
            }
            if s && null(wx, wy + 1) {
                return true;
            }
            if n && null(wx, wy - 1) {
                return true;
            }
        }
        false
    }

    /// `isInvalidChunkAhead()`.
    pub fn is_invalid_chunk_ahead(&self) -> bool {
        let a = self.forward_vector();
        self.is_invalid_chunk_around(a.x < -0.5, a.x > 0.5, a.z < -0.5, a.z > 0.5)
    }

    /// `isInvalidChunkBehind()`.
    pub fn is_invalid_chunk_behind(&self) -> bool {
        let a = self.forward_vector();
        self.is_invalid_chunk_around(a.x > 0.5, a.x < -0.5, a.z > 0.5, a.z < -0.5)
    }

    /// `checkSurroundingChunks()`.
    pub fn check_surrounding_chunks(&self) {
        if self.st().ctrl.is_none() {
            return;
        }
        let (ox, oy) = env::offset();
        let o = self.st().jni_transform.origin;
        let wx = fastfloor(o.x + ox) / 8;
        let wy = fastfloor(o.z + oy) / 8;
        let w = env::world();
        let mut all = true;
        'outer: for dy in -1..=1 {
            for dx in -1..=1 {
                if w.is_valid_chunk(wx + dx, wy + dy) && !w.chunk_loaded(wx + dx, wy + dy) {
                    all = false;
                    break 'outer;
                }
            }
        }
        let desire = self.st().desire_physics_active;
        let disabled = self
            .st()
            .disable_simulation_due_to_lack_of_surrounding_chunks;
        if all {
            if disabled {
                self.st_mut()
                    .disable_simulation_due_to_lack_of_surrounding_chunks = false;
                self.set_physics_active(desire, true);
            }
        } else if !disabled {
            self.st_mut()
                .disable_simulation_due_to_lack_of_surrounding_chunks = true;
            self.set_physics_active(desire, true);
        }
    }

    /// `getDebugZ()`.
    pub fn debug_z(&self) -> f32 {
        self.st().jni_transform.origin.y / LEVEL_HEIGHT
    }
}
