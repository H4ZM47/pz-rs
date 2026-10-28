//! Game controllers: `org.lwjglx.input.Controller(s)` (GLFW gamepads: 6
//! axes, 15 buttons, one hat), `zombie.core.input.Input`'s per-tick
//! controller update, and `zombie.input.JoypadManager` with its per-device
//! `joypads/<guid>.config` files.
//!
//! A backend (the `gilrs` module) writes [`ControllerState`]s into
//! [`Controllers`]; [`Input::update_game_thread`] turns them into the
//! original's press/axis/pov notifications and connect events.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// `Controllers.MAX_CONTROLLERS`.
pub const MAX_CONTROLLERS: usize = 16;
/// `Controllers.MAX_AXES` / `MAX_BUTTONS` (GLFW gamepad layout).
pub const MAX_AXES: usize = 6;
pub const MAX_BUTTONS: usize = 15;

/// GLFW gamepad axis names (`Controller.getAxisName`).
const AXIS_NAMES: [&str; MAX_AXES] = ["left stick X", "left stick Y", "right stick X", "right stick Y", "left trigger", "right trigger"];
/// GLFW gamepad button names (`Controller.getButtonName`).
const BUTTON_NAMES: [&str; MAX_BUTTONS] =
    ["A", "B", "X", "Y", "left bumper", "right bumper", "back", "start", "guide", "left stick", "right stick", "d-pad up", "d-pad right", "d-pad down", "d-pad left"];

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// `GLFWGamepadState` plus the joystick's first hat (`GamepadState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControllerState {
    pub polled: bool,
    /// GLFW order and ranges: sticks -1..1 (y down positive), triggers -1..1
    /// (-1 released).
    pub axes: [f32; MAX_AXES],
    pub buttons: [bool; MAX_BUTTONS],
    /// GLFW hat bits: up 1, right 2, down 4, left 8.
    pub hat: u8,
}

/// `org.lwjglx.input.Controller`.
#[derive(Clone, Debug)]
pub struct Controller {
    pub id: usize,
    pub guid: String,
    pub joystick_name: String,
    pub gamepad_name: String,
    pub is_gamepad: bool,
    pub hat_count: usize,
    pub dead_zone: [f32; MAX_AXES],
    pub state: ControllerState,
    /// Bumped on reconnect so the game thread sees a different controller.
    pub generation: u64,
}

impl Controller {
    pub fn new(id: usize, guid: &str, joystick_name: Option<&str>, gamepad_name: Option<&str>, is_gamepad: bool) -> Self {
        Controller {
            id,
            guid: guid.to_string(),
            joystick_name: joystick_name.map_or_else(|| format!("ControllerName{id}"), str::to_string),
            gamepad_name: gamepad_name.map_or_else(|| format!("GamepadName{id}"), str::to_string),
            is_gamepad,
            hat_count: 1,
            dead_zone: [0.2; MAX_AXES],
            state: ControllerState::default(),
            generation: 0,
        }
    }

    pub fn axis_count(&self) -> usize {
        if self.is_gamepad { MAX_AXES } else { 0 }
    }

    pub fn button_count(&self) -> usize {
        if self.is_gamepad { MAX_BUTTONS } else { 0 }
    }

    /// `getAxisValue(axis)`.
    pub fn axis_value(&self, axis: i32) -> f32 {
        if !self.state.polled {
            return 0.0;
        }
        usize::try_from(axis).ok().and_then(|a| self.state.axes.get(a).copied()).unwrap_or(0.0)
    }

    /// `isButtonPressed(index)`.
    pub fn is_button_pressed(&self, index: i32) -> bool {
        self.state.polled && usize::try_from(index).ok().and_then(|i| self.state.buttons.get(i).copied()).unwrap_or(false)
    }

    /// `getPovX()`.
    pub fn pov_x(&self) -> f32 {
        if !self.state.polled {
            0.0
        } else if self.state.hat & 8 != 0 {
            -1.0
        } else if self.state.hat & 2 != 0 {
            1.0
        } else {
            0.0
        }
    }

    /// `getPovY()`.
    pub fn pov_y(&self) -> f32 {
        if !self.state.polled {
            0.0
        } else if self.state.hat & 1 != 0 {
            -1.0
        } else if self.state.hat & 4 != 0 {
            1.0
        } else {
            0.0
        }
    }

