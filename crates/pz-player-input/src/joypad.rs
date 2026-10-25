//! The joypad half of the adapter: `JoypadManager`'s per-controller
//! queries (`checkJoypad` / `checkOrAnyJoypad`), the `JoypadButton`,
//! `JoypadAxis1d` and `JoypadAxis2d` suppliers and the
//! `CharacterJoypadButtonBinding` / `CharacterJoypadAxis2dBinding`
//! bindings, evaluated over one frame's copy of the controller state.
//!
//! The evaluation is `pz_input::joypad_eval`, shared with
//! `pz_lua_api::bind_joypad`; this reads a snapshot instead of the
//! thread-locals, so `IsoPlayer.update` can hold it while Lua runs.

use pz_input::bindings::{Axis2dBinding, ButtonBinding, IsDown};
use pz_input::controller::{Input, JoypadManager};
use pz_input::joypad_eval;

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

impl JoypadFrame {
    /// `JoypadButton.<name>.isDown(c)`.
    pub fn button(&self, name: &str, c: i32) -> bool {
        joypad_eval::button_down(&self.input, &self.jm, name, c)
    }

    /// `JoypadManager.getMovementAxisX/Y` / `getAimingAxisX/Y`.
    pub fn stick(&self, c: i32, movement: bool, want_x: bool) -> f32 {
        joypad_eval::stick(&self.input, &self.jm, c, movement, want_x)
    }

    /// `JoypadManager.isMovementAxisBeingApplied` / `isAimingAxisBeingApplied`.
    pub fn stick_applied(&self, c: i32, movement: bool) -> bool {
        joypad_eval::stick_applied(&self.input, &self.jm, c, movement)
    }

    /// `JoypadManager.getLTValue` / `getRTValue`.
    pub fn trigger(&self, c: i32, left: bool) -> f32 {
        joypad_eval::trigger(&self.input, &self.jm, c, left)
    }

    /// `IsDownBinding.isDown(c)`.
    pub fn is_down(&self, b: &IsDown, c: i32) -> bool {
        joypad_eval::is_down(&self.input, &self.jm, b, c)
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
        self.axis_binding(name).map_or((0.0, 0.0), |a| {
            joypad_eval::axis2d(&self.input, &self.jm, a, c)
        })
    }

    /// `CharacterJoypadAxis2dBinding.isApplied(joypadBind)`.
    pub fn binding_axis_applied(&self, name: &str, c: i32) -> bool {
        self.axis_binding(name)
            .is_some_and(|a| self.stick_applied(c, joypad_eval::axis2d_is_movement(a)))
    }
}
