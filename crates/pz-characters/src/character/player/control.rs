//! `CharacterInputComponent` (the player's input ECS component) with its
//! `TimedInputHandler`s and the `CharacterInputKeyBinding` /
//! `CharacterJoypad*Binding` queries, over a [`PlayerInput`].

use super::input::{JoypadAxis2dBinding, PlayerInput};

/// `CharacterInputMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterInputMode {
    None,
    Keyboard,
    Gamepad,
}

/// What a `TimedInputHandler` callback reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handler {
    Melee,
    ToggleAim,
    ToggleSprint,
    ToggleRun,
    ToggleCrouch,
    Interact,
    ReloadWeapon,
    RackFirearm,
    ZoomIn,
    ZoomOut,
    Build,
}

/// `TimedInputHandler.MAX_CLICK_TIME`.
pub const MAX_CLICK_TIME: i64 = 400;

/// `TimedInputHandler`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimedInputHandler {
    pub key_down: bool,
    pub key_pressed: bool,
    pub key_released: bool,
    pub key_clicked: bool,
    pub key_is_mouse: bool,
    pub key_was_mouse: bool,
    pub muted: bool,
    pub key_down_ms: i64,
}

impl TimedInputHandler {
    /// `updateState()` with the callbacks' answers.
    pub fn update_state(&mut self, now: i64, down: bool, is_mouse: bool, should_mute: bool) {
        self.key_down = down;
        self.key_is_mouse = is_mouse;
        self.key_pressed = self.key_down && self.key_down_ms == 0;
        self.key_released = !self.key_down && self.key_down_ms != 0;
        self.key_clicked = self.key_released && now - self.key_down_ms < MAX_CLICK_TIME;
        if self.key_pressed {
            self.key_was_mouse = self.key_is_mouse;
        }
        if self.key_pressed {
            self.key_down_ms = now;
        } else if self.key_released {
            self.key_down_ms = 0;
        }
        if self.muted != should_mute {
            self.muted = self.key_down || self.key_released || should_mute;
        }
    }
    /// `mute()`.
    pub fn mute(&mut self) {
        self.muted = true;
    }
    pub fn is_key_down(&self) -> bool {
        !self.muted && self.key_down
    }
    pub fn is_key_pressed(&self) -> bool {
        !self.muted && self.key_pressed
    }
    pub fn is_key_released(&self) -> bool {
        !self.muted && self.key_released
    }
    pub fn is_key_clicked(&self) -> bool {
        !self.muted && self.key_clicked
    }
}

/// `CharacterInputComponent`'s state.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterInput {
    /// `joypadBind` (-1: none).
    pub joypad_bind: i32,
    pub joypad_buttons_active: bool,
    pub ignore_inputs_for_direction: bool,
    pub joypad_ignore_aim: bool,
    pub joypad_ignore_aim_until_centered: bool,
    pub ignore_aiming_input: bool,
    pub allow_sprint: bool,
    pub allow_run: bool,
    pub force_aim: bool,
    pub force_run: bool,
    pub force_sprint: bool,
    pub melee: TimedInputHandler,
    pub toggle_aim: TimedInputHandler,
    pub toggle_sprint: TimedInputHandler,
    pub toggle_run: TimedInputHandler,
    pub toggle_crouch: TimedInputHandler,
    pub interact: TimedInputHandler,
    pub reload_weapon: TimedInputHandler,
    pub rack_firearm: TimedInputHandler,
    pub zoom_in: TimedInputHandler,
    pub zoom_out: TimedInputHandler,
    pub build: TimedInputHandler,
}

impl Default for CharacterInput {
    fn default() -> Self {
        CharacterInput {
            joypad_bind: -1,
            joypad_buttons_active: true,
            ignore_inputs_for_direction: false,
            joypad_ignore_aim: false,
            joypad_ignore_aim_until_centered: false,
            ignore_aiming_input: false,
            allow_sprint: true,
            allow_run: true,
            force_aim: false,
            force_run: false,
            force_sprint: false,
            melee: Default::default(),
            toggle_aim: Default::default(),
            toggle_sprint: Default::default(),
            toggle_run: Default::default(),
            toggle_crouch: Default::default(),
            interact: Default::default(),
            reload_weapon: Default::default(),
            rack_firearm: Default::default(),
            zoom_in: Default::default(),
            zoom_out: Default::default(),
            build: Default::default(),
        }
    }
}

