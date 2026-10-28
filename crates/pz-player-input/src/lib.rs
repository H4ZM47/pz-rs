//! The engine's [`PlayerInput`]: what `IsoPlayer.update` reads from the
//! keyboard, mouse and joypads, implemented over the existing input layer.
//!
//! [`EnginePlayerInput::capture`] copies one frame of state after the input
//! updates (`bind_input::update` and `after_ui_update`) and the camera
//! update:
//!
//! - keyboard: `GameKeyboard.down/lastDown/doLuaKeyPressed`
//!   (`bind_input::game_keyboard_state`), the key bindings of `Core`
//!   (`getKeyBinding` plus `invalidBindingShiftCtrl`, evaluated once per
//!   binding against this frame's held modifiers) and
//!   `Core.currentTextEntryBox`;
//! - mouse: `Mouse`'s statics (`bind_input::mouse_snapshot`) and
//!   `UIManager.isMouseOverInventory`;
//! - joypads: `GameWindow.GameInput`, `JoypadManager` and the character
//!   bindings (`bind_joypad::joypad_snapshot` / `character_bindings`);
//! - `Core` options and `Core.getZoom` for the four players;
//! - the camera: there is no `IsoCamera` global in the port, so the engine
//!   publishes each player's offsets every frame with [`set_camera_info`]
//!   (after the camera update; the aiming reticle lives on the character,
//!   `player.aiming_position`);
//! - `Core`'s screen size and `IsoPlayer.numPlayers` for the split-screen
//!   views (`IsoCamera.getScreenWidth/getScreenHeight`);
//! - `System.currentTimeMillis()` at capture time.
//!
//! Everything `IsoPlayer.update` reads is fixed for the frame in the
//! original too (the statics change only in the per-frame updates), so a
//! copy answers the same and does not hold any thread-local borrowed while
//! the player update calls into Lua.

pub mod joypad;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use pz_characters::character::player::{JoypadAxis2dBinding, PlayerInput};
use pz_core::core::{Core, KeyBinding};
use pz_lua_api::bind_input::MouseSnapshot;

pub use joypad::JoypadFrame;

/// `Mouse.BTN_OFFSET`: key codes at or above it are mouse buttons.
pub const BTN_OFFSET: i32 = 10000;

/// One player's `PlayerCamera` offsets (`IsoCamera.cameras[i]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraInfo {
    /// `offX` / `offY`.
    pub off_x: f32,
    pub off_y: f32,
    /// `rightClickX` / `rightClickY` (the look-ahead pan).
    pub right_click_x: f32,
    pub right_click_y: f32,
}

impl CameraInfo {
    /// `PlayerCamera.getOffX()`: `(int)(offX + rightClickX)`.
    pub fn get_off_x(&self) -> f32 {
        (self.off_x + self.right_click_x) as i32 as f32
    }

    /// `PlayerCamera.getOffY()`.
    pub fn get_off_y(&self) -> f32 {
        (self.off_y + self.right_click_y) as i32 as f32
    }
}

/// What the engine publishes per frame for [`EnginePlayerInput::capture`].
#[derive(Clone, Copy, Debug, Default)]
struct View {
    cameras: [CameraInfo; 4],
}

thread_local! {
    static VIEW: RefCell<View> = RefCell::new(View::default());
}

/// Sets player `player_index`'s camera offsets for this frame (the engine
/// calls it after the camera update; out-of-range players are ignored).
pub fn set_camera_info(player_index: usize, info: CameraInfo) {
    VIEW.with(|v| {
        if let Some(c) = v.borrow_mut().cameras.get_mut(player_index) {
            *c = info;
        }
    });
}

/// A key binding as `GameKeyboard.isKeyDown(String)` sees it this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundKeys {
    /// `keyValue`.
    pub key: i32,
    /// `altKey`.
    pub alt: i32,
    /// `Core.invalidBindingShiftCtrl(keyB)` with this frame's modifiers.
    pub invalid: bool,
}

/// `Core.getKeyBinding(name)` for every bound name.
#[derive(Clone, Debug, Default)]
pub struct KeyBindings {
    pub by_name: HashMap<String, BoundKeys>,
    /// The answer for unbound names (`KEYBINDING_EMPTY`).
    pub empty: Option<BoundKeys>,
}

