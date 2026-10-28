//! [`PlayerInput`]: what `IsoPlayer.update` reads from the keyboard, the
//! mouse and the player's joypad (`GameKeyboard`, `Mouse`,
//! `CharacterInputKeyBinding`, `CharacterJoypadAxis2dBinding`,
//! `CharacterJoypadButtonBinding`, `AimingReticle`, `IsoUtils.XToIso`/
//! `YToIso`, the `Core` input options).
//!
//! The engine implements it over its input state (the rendering thread
//! owns the keyboard, mouse, controllers, camera and zoom); every method
//! has a no-op default, so the default implementation is "nothing pressed".
//! Bindings are named like `Core.getKey`: `"Forward"`, `"Backward"`,
//! `"Left"`, `"Right"`, `"Aim"`, `"Melee"`, `"Attack/Click"`, `"Run"`,
//! `"Interact"`, `"WalkTo"`, `"Crouch"`, `"ReloadWeapon"`,
//! `"Rack Firearm"`, `"Sprint"`, `"CancelAction"`, `"ManualFloorAtk"`,
//! `"Zoom in"`, `"Zoom out"`. Key codes are LWJGL codes; mouse buttons are
//! `10000 + button` (`Mouse.isButtonKey`).

/// `CharacterJoypadAxis2dBinding`: the player's two sticks by role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JoypadAxis2dBinding {
    /// `Movement` (default the left stick).
    Movement,
    /// `Aiming` (default the right stick).
    Aiming,
}

/// The input `IsoPlayer.update` reads. All methods default to no input.
pub trait PlayerInput {
    // --- keyboard (GameKeyboard) by binding name --------------------------

    /// `GameKeyboard.isKeyDown(String)`: the binding's key or alt key is down
    /// (false while text entry is active or for an invalid shift/ctrl bind).
    fn is_key_down(&self, _binding: &str) -> bool {
        false
    }
    /// `GameKeyboard.whichKeyDown(String)`: the code of the binding's key or
    /// alt key that is down, else 0.
    fn which_key_down(&self, _binding: &str) -> i32 {
        0
    }
    /// `GameKeyboard.wasKeyDown(String)`: down on the previous update.
    fn was_key_down(&self, _binding: &str) -> bool {
        false
    }
    /// `GameKeyboard.isKeyPressed(String)`: down now, not last update.
    fn is_key_pressed(&self, binding: &str) -> bool {
        self.is_key_down(binding) && !self.was_key_down(binding)
    }
    /// `GameKeyboard.isKeyDown(int)` for a raw key code (shift 42/54,
    /// ctrl 29/157, F12 88, ...); codes >= 10000 are mouse buttons.
    fn is_key_code_down(&self, _code: i32) -> bool {
        false
    }
    /// `GameKeyboard.wasKeyDown(int)`.
    fn was_key_code_down(&self, _code: i32) -> bool {
        false
    }
    /// `Core.currentTextEntryBox != null && isDoingTextEntry()`.
    fn is_doing_text_entry(&self) -> bool {
        false
    }
    /// `GameKeyboard.doLuaKeyPressed`.
    fn do_lua_key_pressed(&self) -> bool {
        true
    }

    // --- mouse ---------------------------------------------------------------

    /// `Mouse.getXA()` (screen pixels, unscaled by zoom).
    fn mouse_x(&self) -> i32 {
        0
    }
    /// `Mouse.getYA()`.
    fn mouse_y(&self) -> i32 {
        0
    }
    /// `Mouse.isButtonDown(button)`.
    fn is_mouse_button_down(&self, _button: i32) -> bool {
        false
    }
    /// The wheel delta this update (`Mouse.getWheelState` change).
    fn mouse_wheel(&self) -> i32 {
        0
    }
    /// `UIManager.isMouseOverInventory()`.
    fn is_mouse_over_inventory(&self) -> bool {
        false
    }

    // --- joypad -------------------------------------------------------------

    /// `GameWindow.activatedJoyPad != null`.
    fn joypad_activated(&self) -> bool {
        false
    }
    /// `CharacterJoypadAxis2dBinding.getValueX/getValueY(joypad)`: the bound
    /// stick of controller `joypad` (`(0, 0)` when unbound).
    fn joypad_axis(&self, _joypad: i32, _axis: JoypadAxis2dBinding) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// `CharacterJoypadAxis2dBinding.isApplied(joypad)` (outside the dead
    /// zone).
    fn joypad_axis_applied(&self, _joypad: i32, _axis: JoypadAxis2dBinding) -> bool {
        false
    }
    /// `CharacterJoypadButtonBinding.<name>.isDown(joypad)`, by the enum's
    /// name (`"Aim"`, `"PrecisionAim"`, `"Melee"`, `"Attack"`, `"Run"`,
    /// `"Interact"`, `"Crouch"`, `"ReloadWeapon"`, `"RackFirearm"`,
    /// `"Sprint"`, `"CancelAction"`, `"ManualFloorAtk"`, `"ZoomIn"`,
    /// `"ZoomOut"`, ...), the binding (button or trigger/stick threshold)
    /// resolved by the adapter.
    fn joypad_button_down(&self, _joypad: i32, _binding: &str) -> bool {
        false
    }

    // --- options (Core) -------------------------------------------------------

    /// `Core.isToggleToAim()`.
    fn toggle_to_aim(&self) -> bool {
        false
    }
    /// `Core.isToggleToRun()`.
    fn toggle_to_run(&self) -> bool {
        false
    }
    /// `Core.isToggleToSprint()`.
    fn toggle_to_sprint(&self) -> bool {
        false
    }
    /// `Core.isOptiondblTapJogToSprint()`.
    fn dbl_tap_jog_to_sprint(&self) -> bool {
        false
    }
    /// `Core.isSelectingAll()`.
    fn is_selecting_all(&self) -> bool {
        false
    }

    // --- screen / time hooks (rendering) -----------------------------------------

    /// `Core.getZoom(playerIndex)`.
    fn zoom(&self, _player_index: i32) -> f32 {
        1.0
    }
    /// `IsoUtils.XToIso/YToIso(playerIndex, sx, sy, z)`: a screen point (in
    /// zoomed pixels, as `AimingReticle.getX/getY` give) to world x/y at
    /// height `z`, with the player's camera offset and zoom.
    fn screen_to_iso(&self, _player_index: i32, _sx: i32, _sy: i32, _z: f32) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// `IsoCamera.getScreenWidth/getScreenHeight(playerIndex)` (the
    /// player's view in pixels).
    fn screen_size(&self, _player_index: i32) -> (i32, i32) {
        (0, 0)
    }
    /// `PlayerCamera.XToScreenExact/YToScreenExact(x, y, z, 0)`: a world
    /// point to the player's screen (zoomed pixels).
    fn iso_to_screen(&self, _player_index: i32, _x: f32, _y: f32, _z: f32) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// `System.currentTimeMillis()` for `TimedInputHandler` clicks.
    fn current_time_ms(&self) -> i64 {
        0
    }
}

/// No input at all (an NPC, a loading screen, tests).
pub struct NoInput;

impl PlayerInput for NoInput {}
