use super::*;
use pz_input::bindings::{Axis2dBinding, ButtonBinding, IsDown};
use pz_input::controller::{Controller, Controllers, Input, Joypad, JoypadManager};

const SHIFT: i32 = 42;
const CTRL: i32 = 29;
const W: i32 = 17;
const UP: i32 = 200;
const SPACE: i32 = 57;
const A_KEY: i32 = 30;

fn keyboard(down: &[i32], last: &[i32]) -> KeyboardFrame {
    let mut d = vec![false; 256];
    let mut l = vec![false; 256];
    down.iter().for_each(|&k| d[k as usize] = true);
    last.iter().for_each(|&k| l[k as usize] = true);
    KeyboardFrame {
        down: Some(d),
        last_down: l,
        do_lua_key_pressed: true,
        doing_text_entry: false,
    }
}

/// Bindings like the defaults, with `raw` as `GameKeyboard.isKeyDownRaw`.
fn core(raw: &[i32], mouse: &[i32]) -> Core {
    let mut c = Core::new();
    c.add_key_binding("Forward", W, UP, false, false, false);
    c.add_key_binding("Run", SHIFT, 0, false, false, false);
    c.add_key_binding("Melee", SPACE, 0, false, true, false);
    c.add_key_binding("Plain", A_KEY, 0, false, false, false);
    c.add_key_binding("Shifted", A_KEY, 0, true, false, false);
    c.add_key_binding("Attack/Click", BTN_OFFSET, 0, false, false, false);
    c.add_key_binding("Aim", BTN_OFFSET + 1, 0, false, false, false);
    c.keys_down_raw = raw.iter().copied().collect();
    c.mouse_buttons_down = mouse.iter().copied().collect();
    c
}

fn input(down: &[i32], last: &[i32]) -> EnginePlayerInput {
    EnginePlayerInput {
        keyboard: keyboard(down, last),
        key_bindings: KeyBindings::from_core(&core(down, &[])),
        ..Default::default()
    }
}

fn mouse(down: &[usize], prev: &[usize]) -> MouseSnapshot {
    let mut d = vec![false; 8];
    let mut p = vec![false; 8];
    down.iter().for_each(|&b| d[b] = true);
    prev.iter().for_each(|&b| p[b] = true);
    MouseSnapshot {
        x: 0,
        y: 0,
        down: Some(d),
        prev: p,
        wheel_delta: 0,
        ui_captured: [false; 10],
        time_right_pressed: 0.0,
    }
}

#[test]
fn key_down_by_binding_and_alt_key() {
    let i = input(&[W], &[]);
    assert!(i.is_key_down("Forward"));
    assert_eq!(i.which_key_down("Forward"), W);
    let i = input(&[UP], &[]);
    assert!(i.is_key_down("Forward"));
    assert_eq!(i.which_key_down("Forward"), UP);
    let i = input(&[], &[]);
    assert!(!i.is_key_down("Forward"));
    assert_eq!(i.which_key_down("Forward"), 0);
    // Unknown bindings are KEYBINDING_EMPTY: never down.
    assert!(!i.is_key_down("NoSuchBinding"));
    assert!(!i.was_key_down("NoSuchBinding"));
}

#[test]
fn was_down_and_pressed() {
    let i = input(&[W], &[]);
    assert!(i.is_key_pressed("Forward"));
    assert!(!i.was_key_down("Forward"));
    let i = input(&[W], &[UP]);
    assert!(i.was_key_down("Forward"));
    assert!(!i.is_key_pressed("Forward"));
    assert!(i.was_key_code_down(UP));
    assert!(!i.was_key_code_down(W));
}

#[test]
fn ctrl_modifier_invalidates_until_held() {
    // Melee is ctrl+space: space alone is not it.
    assert!(!input(&[SPACE], &[]).is_key_down("Melee"));
    assert!(input(&[SPACE, CTRL], &[]).is_key_down("Melee"));
    // A binding on the shift key itself needs shift held (raw).
    assert!(input(&[SHIFT], &[]).is_key_down("Run"));
}