/// The parts of the owning player the component reads.
#[derive(Clone, Copy, Debug)]
pub struct InputOwner {
    /// `getIndex()`.
    pub player_index: i32,
    /// `ClimbWindowStarted || ClimbFenceStarted` (`shouldAllowForceRunOrSprint`).
    pub climb_started: bool,
}

/// `CharacterInputKeyBinding`: `isKeyDown` etc. by the binding's name.
pub struct KeyBinding;

impl KeyBinding {
    /// `isKeyDown()`.
    pub fn is_key_down(input: &dyn PlayerInput, key: &str) -> bool {
        input.which_key_down(key) != 0
    }
    /// `isKeyDown(keyCode)`.
    pub fn is_key_down_code(input: &dyn PlayerInput, key: &str, code: i32) -> bool {
        input.which_key_down(key) == code
    }
    /// `isMouseKey()`.
    pub fn is_mouse_key(input: &dyn PlayerInput, key: &str) -> bool {
        input.which_key_down(key) >= 10000
    }
    /// `isCtrlKey()`.
    pub fn is_ctrl_key(input: &dyn PlayerInput, key: &str) -> bool {
        let k = input.which_key_down(key);
        k == 29 || k == 157
    }
    /// `isKeyDownInWorld()`.
    pub fn is_key_down_in_world(input: &dyn PlayerInput, key: &str) -> bool {
        let over = (Self::is_mouse_key(input, key) || Self::is_ctrl_key(input, key)) && input.is_mouse_over_inventory();
        !over && Self::is_key_down(input, key)
    }
}

impl CharacterInput {
    /// `isJoypadControllerActive()`.
    pub fn is_joypad_controller_active(&self, input: &dyn PlayerInput) -> bool {
        input.joypad_activated() && self.joypad_bind != -1
    }

    /// `getInputMode()`.
    pub fn input_mode(&self, input: &dyn PlayerInput, owner: InputOwner) -> CharacterInputMode {
        if self.is_joypad_controller_active(input) {
            CharacterInputMode::Gamepad
        } else if owner.player_index == 0 {
            CharacterInputMode::Keyboard
        } else {
            CharacterInputMode::None
        }
    }

    fn axis_value(&self, input: &dyn PlayerInput, axis: JoypadAxis2dBinding) -> (f32, f32) {
        input.joypad_axis(self.joypad_bind, axis)
    }

    fn button(&self, input: &dyn PlayerInput, b: &str) -> bool {
        input.joypad_button_down(self.joypad_bind, b)
    }

    /// `frameStep()`: `checkJoypadIgnoreAimUntilCentered` then the toggles.
    pub fn frame_step(&mut self, input: &dyn PlayerInput, owner: InputOwner) {
        if !self.joypad_ignore_aim {
            let applied = self.is_joypad_aiming_axis_applied_internal(input, owner);
            self.joypad_ignore_aim_until_centered = self.joypad_ignore_aim_until_centered && applied;
        }
        self.update_toggle_buttons(input, owner);
    }