    pub fn axis_name(&self, axis: usize) -> Option<&'static str> {
        AXIS_NAMES.get(axis).copied()
    }

    /// `getButtonName(n)`.
    pub fn button_name(&self, n: usize) -> String {
        BUTTON_NAMES.get(n).map_or_else(|| format!("Extra button {}", n - BUTTON_NAMES.len() + 1), |s| s.to_string())
    }

    pub fn dead_zone(&self, axis: usize) -> f32 {
        self.dead_zone.get(axis).copied().unwrap_or(0.0)
    }
}

/// `org.lwjglx.input.Controllers`: the backend's view of connected devices.
#[derive(Debug, Default)]
pub struct Controllers {
    pub slots: [Option<Controller>; MAX_CONTROLLERS],
    pub created: bool,
    next_generation: u64,
}

impl Controllers {
    /// A device appeared; returns its slot (the first free one).
    pub fn connect(&mut self, mut c: Controller) -> Option<usize> {
        let slot = self.slots.iter().position(Option::is_none)?;
        self.next_generation += 1;
        c.id = slot;
        c.generation = self.next_generation;
        self.slots[slot] = Some(c);
        Some(slot)
    }

    pub fn disconnect(&mut self, slot: usize) {
        if let Some(s) = self.slots.get_mut(slot) {
            *s = None;
        }
    }
}

/// What `Input.updateGameThread` reports, in the order it reports it.
#[derive(Clone, Debug, PartialEq)]
pub enum ControllerEvent {
    Disconnected(usize),
    Connected(usize),
    Pressed(usize, usize),
    PressedTrigger(usize, usize),
    PressedAxisNeg(usize, usize),
    PressedAxis(usize, usize),
    Pov(usize),
}

/// `zombie.core.input.Input`'s controller part (game-thread copy).
/// `Clone` lets readers outside Lua take a per-frame snapshot.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub controllers: [Option<Controller>; MAX_CONTROLLERS],
    pressed: [[bool; MAX_BUTTONS]; MAX_CONTROLLERS],
    was_pressed: [[bool; MAX_BUTTONS]; MAX_CONTROLLERS],
    pov: [[f32; 2]; MAX_CONTROLLERS],
}

impl Input {
    pub fn controller(&self, index: i32) -> Option<&Controller> {
        usize::try_from(index).ok().and_then(|i| self.controllers.get(i)).and_then(Option::as_ref)
    }

    pub fn controller_count(&self) -> usize {
        MAX_CONTROLLERS
    }

    pub fn axis_count(&self, index: i32) -> usize {
        self.controller(index).map_or(0, Controller::axis_count)
    }

    pub fn axis_value(&self, index: i32, axis: i32) -> f32 {
        self.controller(index).map_or(0.0, |c| c.axis_value(axis))
    }

    fn check_button(&self, index: i32, button: i32) -> bool {
        self.controller(index).is_some_and(|c| button >= 0 && (button as usize) < c.button_count())
    }

    /// `isButtonPressedD(button, index)`; index -1 means any controller.
    pub fn is_button_pressed_d(&self, button: i32, index: i32) -> bool {
        if index == -1 {
            return (0..MAX_CONTROLLERS as i32).any(|i| self.is_button_pressed_d(button, i));
        }
        self.check_button(index, button) && self.pressed[index as usize][button as usize]
    }

    /// `wasButtonPressed(index, button)`.
    pub fn was_button_pressed(&self, index: i32, button: i32) -> bool {
        self.check_button(index, button) && self.was_pressed[index as usize][button as usize]
    }

    pub fn is_button_start_press(&self, index: i32, button: i32) -> bool {
        !self.was_button_pressed(index, button) && self.is_button_pressed_d(button, index)
    }

    pub fn is_button_release_press(&self, index: i32, button: i32) -> bool {
        self.was_button_pressed(index, button) && !self.is_button_pressed_d(button, index)
    }

    fn any_or(&self, index: i32, f: &dyn Fn(&Controller) -> bool) -> bool {
        if index == -1 {
            return self.controllers.iter().flatten().any(f);
        }
        self.controller(index).is_some_and(f)
    }

    pub fn is_controller_left_d(&self, index: i32) -> bool {
        self.any_or(index, &|c| c.pov_x() < -0.5)
    }

    pub fn is_controller_right_d(&self, index: i32) -> bool {
        self.any_or(index, &|c| c.pov_x() > 0.5)
    }

    pub fn is_controller_up_d(&self, index: i32) -> bool {
        self.any_or(index, &|c| c.pov_y() < -0.5)
    }

    pub fn is_controller_down_d(&self, index: i32) -> bool {
        self.any_or(index, &|c| c.pov_y() > 0.5)
    }