#[test]
fn shift_conflict_invalidates_the_plain_binding() {
    // `Plain` (A) and `Shifted` (shift+A) share a key: with shift held only
    // the shifted one counts, without it only the plain one.
    let i = input(&[A_KEY], &[]);
    assert!(i.is_key_down("Plain"));
    assert!(!i.is_key_down("Shifted"));
    let i = input(&[A_KEY, SHIFT], &[]);
    assert!(!i.is_key_down("Plain"));
    assert!(i.is_key_down("Shifted"));
    // wasKeyDown(String) does not check modifiers.
    let i = input(&[SHIFT], &[A_KEY]);
    assert!(i.was_key_down("Plain"));
}

#[test]
fn text_entry_hides_keys_but_not_mouse_buttons() {
    let mut i = input(&[W], &[W]);
    i.mouse = mouse(&[0], &[0]);
    i.keyboard.doing_text_entry = true;
    assert!(i.is_doing_text_entry());
    assert!(!i.is_key_code_down(W));
    assert!(!i.was_key_code_down(W));
    assert!(!i.is_key_down("Forward"));
    assert!(i.is_key_code_down(BTN_OFFSET));
    assert!(i.was_key_code_down(BTN_OFFSET));
}

#[test]
fn before_first_keyboard_update_nothing_is_down() {
    let i = EnginePlayerInput::default();
    assert!(!i.is_key_code_down(W));
    assert!(!i.was_key_code_down(W));
    assert!(!i.is_mouse_button_down(0));
    assert!(!i.is_key_code_down(BTN_OFFSET));
}

#[test]
fn mouse_buttons_as_key_codes() {
    let mut i = input(&[], &[]);
    i.mouse = mouse(&[0], &[]);
    assert!(i.is_key_code_down(BTN_OFFSET));
    assert!(i.is_key_down("Attack/Click"));
    assert!(i.is_key_pressed("Attack/Click"));
    assert_eq!(i.which_key_down("Attack/Click"), BTN_OFFSET);
    // Captured by the UI: down for `Mouse.isButtonDown`, not for the game.
    i.mouse.ui_captured[0] = true;
    assert!(i.is_mouse_button_down(0));
    assert!(!i.is_key_code_down(BTN_OFFSET));
}

#[test]
fn right_button_needs_the_delay() {
    let mut i = input(&[], &[]);
    i.mouse = mouse(&[1], &[1]);
    i.mouse.time_right_pressed = 0.1;
    assert!(i.is_mouse_button_down(1));
    assert!(!i.is_key_down("Aim"));
    i.mouse.time_right_pressed = 0.15;
    assert!(i.is_key_down("Aim"));
    i.mouse.ui_captured[1] = true;
    assert!(!i.is_key_down("Aim"));
}

#[test]
fn released_ui_checked_buttons_are_recorded_for_write_back() {
    let mut i = input(&[], &[]);
    i.mouse = mouse(&[], &[2]);
    i.mouse.ui_captured[2] = true;
    assert!(!i.is_key_code_down(BTN_OFFSET + 2));
    assert_eq!(i.released_checked.get(), 1 << 2);
}

#[test]
fn mouse_position_wheel_and_inventory() {
    let mut i = EnginePlayerInput {
        mouse_over_inventory: true,
        ..Default::default()
    };
    i.mouse.x = 640;
    i.mouse.y = 359;
    i.mouse.wheel_delta = -2;
    assert_eq!((i.mouse_x(), i.mouse_y()), (640, 359));
    assert_eq!(i.mouse_wheel(), -2);
    assert!(i.is_mouse_over_inventory());
}

