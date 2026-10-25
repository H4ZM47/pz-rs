//! `JoypadManager`'s per-controller queries (`checkJoypad` /
//! `checkOrAnyJoypad`), the `JoypadButton`, `JoypadAxis1d` and
//! `JoypadAxis2d` suppliers and `CharacterJoypadButtonBinding.IsDownBinding`,
//! as pure functions over an [`Input`] and a [`JoypadManager`].
//!
//! They never create a joypad: `JoypadManager.checkJoypad` creates one on
//! first use of a connected controller, so a caller holding the live state
//! runs [`JoypadManager::check`] for the index first (as
//! `pz_lua_api::bind_joypad` does); a frame snapshot cannot, but a player's
//! joypad always exists (`assignJoypad` checked it).

use crate::bindings::IsDown;
use crate::controller::{trigger_value, Input, Joypad, JoypadManager};

/// `JoypadButton` (and the `Joypad.is<Button>Pressed` it reads).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    L3,
    R3,
    Back,
    Start,
    Guide,
    Up,
    Down,
    Left,
    Right,
}

impl Button {
    /// The `JoypadButton` constant `name` (anything else reads as
    /// `DPadDown`, the last constant; the names come from the jar).
    pub fn from_name(name: &str) -> Button {
        match name {
            "A" => Button::A,
            "B" => Button::B,
            "X" => Button::X,
            "Y" => Button::Y,
            "LeftStick" => Button::L3,
            "RightStick" => Button::R3,
            "LeftBump" => Button::LB,
            "RightBump" => Button::RB,
            "Back" => Button::Back,
            "Start" => Button::Start,
            "Guide" => Button::Guide,
            "DPadLeft" => Button::Left,
            "DPadRight" => Button::Right,
            "DPadUp" => Button::Up,
            _ => Button::Down,
        }
    }

    /// `JoypadManager.is<Button>Pressed(c)` uses `checkOrAnyJoypad` (a
    /// negative index means any active joypad) except for the d-pad and
    /// bumpers (`checkJoypad`).
    pub fn accepts_any(self) -> bool {
        !matches!(
            self,
            Button::Up | Button::Down | Button::Left | Button::Right | Button::LB | Button::RB
        )
    }
}

/// The existing joypad of controller `index` (`joypadsController[index]`).
pub fn joypad(jm: &JoypadManager, index: i32) -> Option<&Joypad> {
    usize::try_from(index)
        .ok()
        .and_then(|i| jm.by_controller.get(i))
        .and_then(Option::as_ref)
}

/// `checkJoypad(index, p)` or, with `any` and `index < 0`, `anyJoypad(p)`
/// over `joypadList`.
pub fn check_or_any(
    input: &Input,
    jm: &JoypadManager,
    index: i32,
    any: bool,
    f: impl Fn(&Input, &Joypad) -> bool,
) -> bool {
    if index < 0 && any {
        return jm
            .list
            .iter()
            .filter_map(|&s| jm.by_controller[s].as_ref())
            .any(|j| f(input, j));
    }
    joypad(jm, index).is_some_and(|j| f(input, j))
}

/// `Joypad.is<Button>Pressed()`.
pub fn pressed(input: &Input, j: &Joypad, b: Button) -> bool {
    let id = j.id as i32;
    let dpad = |n: i32, d: &dyn Fn(i32) -> bool| {
        if n != -1 {
            input.is_button_pressed_d(n, id)
        } else {
            d(id)
        }
    };
    match b {
        Button::A => input.is_button_pressed_d(j.a_button, id),
        Button::B => input.is_button_pressed_d(j.b_button, id),
        Button::X => input.is_button_pressed_d(j.x_button, id),
        Button::Y => input.is_button_pressed_d(j.y_button, id),
        Button::LB => input.is_button_pressed_d(j.bumper_left, id),
        Button::RB => input.is_button_pressed_d(j.bumper_right, id),
        Button::L3 => input.is_button_pressed_d(j.left_stick_button, id),
        Button::R3 => input.is_button_pressed_d(j.right_stick_button, id),
        Button::Back => input.is_button_pressed_d(j.back, id),
        Button::Start => input.is_button_pressed_d(j.start, id),
        Button::Guide => input.is_button_pressed_d(j.guide, id),
        Button::Up => dpad(j.dpad_up, &|i| input.is_controller_up_d(i)),
        Button::Down => dpad(j.dpad_down, &|i| input.is_controller_down_d(i)),
        Button::Left => dpad(j.dpad_left, &|i| input.is_controller_left_d(i)),
        Button::Right => dpad(j.dpad_right, &|i| input.is_controller_right_d(i)),
    }
}

