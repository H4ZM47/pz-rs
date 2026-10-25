//! The joypad half of the adapter: `JoypadManager`'s per-controller
//! queries (`checkJoypad` / `checkOrAnyJoypad`), the `JoypadButton`,
//! `JoypadAxis1d` and `JoypadAxis2d` suppliers and the
//! `CharacterJoypadButtonBinding` / `CharacterJoypadAxis2dBinding`
//! bindings, evaluated over one frame's copy of the controller state.
//!
//! This mirrors the private helpers of `pz_lua_api::bind_joypad` (which
//! serve the same queries to Lua) but reads a snapshot instead of the
//! thread-locals, so `IsoPlayer.update` can hold it while Lua runs.

use pz_input::bindings::{Axis2dBinding, ButtonBinding, IsDown};
use pz_input::controller::{trigger_value, Input, Joypad, JoypadManager};

/// One frame's controller state and character bindings.
#[derive(Clone, Debug, Default)]
pub struct JoypadFrame {
    /// `GameWindow.GameInput`.
    pub input: Input,
    /// `JoypadManager.instance`.
    pub jm: JoypadManager,
    /// `CharacterJoypadButtonBinding.values()` with their current bindings.
    pub buttons: Vec<ButtonBinding>,
    /// `CharacterJoypadAxis2dBinding.values()` with their current bindings.
    pub axes: Vec<Axis2dBinding>,
    /// `GameWindow.activatedJoyPad` (a controller slot).
    pub activated: Option<usize>,
}

/// `JoypadManager.Joypad.is<Button>Pressed()`.
fn joypad_pressed(input: &Input, j: &Joypad, button: &str) -> bool {
    let id = j.id as i32;
    let dpad = |n: i32, d: fn(&Input, i32) -> bool| {
        if n != -1 {
            input.is_button_pressed_d(n, id)
        } else {
            d(input, id)
        }
    };
    match button {
        "A" => input.is_button_pressed_d(j.a_button, id),
        "B" => input.is_button_pressed_d(j.b_button, id),
        "X" => input.is_button_pressed_d(j.x_button, id),
        "Y" => input.is_button_pressed_d(j.y_button, id),
        "LeftStick" => input.is_button_pressed_d(j.left_stick_button, id),
        "RightStick" => input.is_button_pressed_d(j.right_stick_button, id),
        "LeftBump" => input.is_button_pressed_d(j.bumper_left, id),
        "RightBump" => input.is_button_pressed_d(j.bumper_right, id),
        "Back" => input.is_button_pressed_d(j.back, id),
        "Start" => input.is_button_pressed_d(j.start, id),
        "Guide" => input.is_button_pressed_d(j.guide, id),
        "DPadUp" => dpad(j.dpad_up, Input::is_controller_up_d),
        "DPadDown" => dpad(j.dpad_down, Input::is_controller_down_d),
        "DPadLeft" => dpad(j.dpad_left, Input::is_controller_left_d),
        "DPadRight" => dpad(j.dpad_right, Input::is_controller_right_d),
        _ => false,
    }
}