    /// `updateGameThread()`: picks up connects/disconnects (non-gamepads
    /// count as absent), then edges of buttons, axes past ±0.5, trigger
    /// movement and hat changes.
    pub fn update_game_thread(&mut self, polled: &Controllers) -> Vec<ControllerEvent> {
        let mut events = Vec::new();
        let (mut connected, mut disconnected) = (Vec::new(), Vec::new());
        for i in 0..MAX_CONTROLLERS {
            let new = polled.slots[i].as_ref().filter(|c| c.is_gamepad);
            let same = match (&self.controllers[i], new) {
                (Some(a), Some(b)) => a.generation == b.generation,
                (None, None) => true,
                _ => false,
            };
            if same {
                // Same device: take its latest state and settings.
                if let (Some(a), Some(b)) = (self.controllers[i].as_mut(), new) {
                    a.state = b.state.clone();
                }
                continue;
            }
            if new.is_none() && self.controllers[i].is_some() {
                disconnected.push(i);
            }
            if new.is_some() {
                connected.push(i);
            }
            self.controllers[i] = new.cloned();
        }
        events.extend(disconnected.into_iter().map(ControllerEvent::Disconnected));
        events.extend(connected.into_iter().map(ControllerEvent::Connected));
        for i in 0..MAX_CONTROLLERS {
            let Some(c) = self.controllers[i].as_ref() else { continue };
            for b in 0..c.button_count() {
                self.was_pressed[i][b] = self.pressed[i][b];
                let down = c.is_button_pressed(b as i32);
                if self.pressed[i][b] && !down {
                    self.pressed[i][b] = false;
                    continue;
                }
                if self.pressed[i][b] || !down {
                    continue;
                }
                self.pressed[i][b] = true;
                events.push(ControllerEvent::Pressed(i, b));
            }
            for a in 0..c.axis_count() {
                let v = c.axis_value(a as i32);
                if c.is_gamepad && a == 4 || a == 5 {
                    if v > 0.0 {
                        events.push(ControllerEvent::PressedTrigger(i, a));
                    }
                    continue;
                }
                if v < -0.5 {
                    events.push(ControllerEvent::PressedAxisNeg(i, a));
                }
                if v > 0.5 {
                    events.push(ControllerEvent::PressedAxis(i, a));
                }
            }
            let (px, py) = (c.pov_x(), c.pov_y());
            if px == self.pov[i][0] && py == self.pov[i][1] {
                continue;
            }
            self.pov[i] = [px, py];
            events.push(ControllerEvent::Pov(i));
        }
        events
    }
}

/// `JoypadManager.Joypad`: button and axis assignments for one device.
#[derive(Clone, Debug)]
pub struct Joypad {
    pub guid: String,
    pub name: String,
    pub id: usize,
    pub player: i32,
    pub movement_axis_x: i32,
    pub movement_axis_x_flipped: bool,
    pub movement_axis_y: i32,
    pub movement_axis_y_flipped: bool,
    pub movement_axis_dead_zone: f32,
    pub aiming_axis_x: i32,
    pub aiming_axis_x_flipped: bool,
    pub aiming_axis_y: i32,
    pub aiming_axis_y_flipped: bool,
    pub aiming_axis_dead_zone: f32,
    pub a_button: i32,
    pub b_button: i32,
    pub x_button: i32,
    pub y_button: i32,
    pub dpad_up: i32,
    pub dpad_down: i32,
    pub dpad_left: i32,
    pub dpad_right: i32,
    pub bumper_left: i32,
    pub bumper_right: i32,
    pub back: i32,
    pub start: i32,
    pub guide: i32,
    pub left_stick_button: i32,
    pub right_stick_button: i32,
    pub triggers_flipped: bool,
    pub trigger_left: i32,
    pub trigger_right: i32,
    pub disabled: bool,
    pub connected: bool,
    pub last_activity: i64,
}

impl Joypad {
    /// `new Joypad()` with the default assignments (public so readers can
    /// build joypad states without a config file, e.g. in tests).
    pub fn new(id: usize, guid: &str, name: &str) -> Self {
        Joypad {
            guid: guid.to_string(),
            name: name.to_string(),
            id,
            player: -1,
            movement_axis_x: 0,
            movement_axis_x_flipped: false,
            movement_axis_y: 1,
            movement_axis_y_flipped: false,
            movement_axis_dead_zone: 0.0,
            aiming_axis_x: 2,
            aiming_axis_x_flipped: false,
            aiming_axis_y: 3,
            aiming_axis_y_flipped: false,
            aiming_axis_dead_zone: 0.0,
            a_button: 0,
            b_button: 1,
            x_button: 2,
            y_button: 3,
            dpad_up: -1,
            dpad_down: -1,
            dpad_left: -1,
            dpad_right: -1,
            bumper_left: 4,
            bumper_right: 5,
            back: 6,
            start: 7,
            guide: 8,
            left_stick_button: 9,
            right_stick_button: 10,
            triggers_flipped: false,
            trigger_left: 4,
            trigger_right: 5,
            disabled: false,
            connected: true,
            last_activity: 0,
        }
    }