    fn handler_query(&self, h: Handler, input: &dyn PlayerInput, owner: InputOwner) -> (bool, bool, bool) {
        let mode = self.input_mode(input, owner);
        let not_active = !self.joypad_buttons_active;
        match h {
            Handler::Melee => (self.is_melee_button_down_internal(input, owner), false, false),
            Handler::ToggleAim => (
                self.is_toggle_aim_key_down(input, owner),
                input.toggle_to_aim() && mode == CharacterInputMode::Keyboard && KeyBinding::is_mouse_key(input, "Aim"),
                !input.toggle_to_aim(),
            ),
            Handler::ToggleSprint => (input.toggle_to_sprint() && self.is_sprint_button_down(input, owner), false, !input.toggle_to_sprint()),
            Handler::ToggleRun => (input.toggle_to_run() && self.is_run_button_down(input, owner), false, !input.toggle_to_run()),
            Handler::ToggleCrouch => (self.is_crouch_button_down(input, owner), false, false),
            Handler::Interact => (self.is_interact_button_down_internal(input, owner), false, not_active),
            Handler::ReloadWeapon => (self.binding_or_button(input, owner, "ReloadWeapon", "ReloadWeapon", true), false, not_active),
            Handler::RackFirearm => (self.binding_or_button(input, owner, "Rack Firearm", "RackFirearm", true), false, not_active),
            Handler::ZoomIn => (self.binding_or_button(input, owner, "Zoom in", "ZoomIn", true), false, not_active),
            Handler::ZoomOut => (self.binding_or_button(input, owner, "Zoom out", "ZoomOut", true), false, not_active),
            Handler::Build => (self.binding_or_button(input, owner, "Attack/Click", "Interact", false), false, not_active),
        }
    }