impl KeyBindings {
    /// Evaluates every binding of `core` against its current
    /// `keysDownRaw` / mouse buttons.
    pub fn from_core(core: &Core) -> Self {
        let eval = |b: &KeyBinding| BoundKeys {
            key: b.key_value,
            alt: b.alt_key,
            invalid: core.invalid_binding_shift_ctrl(b),
        };
        KeyBindings {
            by_name: core
                .key_maps()
                .iter()
                .map(|(n, b)| (n.clone(), eval(b)))
                .collect(),
            empty: Some(eval(&KeyBinding::empty())),
        }
    }

    fn get(&self, name: &str) -> BoundKeys {
        self.by_name
            .get(name)
            .copied()
            .or(self.empty)
            .unwrap_or(BoundKeys {
                key: 0,
                alt: 0,
                invalid: true,
            })
    }
}

/// `GameKeyboard`'s statics.
#[derive(Clone, Debug, Default)]
pub struct KeyboardFrame {
    /// `down` (`None` before the first `GameKeyboard.update`).
    pub down: Option<Vec<bool>>,
    /// `lastDown`.
    pub last_down: Vec<bool>,
    /// `doLuaKeyPressed`.
    pub do_lua_key_pressed: bool,
    /// `Core.currentTextEntryBox != null && currentTextEntryBox.isDoingTextEntry()`.
    pub doing_text_entry: bool,
}

/// `Core` input options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputOptions {
    pub toggle_to_aim: bool,
    pub toggle_to_run: bool,
    pub toggle_to_sprint: bool,
    pub dbl_tap_jog_to_sprint: bool,
    pub selecting_all: bool,
}

impl InputOptions {
    pub fn from_core(core: &Core) -> Self {
        InputOptions {
            toggle_to_aim: core.is_toggle_to_aim(),
            toggle_to_run: core.is_toggle_to_run(),
            toggle_to_sprint: core.is_toggle_to_sprint(),
            dbl_tap_jog_to_sprint: core.is_optiondbl_tap_jog_to_sprint(),
            selecting_all: core.is_selecting_all(),
        }
    }
}

/// One frame of player input. Construct with [`capture`](Self::capture) in
/// the engine, or field by field (tests, replays).
#[derive(Clone, Debug, Default)]
pub struct EnginePlayerInput {
    pub keyboard: KeyboardFrame,
    pub key_bindings: KeyBindings,
    pub mouse: MouseSnapshot,
    /// `UIManager.isMouseOverInventory()`.
    pub mouse_over_inventory: bool,
    pub joypads: JoypadFrame,
    pub options: InputOptions,
    /// `Core.getZoom(i)` for players 0..3.
    pub zoom: [f32; 4],
    /// `IsoCamera.cameras`.
    pub cameras: [CameraInfo; 4],
    /// `Core.getScreenWidth()` / `getScreenHeight()`.
    pub screen_width: i32,
    pub screen_height: i32,
    /// `IsoPlayer.numPlayers`.
    pub num_players: i32,
    /// `System.currentTimeMillis()`.
    pub time_ms: i64,
    /// Mouse buttons a UI-checked query saw released (bit per button):
    /// `Mouse.isButtonDownUICheck` clears their `uiCaptured`, which
    /// [`write_back`](Self::write_back) applies to the live state.
    pub released_checked: Cell<u16>,
}

impl EnginePlayerInput {
    /// Copies this frame's input from the engine globals (see the module
    /// docs for the order the engine runs things in).
    pub fn capture() -> Self {
        let core = pz_lua_api::bind_core::shared_core();
        let core = core.borrow();
        let (down, last_down, do_lua_key_pressed) = pz_lua_api::bind_input::game_keyboard_state();
        let doing_text_entry = core
            .current_text_entry_box
            .as_ref()
            .is_some_and(|b| b.doing_text_entry);
        let (input, jm) = pz_lua_api::bind_joypad::joypad_snapshot();
        let (buttons, axes) = pz_lua_api::bind_joypad::character_bindings();
        let view = VIEW.with(|v| *v.borrow());
        EnginePlayerInput {
            keyboard: KeyboardFrame {
                down,
                last_down,
                do_lua_key_pressed,
                doing_text_entry,
            },
            key_bindings: KeyBindings::from_core(&core),
            mouse: pz_lua_api::bind_input::mouse_snapshot(),
            mouse_over_inventory: pz_ui::manager::UIManager::is_mouse_over_inventory(),
            joypads: JoypadFrame {
                input,
                jm,
                buttons,
                axes,
                activated: pz_lua_api::bind_joypad::activated_joypad(),
            },
            options: InputOptions::from_core(&core),
            zoom: std::array::from_fn(|i| core.get_zoom(i as i32)),
            cameras: view.cameras,
            screen_width: core.get_screen_width(),
            screen_height: core.get_screen_height(),
            num_players: core.num_players,
            time_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
            released_checked: Cell::new(0),
        }
    }

