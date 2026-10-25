//! `AimingReticle` (where the player aims on screen: the mouse, or for a
//! gamepad the lerped stick position), `AimingMode.lerpAiming` and
//! `IsoPlayer.getAimVector` / `calculateAimVector`.
//!
//! The engine calls [`GameCharacter::update_aiming_reticle`] once per frame
//! for a gamepad player (`AimingReticle.update()`, before the players
//! update) and feeds the position it returns back to its renderer.

use super::control::CharacterInputMode;
use super::input::PlayerInput;
use crate::character::zombie::math::{lerp, lerp_ease_out_quad, sqrt};
use crate::character::GameCharacter;

/// `AIMING_AXIS_MIN`, `AIMING_AXIS_MAX`, `AIMING_VALUE_MIN`, `AIMING_VALUE_MAX`.
pub const AIMING_AXIS_MIN: f32 = 0.2;
pub const AIMING_AXIS_MAX: f32 = 0.99;
pub const AIMING_VALUE_MIN: f32 = 0.01;
pub const AIMING_VALUE_MAX: f32 = 0.98;

/// `AimingMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AimingMode {
    Unarmed,
    UnarmedTargetFound,
    Melee,
    MeleeTargetFound,
    Ranged,
    RangedTargetFound,
    RangedPrecise,
    RangedPreciseTargetFound,
    NotAiming,
}

impl AimingMode {
    /// From the constant name ([`GameCharacter::aiming_mode`]).
    pub fn from_name(name: &str) -> AimingMode {
        match name {
            "UNARMED" => AimingMode::Unarmed,
            "UNARMED_TARGET_FOUND" => AimingMode::UnarmedTargetFound,
            "MELEE" => AimingMode::Melee,
            "MELEE_TARGET_FOUND" => AimingMode::MeleeTargetFound,
            "RANGED" => AimingMode::Ranged,
            "RANGED_TARGET_FOUND" => AimingMode::RangedTargetFound,
            "RANGED_PRECISE" => AimingMode::RangedPrecise,
            "RANGED_PRECISE_TARGET_FOUND" => AimingMode::RangedPreciseTargetFound,
            _ => AimingMode::NotAiming,
        }
    }

    /// `(fixedAtMinDistance, lerpRateOutwards, lerpRateInwards, minDistanceAmount)`.
    fn params(self) -> (bool, f32, f32, f32) {
        match self {
            AimingMode::RangedPrecise => (false, 0.065, 0.095, 0.0),
            AimingMode::RangedPreciseTargetFound => (false, 0.03, 0.085, 0.0),
            AimingMode::NotAiming => (true, -1.0, -1.0, -1.0),
            _ => (true, -1.0, -1.0, 0.05),
        }
    }
}

/// `PZMath.equal(a, b)` (`|a - b| < 1e-7`).
fn eq(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0e-7
}