impl JoypadFrame {
    /// `JoypadManager.checkJoypad(index)`. The original creates the joypad
    /// on first use of a connected controller; a snapshot cannot, but a
    /// player's joypad always exists (`assignJoypad` checked it).
    fn joypad(&self, index: i32) -> Option<&Joypad> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.jm.by_controller.get(i))
            .and_then(Option::as_ref)
    }

    /// `checkJoypad(index, p)` or, with `any` and `index < 0`,
    /// `anyJoypad(p)` over `joypadList`.
    fn check(&self, index: i32, any: bool, f: impl Fn(&Joypad) -> bool) -> bool {
        if index < 0 && any {
            return self
                .jm
                .list
                .iter()
                .filter_map(|&s| self.jm.by_controller.get(s).and_then(Option::as_ref))
                .any(f);
        }
        self.joypad(index).is_some_and(f)
    }

    /// `JoypadButton.<name>.isDown(joypadBind)`: the d-pad and bumpers
    /// check one joypad (`checkJoypad`); the other buttons accept a negative
    /// index for any active joypad (`checkOrAnyJoypad`).
    pub fn button(&self, name: &str, c: i32) -> bool {
        let any = !matches!(
            name,
            "DPadUp" | "DPadDown" | "DPadLeft" | "DPadRight" | "LeftBump" | "RightBump"
        );
        self.check(c, any, |j| joypad_pressed(&self.input, j, name))
    }

    /// `Joypad.isMovementAxisBeingApplied` / `isAimingAxisBeingApplied`.
    fn is_applied(&self, j: &Joypad, movement: bool) -> bool {
        let stick = if movement { j.movement() } else { j.aiming() };
        let id = j.id as i32;
        let (x, y) = (
            stick.value(&self.input, id, true),
            stick.value(&self.input, id, false),
        );
        let (ax, ay) = if movement {
            (j.movement_axis_x, j.movement_axis_y)
        } else {
            (j.aiming_axis_x, j.aiming_axis_y)
        };
        let (dx, dy) = (
            self.jm.joypad_dead_zone(&self.input, j, ax),
            self.jm.joypad_dead_zone(&self.input, j, ay),
        );
        // `isMovementInsideDeadZone` multiplies the two dead zones;
        // `isAimingInsideDeadZone` sums their squares.
        let dz = if movement { dx * dy } else { dx * dx + dy * dy };
        x * x + y * y >= dz
    }

    /// `JoypadManager.getMovementAxisX/Y` / `getAimingAxisX/Y`: zero unless
    /// the stick is outside its dead zone.
    pub fn stick(&self, c: i32, movement: bool, want_x: bool) -> f32 {
        let Some(j) = self.joypad(c) else { return 0.0 };
        if !self.is_applied(j, movement) {
            return 0.0;
        }
        let stick = if movement { j.movement() } else { j.aiming() };
        stick.value(&self.input, j.id as i32, want_x)
    }

    /// `JoypadManager.isMovementAxisBeingApplied` / `isAimingAxisBeingApplied`.
    pub fn stick_applied(&self, c: i32, movement: bool) -> bool {
        self.joypad(c).is_some_and(|j| self.is_applied(j, movement))
    }

    /// `JoypadManager.getLTValue` / `getRTValue`.
    pub fn trigger(&self, c: i32, left: bool) -> f32 {
        self.joypad(c)
            .map_or(0.0, |j| trigger_value(&self.input, j, left))
    }

    /// `JoypadAxis1d.<name>.getValue(joypadBind)`.
    pub fn axis1d(&self, name: &str, c: i32) -> f32 {
        match name {
            "LeftTrigger" => self.trigger(c, true),
            "RightTrigger" => self.trigger(c, false),
            "LeftStickX" => self.stick(c, true, true),
            "LeftStickY" => self.stick(c, true, false),
            "RightStickX" => self.stick(c, false, true),
            "RightStickY" => self.stick(c, false, false),
            _ => 0.0,
        }
    }

    /// `JoypadAxis2d.<name>`: `Some(true)` for the movement stick.
    fn axis2d_is_movement(name: &str) -> Option<bool> {
        match name {
            "LeftStick" => Some(true),
            "RightStick" => Some(false),
            _ => None,
        }
    }

    /// `JoypadAxis2d.<name>.getValueX/getValueY(joypadBind)`.
    pub fn axis2d(&self, name: &str, c: i32) -> (f32, f32) {
        Self::axis2d_is_movement(name).map_or((0.0, 0.0), |m| {
            (self.stick(c, m, true), self.stick(c, m, false))
        })
    }

    /// `IsDownBinding.isDown(joypadBind)`: a button, or an axis value /
    /// stick length in `[min, max)` (`PZMath.isBetween`).
    pub fn is_down(&self, b: &IsDown, c: i32) -> bool {
        match b {
            IsDown::Button(n) => self.button(n, c),
            IsDown::Axis1d(a, min, max) => {
                let v = self.axis1d(a, c);
                v >= *min && v < *max
            }
            IsDown::Axis2d(a, min, max) => {
                let (x, y) = self.axis2d(a, c);
                // `PZMath.getLength`.
                let v = (x * x + y * y).sqrt();
                v >= *min && v < *max
            }
        }
    }

    /// `CharacterJoypadButtonBinding.valueOf(name).isDown(joypadBind)`; an
    /// unknown name or a null binding is up.
    pub fn binding_down(&self, name: &str, c: i32) -> bool {
        self.buttons
            .iter()
            .find(|b| b.name == name)
            .and_then(|b| b.binding.as_ref())
            .is_some_and(|b| self.is_down(b, c))
    }

    /// The `JoypadAxis2d` a `CharacterJoypadAxis2dBinding` is bound to.
    fn axis_binding(&self, name: &str) -> Option<&str> {
        self.axes
            .iter()
            .find(|a| a.name == name)
            .and_then(|a| a.binding.as_deref())
    }

    /// `CharacterJoypadAxis2dBinding.getValueX/getValueY(joypadBind)`.
    pub fn binding_axis(&self, name: &str, c: i32) -> (f32, f32) {
        self.axis_binding(name)
            .map_or((0.0, 0.0), |a| self.axis2d(a, c))
    }

    /// `CharacterJoypadAxis2dBinding.isApplied(joypadBind)`.
    pub fn binding_axis_applied(&self, name: &str, c: i32) -> bool {
        self.axis_binding(name)
            .and_then(Self::axis2d_is_movement)
            .is_some_and(|m| self.stick_applied(c, m))
    }
}