#[test]
fn options_and_lua_key_flag() {
    let mut c = Core::new();
    c.set_toggle_to_aim(true);
    c.set_toggle_to_sprint(true);
    c.set_optiondbl_tap_jog_to_sprint(true);
    c.set_is_selecting_all(true);
    let mut i = EnginePlayerInput {
        options: InputOptions::from_core(&c),
        ..Default::default()
    };
    assert!(i.toggle_to_aim());
    assert!(!i.toggle_to_run());
    assert!(i.toggle_to_sprint());
    assert!(i.dbl_tap_jog_to_sprint());
    assert!(i.is_selecting_all());
    assert!(!i.do_lua_key_pressed());
    i.keyboard.do_lua_key_pressed = true;
    assert!(i.do_lua_key_pressed());
}

#[test]
fn zoom_time_and_aiming_positions() {
    let mut i = EnginePlayerInput {
        zoom: [1.0, 1.5, 2.0, 0.5],
        time_ms: 1234,
        ..Default::default()
    };
    i.aiming_positions[1] = (320.0, 200.0);
    assert_eq!(i.zoom(1), 1.5);
    // Core.getZoom clamps the player index.
    assert_eq!(i.zoom(-1), 1.0);
    assert_eq!(i.zoom(7), 0.5);
    assert_eq!(i.current_time_ms(), 1234);
    assert_eq!(i.joypad_aiming_position(1), (320.0, 200.0));
    assert_eq!(i.joypad_aiming_position(5), (0.0, 0.0));
}

#[test]
fn screen_to_iso_matches_iso_utils() {
    let mut i = EnginePlayerInput::default();
    // getOffX = (int)(100.4 + 0.3) = 100, getOffY = (int)(50.9 + 0) = 50.
    i.cameras[0] = CameraInfo {
        off_x: 100.4,
        off_y: 50.9,
        right_click_x: 0.3,
        right_click_y: 0.0,
    };
    // px = 28 + 100 = 128, py = 14 + 50 = 64:
    // XToIso = (128 + 128) / 128 + 3 * 1 = 5, YToIso = (128 - 128) / -128 + 3 = 3.
    assert_eq!(i.screen_to_iso(0, 28, 14, 1.0), (5.0, 3.0));
    // Player 2, camera (64, 0), screen origin, floor 0: (0.5, -0.5).
    i.cameras[2] = CameraInfo {
        off_x: 64.0,
        ..Default::default()
    };
    assert_eq!(i.screen_to_iso(2, 0, 0, 0.0), (0.5, -0.5));
    // Negative right-click pan truncates towards zero: (int)(10 - 10.5) = 0.
    i.cameras[3] = CameraInfo {
        off_x: 10.0,
        right_click_x: -10.5,
        ..Default::default()
    };
    assert_eq!(i.cameras[3].get_off_x(), 0.0);
    // (0 + 2 * 64) / 128 = 1, (0 - 128) / -128 = 1.
    assert_eq!(i.screen_to_iso(3, 0, 64, 0.0), (1.0, 1.0));
}

#[test]
fn capture_reads_the_engine_globals() {
    set_camera_info(
        1,
        CameraInfo {
            off_x: 3.0,
            ..Default::default()
        },
    );
    set_aiming_position(1, 7.0, 8.0);
    set_camera_info(9, CameraInfo::default());
    {
        let core = pz_lua_api::bind_core::shared_core();
        let mut c = core.borrow_mut();
        c.add_key_binding("Forward", W, UP, false, false, false);
        c.set_toggle_to_run(true);
    }
    let i = EnginePlayerInput::capture();
    assert_eq!(i.cameras[1].off_x, 3.0);
    assert_eq!(i.joypad_aiming_position(1), (7.0, 8.0));
    assert_eq!(
        i.key_bindings.by_name["Forward"],
        BoundKeys {
            key: W,
            alt: UP,
            invalid: false
        }
    );
    assert!(i.toggle_to_run());
    // No GameKeyboard.update has run on this thread: nothing is down.
    assert!(!i.is_key_down("Forward"));
    assert!(i.time_ms > 0);
}

// ---- joypads ---------------------------------------------------------------

const GUID: &str = "030000005e0400008e02000014010000";