/// `PZMath.sign`.
fn sign(v: f32) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `getInputMode()`.
    pub fn input_mode(&self, input: &dyn PlayerInput) -> CharacterInputMode {
        self.player.input.input_mode(input, self.input_owner())
    }

    /// `getScreenChestHeight()` (`def.getScaleY()` is 1).
    pub fn screen_chest_height(&self, input: &dyn PlayerInput) -> f32 {
        20.0 / input.zoom(self.player.player_index)
    }

    /// `AimingReticle.getXA(playerIndex)`.
    pub fn aiming_reticle_xa(&self, input: &dyn PlayerInput) -> i32 {
        if self.input_mode(input) != CharacterInputMode::Gamepad {
            return input.mouse_x();
        }
        self.player.aiming_position.0 as i32
    }

    /// `AimingReticle.getYA(playerIndex)`.
    pub fn aiming_reticle_ya(&self, input: &dyn PlayerInput) -> i32 {
        if self.input_mode(input) != CharacterInputMode::Gamepad {
            return input.mouse_y();
        }
        self.player.aiming_position.1 as i32
    }

    /// `AimingReticle.getX(playerIndex)` (zoomed pixels).
    pub fn aiming_reticle_x(&self, input: &dyn PlayerInput) -> i32 {
        (self.aiming_reticle_xa(input) as f32 * input.zoom(self.player.player_index)) as i32
    }

    /// `AimingReticle.getY(playerIndex)`.
    pub fn aiming_reticle_y(&self, input: &dyn PlayerInput) -> i32 {
        (self.aiming_reticle_ya(input) as f32 * input.zoom(self.player.player_index)) as i32
    }

    fn aiming_center(&self, input: &dyn PlayerInput) -> (f32, f32) {
        let (w, h) = input.screen_size(self.player.player_index);
        (w as f32 / 2.0, h as f32 / 2.0 - self.screen_chest_height(input))
    }

    /// `renormalizeAimingAxisValue`.
    fn renormalize_aiming_axis_value(&self, input: &dyn PlayerInput, v: f32) -> f32 {
        let zoom = input.zoom(self.player.player_index);
        let s = sign(v) as f32;
        let r = ((v * s - AIMING_AXIS_MIN) / 0.79).clamp(0.0, 1.0) * s;
        lerp_ease_out_quad(AIMING_VALUE_MIN / zoom, AIMING_VALUE_MAX, r * s) * s
    }

    /// `AimingReticle.update()` for this player: a gamepad player's aiming
    /// position moves towards its stick (`AimingMode.lerpAiming`). Returns
    /// the position (`None` for other input modes, which aim with the
    /// mouse).
    pub fn update_aiming_reticle(&mut self, input: &dyn PlayerInput, multiplier: f32) -> Option<(f32, f32)> {
        if self.input_mode(input) != CharacterInputMode::Gamepad {
            return None;
        }
        let owner = self.input_owner();
        let (ax, ay) = self.player.input.joypad_aim_vector(input, owner);
        let (cx, cy) = self.aiming_center(input);
        let tx = cx * (1.0 + self.renormalize_aiming_axis_value(input, ax));
        let ty = cy * (1.0 + self.renormalize_aiming_axis_value(input, ay));
        let zoom = input.zoom(self.player.player_index);
        let mode = AimingMode::from_name(self.aiming_mode());
        let (fixed, out_rate, in_rate, min_dist) = mode.params();
        let src = self.player.aiming_position;
        let aim_key = self.player.input.is_aim_key_down(input, owner);
        let snap = !aim_key && self.is_player_aiming() || fixed && min_dist == -1.0;
        let result = if snap {
            // snapAimInFrontOfPlayer.
            let (fx, fy) = crate::character::zombie::math::from_direction(self.anim_angle_radians(), 1.0);
            let h = if self.is_aiming_firearm_equipped() { 0.468 } else { 0.57 };
            let (sx, sy) = input.iso_to_screen(self.player.player_index, self.vars.x + fx, self.vars.y + fy, self.vars.z + h);
            (sx / zoom, sy / zoom)
        } else if fixed {
            // aimFixedDistance.
            let (dx, dy) = (tx - cx, ty - cy);
            if eq(dx, 0.0) && eq(dy, 0.0) {
                (tx, ty)
            } else {
                let d = sqrt(dx * dx + dy * dy);
                let (nx, ny) = (dx / d, dy / d);
                (cx * (1.0 + 2.0 * nx * min_dist / zoom), cy * (1.0 + 2.0 * ny * min_dist / zoom))
            }
        } else if eq(src.0, tx) && eq(src.1, ty) {
            (tx, ty)
        } else {
            let (dsx, dsy) = (src.0 - cx, src.1 - cy);
            let (dtx, dty) = (tx - cx, ty - cy);
            if sign(dsx) != sign(dtx) && sign(dsy) != sign(dty) {
                (tx, ty)
            } else {
                let ds = sqrt(dsx * dsx + dsy * dsy);
                let dt = sqrt(dtx * dtx + dty * dty);
                let rate = if dt > ds { out_rate } else { in_rate };
                let a = (rate * multiplier).clamp(0.0, 1.0);
                (lerp(src.0, tx, a), lerp(src.1, ty, a))
            }
        };
        self.player.aiming_position = result;
        Some(result)
    }

    /// `getAnimAngleRadians()` without an animation player: the forward
    /// direction's angle.
    pub fn anim_angle_radians(&self) -> f32 {
        crate::character::zombie::math::direction(self.vars.forward_x, self.vars.forward_y)
    }

    /// `isAimingFirearmEquipped()`.
    pub fn is_aiming_firearm_equipped(&self) -> bool {
        let aimed = |id: Option<i32>| id.and_then(|i| self.items.item(i)).is_some_and(|i| i.borrow().is_aimed_firearm());
        !self.is_unarmed() && aimed(self.vars.use_hand_weapon)
            || self.items.primary_hand_item().is_some_and(|i| crate::character::items::is_hand_weapon(&i.borrow()) && i.borrow().is_aimed_firearm())
    }

    /// `IsoPlayer.isAiming()`: also while a throw animation runs.
    pub fn is_player_aiming(&self) -> bool {
        if self.player.attack_anim_throw_timer > self.player.now_ms {
            return true;
        }
        self.is_aiming()
    }

    /// `getAimOriginPosZ()` (`getChestHeight()` is 0.495).
    pub fn aim_origin_pos_z(&self) -> f32 {
        self.vars.z + 0.495
    }

    /// `calculateAimVector(vec)`: from the aim origin to the reticle on the
    /// ground at the origin's height, normalized.
    pub fn calculate_aim_vector(&self, input: &dyn PlayerInput) -> (f32, f32) {
        let mx = self.aiming_reticle_x(input);
        let my = self.aiming_reticle_y(input);
        let z = self.aim_origin_pos_z();
        let (ix, iy) = input.screen_to_iso(self.player.player_index, mx, my, z);
        crate::character::zombie::math::normalize(ix - self.vars.x, iy - self.vars.y)
    }

    /// `getAimVector(vec)`: zero unless an aim key is down.
    pub fn aim_vector(&self, input: &dyn PlayerInput) -> (f32, f32) {
        if !self.player.input.is_any_aim_key_down(input, self.input_owner()) {
            return (0.0, 0.0);
        }
        self.calculate_aim_vector(input)
    }
}