    /// Applies the frame's side effect on `Mouse.uiCaptured` (a released
    /// button that was UI-checked is no longer captured) to the live state.
    /// The engine calls it after the player updates.
    pub fn write_back(&self) {
        let bits = self.released_checked.replace(0);
        for n in 0..16 {
            if bits & (1 << n) != 0 {
                pz_lua_api::bind_input::mouse_is_button_down_ui_check(n);
            }
        }
    }

    /// `Mouse.isRightDelay()`.
    fn mouse_right_delay(&self) -> bool {
        let m = &self.mouse;
        !m.ui_captured[1]
            && m.down
                .as_ref()
                .is_some_and(|d| d.get(1).copied().unwrap_or(false))
            && m.time_right_pressed >= 0.15
    }

    /// `Mouse.isButtonDownUICheck(n)`. Clearing `uiCaptured[n]` for a
    /// released button is recorded for [`write_back`](Self::write_back)
    /// (it cannot change an answer within the frame: the button stays up).
    fn mouse_button_down_ui_check(&self, n: i32) -> bool {
        let Some(down) = &self.mouse.down else {
            return false;
        };
        let Some(i) = usize::try_from(n).ok() else {
            return false;
        };
        let b = down.get(i).copied().unwrap_or(false);
        if !b {
            if i < 16 {
                self.released_checked
                    .set(self.released_checked.get() | (1 << i));
            }
        } else if self.mouse.ui_captured.get(i).copied().unwrap_or(false) {
            return false;
        }
        if n == 1 {
            return self.mouse_right_delay();
        }
        b
    }

    /// `Mouse.wasButtonDown(n)`.
    fn mouse_was_button_down(&self, n: i32) -> bool {
        self.mouse.down.is_some()
            && usize::try_from(n)
                .ok()
                .and_then(|i| self.mouse.prev.get(i).copied())
                .unwrap_or(false)
    }
}

fn player_slot(player_index: i32) -> Option<usize> {
    usize::try_from(player_index).ok().filter(|&p| p < 4)
}

impl PlayerInput for EnginePlayerInput {
    /// `GameKeyboard.isKeyDown(String)`.
    fn is_key_down(&self, binding: &str) -> bool {
        let b = self.key_bindings.get(binding);
        !b.invalid && (self.is_key_code_down(b.key) || self.is_key_code_down(b.alt))
    }

    /// `GameKeyboard.whichKeyDown(String)`.
    fn which_key_down(&self, binding: &str) -> i32 {
        if !self.is_key_down(binding) {
            return 0;
        }
        let b = self.key_bindings.get(binding);
        if self.is_key_code_down(b.key) {
            return b.key;
        }
        if self.is_key_code_down(b.alt) {
            return b.alt;
        }
        0
    }

    /// `GameKeyboard.wasKeyDown(String)` (no shift/ctrl check, as in the
    /// original).
    fn was_key_down(&self, binding: &str) -> bool {
        let b = self.key_bindings.get(binding);
        self.was_key_code_down(b.key) || self.was_key_code_down(b.alt)
    }

    /// `GameKeyboard.isKeyDown(int)`.
    fn is_key_code_down(&self, code: i32) -> bool {
        if code >= BTN_OFFSET {
            return self.mouse_button_down_ui_check(code - BTN_OFFSET);
        }
        if self.keyboard.doing_text_entry {
            return false;
        }
        self.keyboard
            .down
            .as_ref()
            .and_then(|d| usize::try_from(code).ok().and_then(|k| d.get(k).copied()))
            .unwrap_or(false)
    }

    /// `GameKeyboard.wasKeyDown(int)`.
    fn was_key_code_down(&self, code: i32) -> bool {
        if code >= BTN_OFFSET {
            return self.mouse_was_button_down(code - BTN_OFFSET);
        }
        if self.keyboard.doing_text_entry || self.keyboard.down.is_none() {
            return false;
        }
        usize::try_from(code)
            .ok()
            .and_then(|k| self.keyboard.last_down.get(k).copied())
            .unwrap_or(false)
    }

    fn is_doing_text_entry(&self) -> bool {
        self.keyboard.doing_text_entry
    }

    fn do_lua_key_pressed(&self) -> bool {
        self.keyboard.do_lua_key_pressed
    }