    /// The keyboard binding in the world, or the joypad button (with the
    /// buttons active when `needs_active`).
    fn binding_or_button(&self, input: &dyn PlayerInput, owner: InputOwner, key: &str, button: &str, needs_active: bool) -> bool {
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => KeyBinding::is_key_down_in_world(input, key),
            CharacterInputMode::Gamepad => (!needs_active || self.joypad_buttons_active) && self.button(input, button),
            CharacterInputMode::None => false,
        }
    }

    fn update_handler(&mut self, h: Handler, input: &dyn PlayerInput, owner: InputOwner) {
        let (down, mouse, mute) = self.handler_query(h, input, owner);
        let now = input.current_time_ms();
        let t = match h {
            Handler::Melee => &mut self.melee,
            Handler::ToggleAim => &mut self.toggle_aim,
            Handler::ToggleSprint => &mut self.toggle_sprint,
            Handler::ToggleRun => &mut self.toggle_run,
            Handler::ToggleCrouch => &mut self.toggle_crouch,
            Handler::Interact => &mut self.interact,
            Handler::ReloadWeapon => &mut self.reload_weapon,
            Handler::RackFirearm => &mut self.rack_firearm,
            Handler::ZoomIn => &mut self.zoom_in,
            Handler::ZoomOut => &mut self.zoom_out,
            Handler::Build => &mut self.build,
        };
        t.update_state(now, down, mouse, mute);
    }

    /// `updateToggleButtons()`.
    pub fn update_toggle_buttons(&mut self, input: &dyn PlayerInput, owner: InputOwner) {
        self.update_handler(Handler::Melee, input, owner);
        // updateToggleToAim
        self.update_handler(Handler::ToggleAim, input, owner);
        if self.toggle_aim.muted {
            self.force_aim = false;
        } else if self.toggle_aim.is_key_released() && self.toggle_aim.key_was_mouse != self.toggle_aim.is_key_clicked() {
            self.force_aim = !self.force_aim;
        }
        // updateToggleToRun
        self.update_handler(Handler::ToggleRun, input, owner);
        if self.toggle_run.muted || owner.climb_started {
            self.force_run = false;
        } else if self.toggle_run.is_key_clicked() {
            self.force_run = !self.force_run;
        }
        // updateToggleToSprint
        self.update_handler(Handler::ToggleSprint, input, owner);
        if self.toggle_sprint.muted || owner.climb_started {
            self.force_sprint = false;
        } else if self.toggle_sprint.is_key_clicked() {
            self.force_sprint = !self.force_sprint;
        }
        self.update_handler(Handler::ToggleCrouch, input, owner);
        self.update_handler(Handler::Interact, input, owner);
        self.update_handler(Handler::ReloadWeapon, input, owner);
        self.update_handler(Handler::RackFirearm, input, owner);
        self.update_handler(Handler::ZoomIn, input, owner);
        self.update_handler(Handler::ZoomOut, input, owner);
        self.update_handler(Handler::Build, input, owner);
    }

    /// `getInputMoveVector(out)`.
    pub fn input_move_vector(&self, input: &dyn PlayerInput, owner: InputOwner) -> (f32, f32) {
        let run = self.allow_run && self.is_run_button_down(input, owner);
        let max = if run { 2.0 } else { 1.95 };
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => {
                let (mut x, mut y) = (0.0f32, 0.0f32);
                if self.is_forward_key_down(input, owner) {
                    y += -1.0;
                } else if self.is_backward_key_down(input, owner) {
                    y += 1.0;
                }
                if self.is_left_key_down(input, owner) {
                    x += -1.0;
                } else if self.is_right_key_down(input, owner) {
                    x += 1.0;
                }
                crate::character::zombie::math::set_length(x, y, max)
            }
            CharacterInputMode::Gamepad => {
                if !self.is_joypad_movement_axis_applied(input, owner) {
                    return (0.0, 0.0);
                }
                let (x, y) = self.axis_value(input, JoypadAxis2dBinding::Movement);
                let (x, y) = (x * max, y * max);
                // setMaxLength
                let len = crate::character::zombie::math::length(x, y);
                if len > max {
                    crate::character::zombie::math::set_length(x, y, max)
                } else {
                    (x, y)
                }
            }
            CharacterInputMode::None => (0.0, 0.0),
        }
    }

    /// `getInputMovementRate()`.
    pub fn input_movement_rate(&self, input: &dyn PlayerInput, owner: InputOwner) -> f32 {
        let (x, y) = self.input_move_vector(input, owner);
        crate::character::zombie::math::length(x, y)
    }

    /// `isInputMoveAxisApplied()` (`PZMath.equal(rate, 0)` is `|rate| < 1e-7`).
    pub fn is_input_move_axis_applied(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.input_movement_rate(input, owner).abs() >= 1.0e-7
    }

    /// `getJoypadAimVector(out)`.
    pub fn joypad_aim_vector(&self, input: &dyn PlayerInput, owner: InputOwner) -> (f32, f32) {
        if !self.is_joypad_aiming_axis_applied(input, owner) {
            return (0.0, 0.0);
        }
        self.axis_value(input, JoypadAxis2dBinding::Aiming)
    }

    fn movement_key(&self, input: &dyn PlayerInput, owner: InputOwner, key: &str, pick: impl Fn((f32, f32)) -> bool) -> bool {
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => !self.is_keyboard_selecting_all(input) && KeyBinding::is_key_down_in_world(input, key),
            CharacterInputMode::Gamepad => pick(self.axis_value(input, JoypadAxis2dBinding::Movement)),
            CharacterInputMode::None => false,
        }
    }
    pub fn is_forward_key_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.movement_key(input, owner, "Forward", |v| v.1 > 0.0)
    }
    pub fn is_backward_key_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.movement_key(input, owner, "Backward", |v| v.1 < 0.0)
    }
    pub fn is_left_key_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.movement_key(input, owner, "Left", |v| v.0 < 0.0)
    }
    pub fn is_right_key_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.movement_key(input, owner, "Right", |v| v.0 > 0.0)
    }

    /// `isKeyboardSelectingAll()`.
    pub fn is_keyboard_selecting_all(&self, input: &dyn PlayerInput) -> bool {
        input.is_selecting_all()
            && input.is_mouse_over_inventory()
            && self.is_ctrl_key_down(input)
            && ["Left", "Right", "Forward", "Backward"].iter().any(|k| KeyBinding::is_key_down_code(input, k, 30))
    }

    /// `isAimKeyDown()`.
    pub fn is_aim_key_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.force_aim || self.is_aim_key_down_internal(input, owner)
    }
    /// `isPrecisionAimKeyDown()`.
    pub fn is_precision_aim_key_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.force_aim || self.is_precision_aim_key_down_internal(input, owner)
    }
    /// `isToggleAimKeyDown()`.
    pub fn is_toggle_aim_key_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        input.toggle_to_aim() && self.input_mode(input, owner) == CharacterInputMode::Keyboard && self.is_aim_key_down_internal(input, owner)
    }
    /// `isAimKeyDownInternal()`.
    pub fn is_aim_key_down_internal(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => KeyBinding::is_key_down_in_world(input, "Aim"),
            CharacterInputMode::Gamepad => self.is_joypad_aiming_axis_applied(input, owner) && self.button(input, "Aim"),
            CharacterInputMode::None => false,
        }
    }
    /// `isPrecisionAimKeyDownInternal()`.
    pub fn is_precision_aim_key_down_internal(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => self.is_aim_key_down_internal(input, owner),
            CharacterInputMode::Gamepad => {
                !self.joypad_ignore_aim && !self.joypad_ignore_aim_until_centered && self.button(input, "PrecisionAim")
            }
            CharacterInputMode::None => false,
        }
    }
    /// `isAnyAimKeyDown()`.
    pub fn is_any_aim_key_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.is_aim_key_down(input, owner) || self.is_precision_aim_key_down(input, owner)
    }
    /// `isMeleeButtonDown()`.
    pub fn is_melee_button_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => self.melee.is_key_down(),
            CharacterInputMode::Gamepad => self.melee.is_key_down() && !self.melee.is_key_pressed(),
            CharacterInputMode::None => false,
        }
    }
    /// `isMeleeButtonDownInternal()`.
    pub fn is_melee_button_down_internal(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.binding_or_button(input, owner, "Melee", "Melee", false)
    }
    /// `isAttackButtonDown()`.
    pub fn is_attack_button_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.binding_or_button(input, owner, "Attack/Click", "Attack", false)
    }
    /// `isRunButtonDown()`.
    pub fn is_run_button_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        if !self.allow_run {
            return false;
        }
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => KeyBinding::is_key_down(input, "Run"),
            CharacterInputMode::Gamepad => self.is_joypad_movement_axis_applied(input, owner) && self.button(input, "Run"),
            CharacterInputMode::None => false,
        }
    }
    /// `wasRunButtonDown()`.
    pub fn was_run_button_down(&self, input: &dyn PlayerInput) -> bool {
        input.was_key_down("Run")
    }
    /// `isInteractButtonPressed()`.
    pub fn is_interact_button_pressed(&self) -> bool {
        self.interact.is_key_pressed()
    }
    /// `isInteractButtonDown()`.
    pub fn is_interact_button_down(&self) -> bool {
        self.interact.is_key_down()
    }
    /// `isInteractButtonDownInternal()`.
    pub fn is_interact_button_down_internal(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.binding_or_button(input, owner, "Interact", "Interact", false)
    }
    /// `isWalkToButtonDown()`.
    pub fn is_walk_to_button_down(&self, input: &dyn PlayerInput) -> bool {
        KeyBinding::is_key_down_in_world(input, "WalkTo")
    }
    /// `isCrouchButtonDown()`.
    pub fn is_crouch_button_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.binding_or_button(input, owner, "Crouch", "Crouch", true)
    }
    /// `isCrouchButtonPressed()`.
    pub fn is_crouch_button_pressed(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => self.toggle_crouch.is_key_pressed(),
            CharacterInputMode::Gamepad => self.joypad_buttons_active && self.toggle_crouch.is_key_clicked(),
            CharacterInputMode::None => false,
        }
    }
    /// `isReloadWeaponButtonPressed()`.
    pub fn is_reload_weapon_button_pressed(&self) -> bool {
        self.reload_weapon.is_key_pressed()
    }
    /// `isRackFirearmButtonPressed()`.
    pub fn is_rack_firearm_button_pressed(&self) -> bool {
        self.rack_firearm.is_key_pressed()
    }
    /// `isSprintButtonDown()`.
    pub fn is_sprint_button_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        if !self.allow_sprint {
            return false;
        }
        match self.input_mode(input, owner) {
            CharacterInputMode::Keyboard => KeyBinding::is_key_down(input, "Sprint"),
            CharacterInputMode::Gamepad => self.joypad_buttons_active && self.button(input, "Sprint"),
            CharacterInputMode::None => false,
        }
    }
    /// `isCancelActionButtonDown()`.
    pub fn is_cancel_action_button_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.binding_or_button(input, owner, "CancelAction", "CancelAction", true)
    }
    /// `isManualFloorAtkButtonDown()`.
    pub fn is_manual_floor_atk_button_down(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.binding_or_button(input, owner, "ManualFloorAtk", "ManualFloorAtk", true)
    }
    /// `isShiftKeyDown()`.
    pub fn is_shift_key_down(&self, input: &dyn PlayerInput) -> bool {
        input.is_key_code_down(42) || input.is_key_code_down(54)
    }
    /// `isCtrlKeyDown()`.
    pub fn is_ctrl_key_down(&self, input: &dyn PlayerInput) -> bool {
        input.is_key_code_down(29) || input.is_key_code_down(157)
    }
    /// `isJoypadMovementAxisApplied()`.
    pub fn is_joypad_movement_axis_applied(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        !self.ignore_inputs_for_direction
            && self.input_mode(input, owner) == CharacterInputMode::Gamepad
            && input.joypad_axis_applied(self.joypad_bind, JoypadAxis2dBinding::Movement)
    }
    /// `isJoypadAimingAxisApplied()`.
    pub fn is_joypad_aiming_axis_applied(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        !self.joypad_ignore_aim && !self.joypad_ignore_aim_until_centered && self.is_joypad_aiming_axis_applied_internal(input, owner)
    }
    fn is_joypad_aiming_axis_applied_internal(&self, input: &dyn PlayerInput, owner: InputOwner) -> bool {
        self.input_mode(input, owner) == CharacterInputMode::Gamepad && input.joypad_axis_applied(self.joypad_bind, JoypadAxis2dBinding::Aiming)
    }

    /// `onGameLoadingStateEnter()`.
    pub fn on_game_loading_state_enter(&mut self) {
        self.joypad_buttons_active = false;
        self.ignore_aiming_input = true;
        self.ignore_inputs_for_direction = true;
        for t in [
            &mut self.melee,
            &mut self.toggle_aim,
            &mut self.toggle_sprint,
            &mut self.toggle_run,
            &mut self.toggle_crouch,
            &mut self.interact,
            &mut self.reload_weapon,
            &mut self.rack_firearm,
            &mut self.zoom_in,
            &mut self.zoom_out,
        ] {
            t.mute();
        }
    }

    /// `onInGameStateEnter()`.
    pub fn on_in_game_state_enter(&mut self) {
        self.joypad_buttons_active = true;
        self.ignore_aiming_input = false;
        self.ignore_inputs_for_direction = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Keys(Vec<(&'static str, i32)>);
    impl PlayerInput for Keys {
        fn which_key_down(&self, b: &str) -> i32 {
            self.0.iter().find(|k| k.0 == b).map_or(0, |k| k.1)
        }
    }
    const P0: InputOwner = InputOwner { player_index: 0, climb_started: false };

    #[test]
    fn keyboard_move_vector_and_modes() {
        let c = CharacterInput::default();
        let k = Keys(vec![("Forward", 17), ("Right", 32)]);
        assert_eq!(c.input_mode(&k, P0), CharacterInputMode::Keyboard);
        assert_eq!(c.input_mode(&k, InputOwner { player_index: 1, climb_started: false }), CharacterInputMode::None);
        let (x, y) = c.input_move_vector(&k, P0);
        assert!((x - 1.3788582).abs() < 1e-6 && (y + 1.3788582).abs() < 1e-6);
        let k = Keys(vec![("Forward", 17), ("Run", 42)]);
        assert_eq!(c.input_move_vector(&k, P0), (0.0, -2.0));
    }

    #[test]
    fn timed_handler_click() {
        let mut t = TimedInputHandler::default();
        t.update_state(1000, true, false, false);
        assert!(t.is_key_pressed());
        t.update_state(1100, true, false, false);
        assert!(!t.is_key_pressed() && t.is_key_down());
        t.update_state(1200, false, false, false);
        assert!(t.is_key_released() && t.is_key_clicked());
        t.update_state(1300, true, false, false);
        t.update_state(2000, false, false, false);
        assert!(t.is_key_released() && !t.is_key_clicked());
    }
}
