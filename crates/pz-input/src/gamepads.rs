//! gilrs backend for [`crate::controller::Controllers`]: the GLFW gamepad
//! view (`glfwGetGamepadState`) of each connected device, with the game's
//! `media/gamecontrollerdb.txt` and `<cache>/joypads/gamecontrollerdb.txt`
//! mappings loaded on top of the built-in ones (`Controllers.readGameControllerDB`).

use std::collections::HashMap;
use std::path::Path;

use gilrs::{Axis, Button, GamepadId, Gilrs, GilrsBuilder, MappingSource};

use crate::controller::{Controller, ControllerState, Controllers};

/// GLFW gamepad button order.
const BUTTONS: [Button; 15] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::Select,
    Button::Start,
    Button::Mode,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadRight,
    Button::DPadDown,
    Button::DPadLeft,
];

pub struct Gamepads {
    gilrs: Gilrs,
    slots: HashMap<GamepadId, usize>,
}

/// SDL/GLFW GUID text: the 16 bytes as lowercase hex.
fn guid(uuid: [u8; 16]) -> String {
    uuid.iter().map(|b| format!("{b:02x}")).collect()
}

/// The mapping file without its `#` comment lines.
fn read_db(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().filter(|l| !l.starts_with('#')).map(|l| format!("{l}\n")).collect())
}

impl Gamepads {
    /// `Controllers.create()`: loads the mappings and registers the devices
    /// already connected.
    pub fn new(game_dir: &Path, cache_dir: &Path, controllers: &mut Controllers) -> Option<Self> {
        let mut b = GilrsBuilder::new();
        for p in [game_dir.join("media").join("gamecontrollerdb.txt"), cache_dir.join("joypads").join("gamecontrollerdb.txt")] {
            if let Some(m) = read_db(&p) {
                b = b.add_mappings(&m);
            }
        }
        let gilrs = match b.build() {
            Ok(g) => g,
            Err(gilrs::Error::NotImplemented(g)) => g,
            Err(e) => {
                eprintln!("gamepads unavailable: {e}");
                return None;
            }
        };
        let mut gp = Gamepads { gilrs, slots: HashMap::new() };
        let ids: Vec<GamepadId> = gp.gilrs.gamepads().map(|(id, _)| id).collect();
        for id in ids {
            gp.connect(id, controllers);
        }
        controllers.created = true;
        Some(gp)
    }

    fn connect(&mut self, id: GamepadId, controllers: &mut Controllers) {
        let g = self.gilrs.gamepad(id);
        let is_gamepad = g.mapping_source() != MappingSource::None;
        let c = Controller::new(0, &guid(g.uuid()), Some(g.os_name()), Some(g.name()), is_gamepad);
        if let Some(slot) = controllers.connect(c) {
            self.slots.insert(id, slot);
        }
    }

    /// `Controllers.poll`: handles hot-plugging, then copies each device's
    /// state in GLFW's gamepad layout.
    pub fn poll(&mut self, controllers: &mut Controllers) {
        while let Some(ev) = self.gilrs.next_event() {
            match ev.event {
                gilrs::EventType::Connected => self.connect(ev.id, controllers),
                gilrs::EventType::Disconnected => {
                    if let Some(slot) = self.slots.remove(&ev.id) {
                        controllers.disconnect(slot);
                    }
                }
                _ => {}
            }
        }
        for (&id, &slot) in &self.slots {
            let g = self.gilrs.gamepad(id);
            let Some(c) = controllers.slots[slot].as_mut() else { continue };
            let trig = |b: Button| g.button_data(b).map_or(0.0, |d| d.value()) * 2.0 - 1.0;
            let mut st = ControllerState {
                polled: g.is_connected(),
                axes: [
                    g.value(Axis::LeftStickX),
                    // GLFW's y axes grow downwards.
                    -g.value(Axis::LeftStickY),
                    g.value(Axis::RightStickX),
                    -g.value(Axis::RightStickY),
                    trig(Button::LeftTrigger2),
                    trig(Button::RightTrigger2),
                ],
                buttons: [false; 15],
                hat: 0,
            };
            for (i, b) in BUTTONS.iter().enumerate() {
                st.buttons[i] = g.is_pressed(*b);
            }
            for (bit, b) in [(1u8, Button::DPadUp), (2, Button::DPadRight), (4, Button::DPadDown), (8, Button::DPadLeft)] {
                if g.is_pressed(b) {
                    st.hat |= bit;
                }
            }
            c.state = st;
        }
    }
}

/// Opens the platform's gamepad API and lists devices:
/// `PZ_GAME_DIR=... cargo test -p pz-input --features gilrs -- --ignored`.
#[cfg(test)]
#[test]
#[ignore]
fn opens_gamepads() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
    let mut c = Controllers::default();
    let tmp = std::env::temp_dir();
    let mut g = Gamepads::new(Path::new(&dir), &tmp, &mut c).expect("gilrs");
    g.poll(&mut c);
    eprintln!("connected: {:?}", c.slots.iter().flatten().map(|c| (&c.gamepad_name, &c.guid)).collect::<Vec<_>>());
    assert!(c.created);
}