    /// `Mouse.getXA()`.
    fn mouse_x(&self) -> i32 {
        self.mouse.x
    }

    /// `Mouse.getYA()`.
    fn mouse_y(&self) -> i32 {
        self.mouse.y
    }

    /// `Mouse.isButtonDown(button)` (no UI check).
    fn is_mouse_button_down(&self, button: i32) -> bool {
        self.mouse
            .down
            .as_ref()
            .and_then(|d| usize::try_from(button).ok().and_then(|i| d.get(i).copied()))
            .unwrap_or(false)
    }

    /// `Mouse.getWheelState()`.
    fn mouse_wheel(&self) -> i32 {
        self.mouse.wheel_delta
    }

    fn is_mouse_over_inventory(&self) -> bool {
        self.mouse_over_inventory
    }

    fn joypad_activated(&self) -> bool {
        self.joypads.activated.is_some()
    }

    fn joypad_axis(&self, joypad: i32, axis: JoypadAxis2dBinding) -> (f32, f32) {
        self.joypads.binding_axis(axis_name(axis), joypad)
    }

    fn joypad_axis_applied(&self, joypad: i32, axis: JoypadAxis2dBinding) -> bool {
        self.joypads.binding_axis_applied(axis_name(axis), joypad)
    }

    fn joypad_button_down(&self, joypad: i32, binding: &str) -> bool {
        self.joypads.binding_down(binding, joypad)
    }

    fn toggle_to_aim(&self) -> bool {
        self.options.toggle_to_aim
    }

    fn toggle_to_run(&self) -> bool {
        self.options.toggle_to_run
    }

    fn toggle_to_sprint(&self) -> bool {
        self.options.toggle_to_sprint
    }

    fn dbl_tap_jog_to_sprint(&self) -> bool {
        self.options.dbl_tap_jog_to_sprint
    }

    fn is_selecting_all(&self) -> bool {
        self.options.selecting_all
    }

    /// `Core.getZoom(playerIndex)` (the index clamped to 0..3 as `Core` does).
    fn zoom(&self, player_index: i32) -> f32 {
        self.zoom[player_index.clamp(0, 3) as usize]
    }

    /// `IsoUtils.XToIso/YToIso(playerIndex, sx, sy, z)`: `sx`/`sy` are
    /// already zoomed (`AimingReticle.getX/getY`), so only the camera
    /// offset is added here.
    fn screen_to_iso(&self, player_index: i32, sx: i32, sy: i32, z: f32) -> (f32, f32) {
        let cam = player_slot(player_index).map_or_else(CameraInfo::default, |p| self.cameras[p]);
        let px = sx as f32 + cam.get_off_x();
        let py = sy as f32 + cam.get_off_y();
        (
            pz_render::iso::x_to_iso(px, py, z),
            pz_render::iso::y_to_iso(px, py, z),
        )
    }

    /// `IsoCamera.getScreenWidth/getScreenHeight(playerIndex)`: half the
    /// width with more than one player, half the height with more than two.
    fn screen_size(&self, _player_index: i32) -> (i32, i32) {
        let w = if self.num_players > 1 {
            self.screen_width / 2
        } else {
            self.screen_width
        };
        let h = if self.num_players > 2 {
            self.screen_height / 2
        } else {
            self.screen_height
        };
        (w, h)
    }

    /// `PlayerCamera.XToScreenExact/YToScreenExact(x, y, z, 0)`:
    /// `IsoUtils.XToScreen/YToScreen` minus the camera offset, in zoomed
    /// pixels (the inverse of [`screen_to_iso`](PlayerInput::screen_to_iso)).
    fn iso_to_screen(&self, player_index: i32, x: f32, y: f32, z: f32) -> (f32, f32) {
        let cam = player_slot(player_index).map_or_else(CameraInfo::default, |p| self.cameras[p]);
        let sx = pz_render::iso::x_to_screen(x, y) - cam.get_off_x();
        let sy = pz_render::iso::y_to_screen(x, y, z, 0) - cam.get_off_y();
        (sx, sy)
    }

    fn current_time_ms(&self) -> i64 {
        self.time_ms
    }
}

/// `CharacterJoypadAxis2dBinding.name()`.
fn axis_name(axis: JoypadAxis2dBinding) -> &'static str {
    match axis {
        JoypadAxis2dBinding::Movement => "Movement",
        JoypadAxis2dBinding::Aiming => "Aiming",
    }
}

#[cfg(test)]
mod tests;