    pub fn touch(&mut self) {
        self.last_activity = now_ms();
    }
}

/// What a [`JoypadManager`] change means for Lua.
#[derive(Clone, Debug, PartialEq)]
pub enum JoypadEvent {
    /// (`OnJoypadBeforeReactivate`, then `OnJoypadReactivate`), joypad id.
    Reactivated(usize),
    /// (`OnJoypadBeforeDeactivate`, then `OnJoypadDeactivate`), joypad id.
    Deactivated(usize),
}

/// `zombie.input.JoypadManager`. `Clone` as for [`Input`].
#[derive(Clone, Debug, Default)]
pub struct JoypadManager {
    /// `joypads`: per player.
    pub joypads: [Option<usize>; 4],
    /// `joypadsController`: per controller slot.
    pub by_controller: [Option<Joypad>; MAX_CONTROLLERS],
    /// `joypadList`: active, enabled joypads by controller slot.
    pub list: Vec<usize>,
    pub active_guids: std::collections::HashSet<String>,
    /// `getCacheDir()`, for `joypads/<guid>.config`.
    pub cache_dir: PathBuf,
}

/// `PZMath.equal(a, b)`.
fn equal(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0e-7
}

impl JoypadManager {
    fn config_path(&self, guid: &str) -> PathBuf {
        self.cache_dir.join("joypads").join(format!("{guid}.config"))
    }

    fn add_joypad(&mut self, input: &mut Input, slot: usize, guid: &str, name: &str) {
        let mut j = Joypad::new(slot, guid, name);
        self.do_controller_file(input, &mut j);
        let active = !j.disabled && self.active_guids.contains(guid);
        self.by_controller[slot] = Some(j);
        if active {
            self.list.push(slot);
        }
    }

    /// `checkJoypad(index)`: creates the joypad on first use of a connected
    /// controller.
    pub fn check(&mut self, input: &mut Input, index: i32) -> Option<usize> {
        let slot = usize::try_from(index).ok().filter(|&s| s < MAX_CONTROLLERS)?;
        if self.by_controller[slot].is_none() {
            if let Some(c) = input.controller(index) {
                let (guid, name) = (c.guid.clone(), c.gamepad_name.clone());
                self.add_joypad(input, slot, &guid, &name);
            }
        }
        self.by_controller[slot].as_ref().map(|_| slot)
    }