fn button(name: &str, b: IsDown) -> ButtonBinding {
    ButtonBinding {
        name: name.into(),
        default: Some(b.clone()),
        binding: Some(b),
    }
}

fn axis(name: &str, a: Option<&str>) -> Axis2dBinding {
    Axis2dBinding {
        name: name.into(),
        default: a.map(str::to_string),
        binding: a.map(str::to_string),
    }
}

/// One gamepad in slot 0 with `axes` (sticks then triggers) and `buttons`
/// held, joypad 0 active, and bindings in the shape of the defaults.
fn pads(axes: [f32; 6], buttons: &[usize], hat: u8) -> EnginePlayerInput {
    let mut ctl = Controllers::default();
    let mut c = Controller::new(0, GUID, Some("Pad"), Some("XBOX 360 Controller"), true);
    c.state.polled = true;
    c.state.axes = axes;
    buttons.iter().for_each(|&b| c.state.buttons[b] = true);
    c.state.hat = hat;
    ctl.connect(c).unwrap();
    let mut input = Input::default();
    input.update_game_thread(&ctl);
    let mut jm = JoypadManager::default();
    jm.by_controller[0] = Some(Joypad::new(0, GUID, "XBOX 360 Controller"));
    jm.list = vec![0];
    let buttons = vec![
        button("Interact", IsDown::Button("A".into())),
        button("Melee", IsDown::Button("B".into())),
        button("Crouch", IsDown::Button("DPadDown".into())),
        button("ZoomIn", IsDown::Button("LeftBump".into())),
        button("Aim", IsDown::Axis1d("LeftTrigger".into(), 0.5, f32::MAX)),
        button(
            "Attack",
            IsDown::Axis1d("RightTrigger".into(), 0.5, f32::MAX),
        ),
        button("Run", IsDown::Axis2d("LeftStick".into(), 0.3, 0.9)),
        button("Unbound", IsDown::Button("A".into())),
    ];
    let mut frame = JoypadFrame {
        input,
        jm,
        buttons,
        axes: vec![
            axis("Movement", Some("LeftStick")),
            axis("Aiming", Some("RightStick")),
        ],
        activated: Some(0),
    };
    frame.buttons[7].binding = None;
    EnginePlayerInput {
        joypads: frame,
        ..Default::default()
    }
}

const REST: [f32; 6] = [0.0, 0.0, 0.0, 0.0, -1.0, -1.0];

#[test]
fn joypad_activation() {
    assert!(pads(REST, &[], 0).joypad_activated());
    assert!(!EnginePlayerInput::default().joypad_activated());
}

#[test]
fn joypad_button_bindings() {
    let i = pads(REST, &[0], 0);
    assert!(i.joypad_button_down(0, "Interact"));
    assert!(!i.joypad_button_down(0, "Melee"));
    // A null binding and an unknown name are up.
    assert!(!i.joypad_button_down(0, "Unbound"));
    assert!(!i.joypad_button_down(0, "NoSuchBinding"));
    // No such joypad.
    assert!(!i.joypad_button_down(3, "Interact"));
    // Face buttons accept -1 (any active joypad); bumpers and the d-pad don't.
    assert!(i.joypad_button_down(-1, "Interact"));
    let i = pads(REST, &[4], 4);
    assert!(i.joypad_button_down(0, "ZoomIn"));
    assert!(!i.joypad_button_down(-1, "ZoomIn"));
    // The d-pad is the hat when DPad* are -1 (hat 4 = down).
    assert!(i.joypad_button_down(0, "Crouch"));
    assert!(!i.joypad_button_down(-1, "Crouch"));
}