/// `JoypadManager.is<Button>Pressed(c)`.
pub fn button_pressed(input: &Input, jm: &JoypadManager, c: i32, b: Button) -> bool {
    check_or_any(input, jm, c, b.accepts_any(), |input, j| {
        pressed(input, j, b)
    })
}

/// `JoypadButton.<name>.isDown(c)`.
pub fn button_down(input: &Input, jm: &JoypadManager, name: &str, c: i32) -> bool {
    button_pressed(input, jm, c, Button::from_name(name))
}

/// `JoypadManager.isLTPressed` / `isRTPressed`.
pub fn trigger_pressed(input: &Input, jm: &JoypadManager, c: i32, left: bool) -> bool {
    check_or_any(input, jm, c, true, |input, j| {
        trigger_value(input, j, left) > 0.7
    })
}

/// `JoypadManager.getLTValue` / `getRTValue`.
pub fn trigger(input: &Input, jm: &JoypadManager, c: i32, left: bool) -> f32 {
    joypad(jm, c).map_or(0.0, |j| trigger_value(input, j, left))
}

/// `Joypad.isMovementAxisBeingApplied` / `isAimingAxisBeingApplied`.
pub fn is_applied(input: &Input, jm: &JoypadManager, j: &Joypad, movement: bool) -> bool {
    let stick = if movement { j.movement() } else { j.aiming() };
    let (x, y) = (
        stick.value(input, j.id as i32, true),
        stick.value(input, j.id as i32, false),
    );
    let (ax, ay) = if movement {
        (j.movement_axis_x, j.movement_axis_y)
    } else {
        (j.aiming_axis_x, j.aiming_axis_y)
    };
    let (dx, dy) = (
        jm.joypad_dead_zone(input, j, ax),
        jm.joypad_dead_zone(input, j, ay),
    );
    // The movement check multiplies the two dead zones; the aiming check
    // sums their squares, as in the original.
    let dz = if movement { dx * dy } else { dx * dx + dy * dy };
    x * x + y * y >= dz
}

/// `JoypadManager.getMovementAxisX/Y` and `getAimingAxisX/Y`: zero unless
/// the stick is outside its dead zone.
pub fn stick(input: &Input, jm: &JoypadManager, c: i32, movement: bool, want_x: bool) -> f32 {
    let Some(j) = joypad(jm, c) else { return 0.0 };
    if !is_applied(input, jm, j, movement) {
        return 0.0;
    }
    let s = if movement { j.movement() } else { j.aiming() };
    s.value(input, j.id as i32, want_x)
}

/// `JoypadManager.isMovementAxisBeingApplied` / `isAimingAxisBeingApplied`.
pub fn stick_applied(input: &Input, jm: &JoypadManager, c: i32, movement: bool) -> bool {
    joypad(jm, c).is_some_and(|j| is_applied(input, jm, j, movement))
}

/// `JoypadAxis2d.<name>`: the movement stick for `LeftStick`, else the
/// aiming stick.
pub fn axis2d_is_movement(name: &str) -> bool {
    name == "LeftStick"
}

/// `JoypadAxis1d.<name>.getValue(c)` (anything else reads as
/// `RightStickY`, the last constant).
pub fn axis1d(input: &Input, jm: &JoypadManager, name: &str, c: i32) -> f32 {
    match name {
        "LeftTrigger" => trigger(input, jm, c, true),
        "RightTrigger" => trigger(input, jm, c, false),
        "LeftStickX" => stick(input, jm, c, true, true),
        "LeftStickY" => stick(input, jm, c, true, false),
        "RightStickX" => stick(input, jm, c, false, true),
        _ => stick(input, jm, c, false, false),
    }
}

/// `JoypadAxis2d.<name>.getValueX/getValueY(c)`.
pub fn axis2d(input: &Input, jm: &JoypadManager, name: &str, c: i32) -> (f32, f32) {
    let m = axis2d_is_movement(name);
    (stick(input, jm, c, m, true), stick(input, jm, c, m, false))
}

/// `IsDownBinding.isDown(c)`: a button, or an axis value / stick length in
/// `[min, max)` (`PZMath.isBetween`).
pub fn is_down(input: &Input, jm: &JoypadManager, b: &IsDown, c: i32) -> bool {
    match b {
        IsDown::Button(n) => button_down(input, jm, n, c),
        IsDown::Axis1d(a, min, max) => {
            let v = axis1d(input, jm, a, c);
            v >= *min && v < *max
        }
        IsDown::Axis2d(a, min, max) => {
            let (x, y) = axis2d(input, jm, a, c);
            let v = (x * x + y * y).sqrt();
            v >= *min && v < *max
        }
    }
}