    /// `doControllerFile(j)`: reads `joypads/<guid>.config`, then rewrites it.
    fn do_controller_file(&mut self, input: &mut Input, j: &mut Joypad) {
        let path = self.config_path(&j.guid);
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut version = -1;
            for line in pz_lines(&text) {
                let t = line.trim();
                if t.is_empty() || t.starts_with("//") {
                    continue;
                }
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() != 2 {
                    continue;
                }
                let (k, v) = (parts[0].trim(), parts[1].trim());
                if k == "Version" {
                    match v.parse::<i32>() {
                        Ok(n) => version = n,
                        Err(_) => break,
                    }
                    if !(1..=2).contains(&version) || version == 1 {
                        break;
                    }
                }
                if version == -1 {
                    continue;
                }
                let int = |v: &str| v.parse::<i32>().ok();
                let ok = match k {
                    "MovementAxisX" => int(v).map(|n| j.movement_axis_x = n).is_some(),
                    "MovementAxisXFlipped" => {
                        j.movement_axis_x_flipped = v == "true";
                        true
                    }
                    "MovementAxisY" => int(v).map(|n| j.movement_axis_y = n).is_some(),
                    "MovementAxisYFlipped" => {
                        j.movement_axis_y_flipped = v == "true";
                        true
                    }
                    "MovementAxisDeadZone" => v.parse::<f32>().map(|f| j.movement_axis_dead_zone = f).is_ok(),
                    "AimingAxisX" => int(v).map(|n| j.aiming_axis_x = n).is_some(),
                    "AimingAxisXFlipped" => {
                        j.aiming_axis_x_flipped = v == "true";
                        true
                    }
                    "AimingAxisY" => int(v).map(|n| j.aiming_axis_y = n).is_some(),
                    "AimingAxisYFlipped" => {
                        j.aiming_axis_y_flipped = v == "true";
                        true
                    }
                    "AimingAxisDeadZone" => v.parse::<f32>().map(|f| j.aiming_axis_dead_zone = f).is_ok(),
                    "AButton" => int(v).map(|n| j.a_button = n).is_some(),
                    "BButton" => int(v).map(|n| j.b_button = n).is_some(),
                    "XButton" => int(v).map(|n| j.x_button = n).is_some(),
                    "YButton" => int(v).map(|n| j.y_button = n).is_some(),
                    "LBumper" => int(v).map(|n| j.bumper_left = n).is_some(),
                    "RBumper" => int(v).map(|n| j.bumper_right = n).is_some(),
                    "L3" => int(v).map(|n| j.left_stick_button = n).is_some(),
                    "R3" => int(v).map(|n| j.right_stick_button = n).is_some(),
                    "Back" => int(v).map(|n| j.back = n).is_some(),
                    "Start" => int(v).map(|n| j.start = n).is_some(),
                    "DPadUp" => int(v).map(|n| j.dpad_up = n).is_some(),
                    "DPadDown" => int(v).map(|n| j.dpad_down = n).is_some(),
                    "DPadLeft" => int(v).map(|n| j.dpad_left = n).is_some(),
                    "DPadRight" => int(v).map(|n| j.dpad_right = n).is_some(),
                    "TriggersFlipped" => {
                        j.triggers_flipped = v == "true";
                        true
                    }
                    "TriggerLeft" => int(v).map(|n| j.trigger_left = n).is_some(),
                    "TriggerRight" => int(v).map(|n| j.trigger_right = n).is_some(),
                    "Disabled" => {
                        j.disabled = v == "true";
                        true
                    }
                    "Sensitivity" => v.parse::<f32>().map(|f| set_all_dead_zones(input, j.id, f)).is_ok(),
                    _ => true,
                };
                // A malformed number ends the read, as the exception does.
                if !ok {
                    break;
                }
            }
        }
        self.save_file(input, j);
    }

    /// `saveFile(j)`.
    fn save_file(&self, input: &Input, j: &Joypad) {
        let path = self.config_path(&j.guid);
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let sep = if cfg!(windows) { "\r\n" } else { "\n" };
        let f = |v: f32| java_float(v);
        let lines = [
            "Version=2".to_string(),
            format!("Name={}", j.name),
            format!("MovementAxisX={}", j.movement_axis_x),
            format!("MovementAxisXFlipped={}", j.movement_axis_x_flipped),
            format!("MovementAxisY={}", j.movement_axis_y),
            format!("MovementAxisYFlipped={}", j.movement_axis_y_flipped),
            "// Set the dead zone to the smallest number between 0.0 and 1.0.".to_string(),
            "// This is to fix \"loose sticks\".".to_string(),
            format!("MovementAxisDeadZone={}", f(j.movement_axis_dead_zone)),
            format!("AimingAxisX={}", j.aiming_axis_x),
            format!("AimingAxisXFlipped={}", j.aiming_axis_x_flipped),
            format!("AimingAxisY={}", j.aiming_axis_y),
            format!("AimingAxisYFlipped={}", j.aiming_axis_y_flipped),
            format!("AimingAxisDeadZone={}", f(j.aiming_axis_dead_zone)),
            format!("AButton={}", j.a_button),
            format!("BButton={}", j.b_button),
            format!("XButton={}", j.x_button),
            format!("YButton={}", j.y_button),
            format!("LBumper={}", j.bumper_left),
            format!("RBumper={}", j.bumper_right),
            format!("L3={}", j.left_stick_button),
            format!("R3={}", j.right_stick_button),
            format!("Back={}", j.back),
            format!("Start={}", j.start),
            "// Normally the D-pad is treated as a single axis (the POV Hat), and these should be -1.".to_string(),
            "// If your D-pad is actually 4 separate buttons, set the button numbers here.".to_string(),
            format!("DPadUp={}", j.dpad_up),
            format!("DPadDown={}", j.dpad_down),
            format!("DPadLeft={}", j.dpad_left),
            format!("DPadRight={}", j.dpad_right),
            format!("TriggersFlipped={}", j.triggers_flipped),
            "// If your triggers are buttons, set the button numbers here.".to_string(),
            "// If these are set to something other than -1, then Triggers= is ignored.".to_string(),
            format!("TriggerLeft={}", j.trigger_left),
            format!("TriggerRight={}", j.trigger_right),
            format!("Disabled={}", j.disabled),
            format!("Sensitivity={}", f(self.joypad_dead_zone(input, j, 0))),
        ];
        let mut out = String::new();
        for l in lines {
            out.push_str(&l);
            out.push_str(sep);
        }
        let _ = std::fs::write(path, out);
    }

    /// `reloadControllerFiles()`.
    pub fn reload_controller_files(&mut self, input: &mut Input) {
        for i in 0..MAX_CONTROLLERS {
            let Some(c) = input.controllers[i].as_ref() else { continue };
            let (guid, name) = (c.guid.clone(), c.gamepad_name.clone());
            match self.by_controller[i].take() {
                None => self.add_joypad(input, i, &guid, &name),
                Some(mut j) => {
                    self.do_controller_file(input, &mut j);
                    self.by_controller[i] = Some(j);
                }
            }
        }
    }

    /// `saveControllerSettings(index)`.
    pub fn save_controller_settings(&mut self, input: &mut Input, index: i32) {
        if let Some(slot) = self.check(input, index) {
            let j = self.by_controller[slot].clone().unwrap();
            self.save_file(input, &j);
        }
    }

    /// `setControllerActive(guid, active)`.
    pub fn set_controller_active(&mut self, guid: &str, active: bool) {
        if active {
            self.active_guids.insert(guid.to_string());
        } else {
            self.active_guids.remove(guid);
        }
        self.sync_active_controllers();
    }

    /// `syncActiveControllers()`.
    pub fn sync_active_controllers(&mut self) {
        self.list = (0..MAX_CONTROLLERS)
            .filter(|&i| self.by_controller[i].as_ref().is_some_and(|j| !j.disabled && self.active_guids.contains(&j.guid)))
            .collect();
    }

    /// `assignJoypad(index, player)`.
    pub fn assign_joypad(&mut self, input: &mut Input, index: i32, player: usize) {
        if let Some(slot) = self.check(input, index) {
            if let Some(p) = self.joypads.get_mut(player) {
                *p = Some(slot);
            }
            self.by_controller[slot].as_mut().unwrap().player = player as i32;
        }
    }

    /// `onControllerConnected` / `onControllerDisconnected`.
    pub fn on_controller(&mut self, slot: usize, connected: bool) -> Option<JoypadEvent> {
        let j = self.by_controller.get_mut(slot)?.as_mut()?;
        j.connected = connected;
        Some(if connected { JoypadEvent::Reactivated(slot) } else { JoypadEvent::Deactivated(slot) })
    }

    /// Applies one `Input.updateGameThread` notification (`onPressed*`).
    pub fn on_input(&mut self, input: &mut Input, e: &ControllerEvent) {
        let slot = match *e {
            ControllerEvent::Pressed(i, _) | ControllerEvent::PressedAxis(i, _) | ControllerEvent::PressedAxisNeg(i, _) | ControllerEvent::PressedTrigger(i, _) | ControllerEvent::Pov(i) => i,
            _ => return,
        };
        if self.check(input, slot as i32).is_some() {
            self.by_controller[slot].as_mut().unwrap().touch();
        }
    }

    pub fn last_activity(&self, index: usize) -> i64 {
        self.by_controller.get(index).and_then(Option::as_ref).map_or(0, |j| j.last_activity)
    }

    /// `Joypad.getDeadZone(axis)`.
    pub fn joypad_dead_zone(&self, input: &Input, j: &Joypad, axis: i32) -> f32 {
        let mut settings = 0.0;
        if (axis == j.movement_axis_x || axis == j.movement_axis_y) && j.movement_axis_dead_zone > 0.0 && j.movement_axis_dead_zone < 1.0 {
            settings = j.movement_axis_dead_zone;
        }
        if (axis == j.aiming_axis_x || axis == j.aiming_axis_y) && j.aiming_axis_dead_zone > 0.0 && j.aiming_axis_dead_zone < 1.0 {
            settings = j.aiming_axis_dead_zone;
        }
        if axis < 0 || axis as usize >= input.axis_count(j.id as i32) {
            return settings;
        }
        let ctl = input.controller(j.id as i32).map_or(0.0, |c| c.dead_zone(axis as usize));
        ctl.max(settings)
    }
}