#[test]
fn joypad_trigger_thresholds() {
    // Trigger rescaled past the 0.2 dead zone: raw 0.5 -> 2 * 1.3 / 1.8 - 1 = 0.444 < 0.5.
    let i = pads([0.0, 0.0, 0.0, 0.0, 0.5, 1.0], &[], 0);
    assert!((i.joypads.trigger(0, true) - 0.444_444_4).abs() < 1e-6);
    assert!(!i.joypad_button_down(0, "Aim"));
    assert!(i.joypad_button_down(0, "Attack"));
    let i = pads(REST, &[], 0);
    assert_eq!(i.joypads.trigger(0, true), -1.0);
    assert!(!i.joypad_button_down(0, "Attack"));
}

#[test]
fn joypad_stick_length_threshold() {
    // (0.5, 0) past the 0.2 dead zone: (0.5 - 0.2) / 0.8 = 0.375, inside [0.3, 0.9).
    assert!(pads([0.5, 0.0, 0.0, 0.0, -1.0, -1.0], &[], 0).joypad_button_down(0, "Run"));
    // Full tilt: length 1 is not below the 0.9 maximum.
    assert!(!pads([1.0, 0.0, 0.0, 0.0, -1.0, -1.0], &[], 0).joypad_button_down(0, "Run"));
    assert!(!pads(REST, &[], 0).joypad_button_down(0, "Run"));
}

#[test]
fn joypad_movement_axis_and_dead_zone() {
    let i = pads([0.5, 0.0, 0.0, 0.0, -1.0, -1.0], &[], 0);
    let (x, y) = i.joypad_axis(0, JoypadAxis2dBinding::Movement);
    assert!((x - 0.375).abs() < 1e-6 && y == 0.0, "{x} {y}");
    assert!(i.joypad_axis_applied(0, JoypadAxis2dBinding::Movement));
    // Inside the radial dead zone: zero and not applied.
    let i = pads([0.1, 0.1, 0.0, 0.0, -1.0, -1.0], &[], 0);
    assert_eq!(i.joypad_axis(0, JoypadAxis2dBinding::Movement), (0.0, 0.0));
    assert!(!i.joypad_axis_applied(0, JoypadAxis2dBinding::Movement));
    // Diagonal full tilt: squaring scaler then clamp to 1.
    let i = pads([0.8, 0.8, 0.0, 0.0, -1.0, -1.0], &[], 0);
    let (x, y) = i.joypad_axis(0, JoypadAxis2dBinding::Movement);
    assert_eq!((x, y), (1.0, 1.0));
}

#[test]
fn joypad_aiming_dead_zone_is_stricter() {
    // (0.25, 0): past the stick's radial dead zone (0.0625), but the aiming
    // check needs x^2 + y^2 >= 0.2^2 + 0.2^2 = 0.08: not applied, so zero.
    let i = pads([0.0, 0.0, 0.25, 0.0, -1.0, -1.0], &[], 0);
    assert!(!i.joypad_axis_applied(0, JoypadAxis2dBinding::Aiming));
    assert_eq!(i.joypad_axis(0, JoypadAxis2dBinding::Aiming), (0.0, 0.0));
    // Full tilt up on the aiming stick is applied (y down positive).
    let i = pads([0.0, 0.0, 0.0, -1.0, -1.0, -1.0], &[], 0);
    assert!(i.joypad_axis_applied(0, JoypadAxis2dBinding::Aiming));
    assert_eq!(i.joypad_axis(0, JoypadAxis2dBinding::Aiming), (0.0, -1.0));
}

#[test]
fn joypad_axis_follows_the_character_binding() {
    let mut i = pads([0.0, 0.0, 1.0, 0.0, -1.0, -1.0], &[], 0);
    // Movement rebound to the right stick reads it.
    i.joypads.axes[0].binding = Some("RightStick".into());
    assert_eq!(i.joypad_axis(0, JoypadAxis2dBinding::Movement), (1.0, 0.0));
    // Unbound: zero and not applied.
    i.joypads.axes[1].binding = None;
    assert_eq!(i.joypad_axis(0, JoypadAxis2dBinding::Aiming), (0.0, 0.0));
    assert!(!i.joypad_axis_applied(0, JoypadAxis2dBinding::Aiming));
}