/// `Joypad.setDeadZone(float)`: every axis of the controller.
fn set_all_dead_zones(input: &mut Input, id: usize, v: f32) {
    if let Some(c) = input.controllers.get_mut(id).and_then(Option::as_mut) {
        let n = c.axis_count();
        for d in c.dead_zone.iter_mut().take(n) {
            *d = v;
        }
    }
}

/// `Joypad.setDeadZone(axis, value)`.
pub fn set_dead_zone(input: &mut Input, id: usize, axis: i32, v: f32) {
    if let Some(c) = input.controllers.get_mut(id).and_then(Option::as_mut) {
        if axis >= 0 && (axis as usize) < c.axis_count() {
            c.dead_zone[axis as usize] = v;
        }
    }
}

/// `BufferedReader.readLine` lines.
fn pz_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
        out.push(&rest[..end]);
        rest = &rest[end..];
        if rest.starts_with("\r\n") {
            rest = &rest[2..];
        } else if !rest.is_empty() {
            rest = &rest[1..];
        }
    }
    out
}

/// `String.valueOf(float)`.
fn java_float(v: f32) -> String {
    pz_java_float(v)
}

/// Java's shortest-repr float formatting for the values these files hold.
pub fn pz_java_float(v: f32) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let a = v.abs();
    if a != 0.0 && !(1.0e-3..1.0e7).contains(&a) {
        // Scientific form: d.dddE±n.
        let s = format!("{v:e}");
        let (m, e) = s.split_once('e').unwrap();
        let m = if m.contains('.') { m.to_string() } else { format!("{m}.0") };
        return format!("{m}E{e}");
    }
    let s = format!("{v}");
    if s.contains('.') { s } else { format!("{s}.0") }
}

/// Stick processing shared by movement and aiming (`get*AxisXRaw/YRaw`
/// and the squaring scaler of `get*AxisX/Y`).
pub struct Stick {
    pub x_axis: i32,
    pub y_axis: i32,
    pub x_flipped: bool,
    pub y_flipped: bool,
}

impl Stick {
    fn raw(&self, input: &Input, id: i32, want_x: bool) -> f32 {
        let axis = if want_x { self.x_axis } else { self.y_axis };
        if input.axis_count(id) as i32 <= axis {
            return 0.0;
        }
        let dz = input.controller(id).map_or(0.0, |c| c.dead_zone(axis as usize));
        let flipped = if want_x { self.x_flipped } else { self.y_flipped };
        if dz > 0.0 && dz < 1.0 {
            let (mut x, mut y) = (input.axis_value(id, self.x_axis), input.axis_value(id, self.y_axis));
            let len = (x * x + y * y).sqrt();
            if len < dz {
                x = 0.0;
                y = 0.0;
            } else {
                let s = ((len - dz) / (1.0 - dz)) / len;
                x *= s;
                y *= s;
            }
            let v = if want_x { x } else { y };
            return if flipped { -v } else { v };
        }
        let v = input.axis_value(id, axis);
        if flipped { -v } else { v }
    }

    /// `getMovementAxisX/Y` / `getAimingAxisX/Y`.
    pub fn value(&self, input: &Input, id: i32, want_x: bool) -> f32 {
        let a = self.raw(input, id, want_x);
        if equal(a, 0.0) {
            return 0.0;
        }
        let b = self.raw(input, id, !want_x);
        if equal(b, 0.0) {
            return a;
        }
        let (x, y) = if want_x { (a, b) } else { (b, a) };
        (a * squaring_scaler(x, y)).clamp(-1.0, 1.0)
    }
}

/// `Joypad.getAxisSquaringScaler`: up to √2 towards the diagonals.
fn squaring_scaler(x: f32, y: f32) -> f32 {
    let len = (x * x + y * y).sqrt();
    if equal(len, 0.0) {
        return 1.0;
    }
    let (xa, ya) = ((x / len).abs(), (y / len).abs());
    let d = std::f32::consts::FRAC_1_SQRT_2;
    // `PZMath.angleBetweenNormalized`: acos of the dot product.
    let angle = (d * xa + d * ya).clamp(-1.0, 1.0).acos();
    let alpha = (1.0 - angle.abs() * 4.0 / std::f32::consts::PI).clamp(0.0, 1.0);
    // `PZMath.lerp(1, 1.4142f, alpha)`: the original's rounded √2.
    #[allow(clippy::approx_constant)]
    let max = 1.4142;
    1.0 + (max - 1.0) * alpha
}

/// `Joypad.getLTValue/getRTValue`: the trigger axis rescaled past its dead
/// zone, or the bumper when the controller has no such axis.
pub fn trigger_value(input: &Input, j: &Joypad, left: bool) -> f32 {
    let trigger = if left { j.trigger_left } else { j.trigger_right };
    let id = j.id as i32;
    if input.axis_count(id) as i32 <= trigger {
        let bumper = if left { j.bumper_left } else { j.bumper_right };
        return if input.is_button_pressed_d(bumper, id) { 1.0 } else { -1.0 };
    }
    let dz = input.controller(id).map_or(0.0, |c| c.dead_zone(trigger as usize));
    let raw = input.axis_value(id, trigger);
    let raw = if j.triggers_flipped { -raw } else { raw };
    let min = -1.0 + dz;
    (2.0 * (raw - min) / (1.0 - min) - 1.0).clamp(-1.0, 1.0)
}

impl Joypad {
    pub fn movement(&self) -> Stick {
        Stick { x_axis: self.movement_axis_x, y_axis: self.movement_axis_y, x_flipped: self.movement_axis_x_flipped, y_flipped: self.movement_axis_y_flipped }
    }

    pub fn aiming(&self) -> Stick {
        Stick { x_axis: self.aiming_axis_x, y_axis: self.aiming_axis_y, x_flipped: self.aiming_axis_x_flipped, y_flipped: self.aiming_axis_y_flipped }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(slot: usize) -> Controller {
        let mut c = Controller::new(slot, "03000000de280000ff11000001000000", Some("Pad"), Some("Xbox Controller"), true);
        c.state.polled = true;
        c.state.axes[4] = -1.0;
        c.state.axes[5] = -1.0;
        c
    }

    #[test]
    fn connects_presses_and_pov() {
        let mut ctl = Controllers::default();
        let mut input = Input::default();
        let slot = ctl.connect(pad(0)).unwrap();
        assert_eq!(input.update_game_thread(&ctl), [ControllerEvent::Connected(slot)]);
        ctl.slots[slot].as_mut().unwrap().state.buttons[0] = true;
        ctl.slots[slot].as_mut().unwrap().state.hat = 1;
        let ev = input.update_game_thread(&ctl);
        assert_eq!(ev, [ControllerEvent::Pressed(slot, 0), ControllerEvent::Pov(slot)]);
        assert!(input.is_button_start_press(slot as i32, 0));
        assert!(input.is_controller_up_d(-1));
        assert!(input.update_game_thread(&ctl).is_empty());
        assert!(!input.is_button_start_press(slot as i32, 0));
        ctl.disconnect(slot);
        assert_eq!(input.update_game_thread(&ctl), [ControllerEvent::Disconnected(slot)]);
    }

    #[test]
    fn config_file_round_trip_and_dead_zones() {
        let dir = crate::tests::unique_temp_dir("pz-input-joypad");
        let _ = std::fs::remove_dir_all(&dir);
        let mut ctl = Controllers::default();
        let mut input = Input::default();
        let slot = ctl.connect(pad(0)).unwrap();
        input.update_game_thread(&ctl);
        let mut jm = JoypadManager { cache_dir: dir.clone(), ..Default::default() };
        std::fs::create_dir_all(dir.join("joypads")).unwrap();
        let guid = "03000000de280000ff11000001000000";
        std::fs::write(dir.join("joypads").join(format!("{guid}.config")), "Version=2\nAButton=1\nSensitivity=0.3\n").unwrap();
        jm.set_controller_active(guid, true);
        assert_eq!(jm.check(&mut input, slot as i32), Some(slot));
        assert_eq!(jm.list, [slot]);
        let j = jm.by_controller[slot].clone().unwrap();
        assert_eq!(j.a_button, 1);
        assert!((jm.joypad_dead_zone(&input, &j, 0) - 0.3).abs() < 1e-6);
        let saved = std::fs::read_to_string(dir.join("joypads").join(format!("{guid}.config"))).unwrap();
        assert!(saved.contains("AButton=1") && saved.contains("Sensitivity=0.3"), "{saved}");
        // Stick: inside the dead zone reads 0; full tilt reads 1.
        ctl.slots[slot].as_mut().unwrap().state.axes[0] = 0.2;
        input.update_game_thread(&ctl);
        assert_eq!(j.movement().value(&input, slot as i32, true), 0.0);
        ctl.slots[slot].as_mut().unwrap().state.axes[0] = 1.0;
        input.update_game_thread(&ctl);
        assert!((j.movement().value(&input, slot as i32, true) - 1.0).abs() < 1e-6);
        assert_eq!(trigger_value(&input, &j, true), -1.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn java_float_format() {
        assert_eq!(pz_java_float(0.0), "0.0");
        assert_eq!(pz_java_float(0.2), "0.2");
        assert_eq!(pz_java_float(1.0), "1.0");
    }
}
