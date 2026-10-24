//! Controllers: `zombie.input.JoypadManager`, the `JoypadButton`,
//! `JoypadAxis1d` and `JoypadAxis2d` enums (constant names read from the
//! jar), `GameWindow.activatedJoyPad` and the controller globals of
//! `GlobalObject`.
//!
//! The window layer runs `pz_input::gamepads::Gamepads::poll` into
//! `pz_input::with_controllers`; [`update`] (`GameInput.updateGameThread`)
//! and [`after_ui_update`] (joypad activation) run from
//! `bind_input::update` / `bind_input::after_ui_update`.

use std::cell::RefCell;
use std::path::Path;

use pz_input::controller::{set_dead_zone, trigger_value, ControllerEvent, Input, Joypad, JoypadEvent, JoypadManager};
use pz_input::with_controllers;
use pz_kahlua::{LuaError, NativeCall, Thread, Value};

use crate::expose::{expose_enum, global, int_arg, num_arg, with_this, JavaEnum};
use crate::LuaManager;

pub const JOYPAD_BUTTON: &str = "zombie.input.JoypadButton";
pub const JOYPAD_AXIS_1D: &str = "zombie.input.JoypadAxis1d";
pub const JOYPAD_AXIS_2D: &str = "zombie.input.JoypadAxis2d";

struct State {
    input: Input,
    jm: JoypadManager,
    /// `GameWindow.activatedJoyPad` (controller slot).
    activated: Option<usize>,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State { input: Input::default(), jm: JoypadManager::default(), activated: None });
}

fn with<R>(f: impl FnOnce(&mut Input, &mut JoypadManager) -> R) -> R {
    ST.with(|s| {
        let s = &mut *s.borrow_mut();
        f(&mut s.input, &mut s.jm)
    })
}

/// A joypad by controller index, created on first use (`checkJoypad`).
fn joypad(input: &mut Input, jm: &mut JoypadManager, index: i32) -> Option<Joypad> {
    jm.check(input, index).and_then(|s| jm.by_controller[s].clone())
}

/// `checkOrAnyJoypad`: index < 0 means any active joypad.
fn check_or_any(index: i32, any: bool, f: impl Fn(&Input, &Joypad) -> bool) -> bool {
    with(|input, jm| {
        if index < 0 && any {
            return jm.list.iter().filter_map(|&s| jm.by_controller[s].as_ref()).any(|j| f(input, j));
        }
        joypad(input, jm, index).is_some_and(|j| f(input, &j))
    })
}

#[derive(Clone, Copy)]
enum Btn {
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

/// `Joypad.is<Button>Pressed()`.
fn pressed(input: &Input, j: &Joypad, b: Btn) -> bool {
    let id = j.id as i32;
    let dpad = |n: i32, d: &dyn Fn(i32) -> bool| if n != -1 { input.is_button_pressed_d(n, id) } else { d(id) };
    match b {
        Btn::A => input.is_button_pressed_d(j.a_button, id),
        Btn::B => input.is_button_pressed_d(j.b_button, id),
        Btn::X => input.is_button_pressed_d(j.x_button, id),
        Btn::Y => input.is_button_pressed_d(j.y_button, id),
        Btn::LB => input.is_button_pressed_d(j.bumper_left, id),
        Btn::RB => input.is_button_pressed_d(j.bumper_right, id),
        Btn::L3 => input.is_button_pressed_d(j.left_stick_button, id),
        Btn::R3 => input.is_button_pressed_d(j.right_stick_button, id),
        Btn::Back => input.is_button_pressed_d(j.back, id),
        Btn::Start => input.is_button_pressed_d(j.start, id),
        Btn::Guide => input.is_button_pressed_d(j.guide, id),
        Btn::Up => dpad(j.dpad_up, &|i| input.is_controller_up_d(i)),
        Btn::Down => dpad(j.dpad_down, &|i| input.is_controller_down_d(i)),
        Btn::Left => dpad(j.dpad_left, &|i| input.is_controller_left_d(i)),
        Btn::Right => dpad(j.dpad_right, &|i| input.is_controller_right_d(i)),
    }
}

/// `JoypadManager.is<Button>Pressed(c)`: the d-pad and bumpers check one
/// joypad; the face, stick, trigger and menu buttons accept -1 for any.
fn jm_pressed(c: i32, b: Btn) -> bool {
    let any = !matches!(b, Btn::Up | Btn::Down | Btn::Left | Btn::Right | Btn::LB | Btn::RB);
    check_or_any(c, any, |input, j| pressed(input, j, b))
}

fn jm_trigger_pressed(c: i32, left: bool) -> bool {
    check_or_any(c, true, |input, j| trigger_value(input, j, left) > 0.7)
}

fn jm_trigger_value(c: i32, left: bool) -> f32 {
    with(|input, jm| joypad(input, jm, c).map_or(0.0, |j| trigger_value(input, &j, left)))
}

/// `Joypad.isMovementInsideDeadZone` / `isAimingInsideDeadZone`.
fn is_applied(input: &Input, jm: &JoypadManager, j: &Joypad, movement: bool) -> bool {
    let stick = if movement { j.movement() } else { j.aiming() };
    let (x, y) = (stick.value(input, j.id as i32, true), stick.value(input, j.id as i32, false));
    let (ax, ay) = if movement { (j.movement_axis_x, j.movement_axis_y) } else { (j.aiming_axis_x, j.aiming_axis_y) };
    let (dx, dy) = (jm.joypad_dead_zone(input, j, ax), jm.joypad_dead_zone(input, j, ay));
    // The movement check multiplies the two dead zones; the aiming check
    // sums their squares, as in the original.
    let dz = if movement { dx * dy } else { dx * dx + dy * dy };
    x * x + y * y >= dz
}

/// `JoypadManager.getMovementAxisX/Y` and `getAimingAxisX/Y`: zero unless
/// the stick is outside its dead zone.
fn jm_stick(c: i32, movement: bool, want_x: bool) -> f32 {
    with(|input, jm| {
        let Some(j) = joypad(input, jm, c) else { return 0.0 };
        if !is_applied(input, jm, &j, movement) {
            return 0.0;
        }
        let stick = if movement { j.movement() } else { j.aiming() };
        stick.value(input, j.id as i32, want_x)
    })
}

fn jm_applied(c: i32, movement: bool) -> bool {
    with(|input, jm| joypad(input, jm, c).is_some_and(|j| is_applied(input, jm, &j, movement)))
}

/// The axis behind a `JoypadAxis1d` dead zone.
fn dead_zone_axis(j: &Joypad, which: &str) -> i32 {
    match which {
        "LeftTrigger" => j.trigger_left,
        "RightTrigger" => j.trigger_right,
        "LeftStickX" => j.movement_axis_x,
        "LeftStickY" => j.movement_axis_y,
        "RightStickX" => j.aiming_axis_x,
        _ => j.aiming_axis_y,
    }
}

fn axis1d_value(name: &str, c: i32) -> f32 {
    match name {
        "LeftTrigger" => jm_trigger_value(c, true),
        "RightTrigger" => jm_trigger_value(c, false),
        "LeftStickX" => jm_stick(c, true, true),
        "LeftStickY" => jm_stick(c, true, false),
        "RightStickX" => jm_stick(c, false, true),
        _ => jm_stick(c, false, false),
    }
}

fn button_down(name: &str, c: i32) -> bool {
    match name {
        "A" => jm_pressed(c, Btn::A),
        "B" => jm_pressed(c, Btn::B),
        "X" => jm_pressed(c, Btn::X),
        "Y" => jm_pressed(c, Btn::Y),
        "LeftStick" => jm_pressed(c, Btn::L3),
        "RightStick" => jm_pressed(c, Btn::R3),
        "LeftBump" => jm_pressed(c, Btn::LB),
        "RightBump" => jm_pressed(c, Btn::RB),
        "Back" => jm_pressed(c, Btn::Back),
        "Start" => jm_pressed(c, Btn::Start),
        "Guide" => jm_pressed(c, Btn::Guide),
        "DPadLeft" => jm_pressed(c, Btn::Left),
        "DPadRight" => jm_pressed(c, Btn::Right),
        "DPadUp" => jm_pressed(c, Btn::Up),
        _ => jm_pressed(c, Btn::Down),
    }
}

// ---- per tick ---------------------------------------------------------------------

/// `GameInput.updateGameThread()` and the Lua events it causes.
pub fn update(mgr: &mut LuaManager) {
    let events = with_controllers(|c| ST.with(|s| s.borrow_mut().input.update_game_thread(c)));
    for e in events {
        match e {
            ControllerEvent::Connected(i) | ControllerEvent::Disconnected(i) => {
                let connected = matches!(e, ControllerEvent::Connected(_));
                let ev = with(|_, jm| jm.on_controller(i, connected));
                let id = Value::Num(i as f64);
                match ev {
                    Some(JoypadEvent::Reactivated(_)) => {
                        mgr.trigger_event("OnJoypadBeforeReactivate", std::slice::from_ref(&id));
                        mgr.trigger_event("OnJoypadReactivate", std::slice::from_ref(&id));
                    }
                    Some(JoypadEvent::Deactivated(_)) => {
                        mgr.trigger_event("OnJoypadBeforeDeactivate", std::slice::from_ref(&id));
                        mgr.trigger_event("OnJoypadDeactivate", std::slice::from_ref(&id));
                    }
                    None => {}
                }
                let name = if connected { "OnGamepadConnect" } else { "OnGamepadDisconnect" };
                mgr.trigger_event(name, std::slice::from_ref(&id));
            }
            other => with(|input, jm| jm.on_input(input, &other)),
        }
    }
}

/// `GameWindow.logic`'s activation after `UIManager.update`: the first
/// active joypad pressing A becomes `activatedJoyPad`.
pub fn after_ui_update(mgr: &mut LuaManager) {
    if !jm_pressed(-1, Btn::A) {
        return;
    }
    let hit = ST.with(|s| {
        let s = &mut *s.borrow_mut();
        let list = s.jm.list.clone();
        for slot in list {
            let Some(j) = s.jm.by_controller[slot].as_ref() else { continue };
            if !pressed(&s.input, j, Btn::A) {
                continue;
            }
            if s.activated.is_none() {
                s.activated = Some(slot);
            }
            return Some(slot);
        }
        None
    });
    // No player exists before a game is loaded: the UI variant.
    if let Some(slot) = hit {
        mgr.trigger_event("OnJoypadActivateUI", &[Value::Num(slot as f64)]);
    }
}

/// Points the joypad config files at the user folder.
pub fn set_cache_dir(dir: &Path) {
    ST.with(|s| s.borrow_mut().jm.cache_dir = dir.to_path_buf());
}

// ---- Lua ------------------------------------------------------------------------------

fn enum_names(base: &Path, class: &str) -> Vec<&'static str> {
    let names = pz_classfile::Jar::open(base.join("projectzomboid.jar")).and_then(|j| pz_classfile::enum_constants(&j, class));
    match names {
        Ok(n) => n.into_iter().map(|s| &*Box::leak(s.into_boxed_str())).collect(),
        Err(e) => {
            eprintln!("{class}: {e}");
            Vec::new()
        }
    }
}

fn enum_name(c: &NativeCall<'_>) -> Result<&'static str, LuaError> {
    with_this::<JavaEnum, _>(c, |e| e.name)
}

pub fn register(mgr: &mut LuaManager) {
    let (base, cache) = {
        let fs = mgr.fs.borrow();
        (fs.base.clone(), fs.cache_dir().to_path_buf())
    };
    set_cache_dir(&cache);
    let t = &mut mgr.thread;
    register_enums(t, &base);
    register_globals(t);
}

fn register_enums(t: &mut Thread, base: &Path) {
    let names = enum_names(base, "zombie/input/JoypadButton");
    let count = names.len();
    let b = expose_enum(t, JOYPAD_BUTTON, &names);
    b.method("isDown", |c| {
        let n = enum_name(c)?;
        Ok(c.push(button_down(n, int_arg(c, 1))))
    });
    b.method("getNameTranslationKey", |c| {
        let n = enum_name(c)?;
        Ok(c.push(Value::str(&format!("UI_optionscreen_gamepad_JoypadButton_name_{n}"))))
    });
    b.static_fn("getButtonCount", move |c| Ok(c.push(count as f64)));
    let n2 = names.clone();
    b.static_fn("isButtonDown", move |c| {
        let (bind, idx) = (int_arg(c, 0), int_arg(c, 1));
        let r = usize::try_from(idx).ok().and_then(|i| n2.get(i)).is_some_and(|n| button_down(n, bind));
        Ok(c.push(r))
    });
    let n3 = names.clone();
    b.static_fn("getButtons", move |c| {
        let tbl = pz_kahlua::TableRef::new();
        for (i, n) in n3.iter().enumerate() {
            tbl.set_int(i as i64 + 1, crate::expose::enum_value(JOYPAD_BUTTON, n));
        }
        Ok(c.push(tbl))
    });
    let n4 = names;
    b.static_fn("fromIndex", move |c| {
        let i = int_arg(c, 0);
        let n = usize::try_from(i).ok().and_then(|k| n4.get(k)).ok_or_else(|| LuaError::Runtime {
            message: format!("Index out of range: {i}. Expected 0 - {}", n4.len()),
            class: "java.lang.ArrayIndexOutOfBoundsException",
        })?;
        Ok(c.push(crate::expose::enum_value(JOYPAD_BUTTON, n)))
    });

    let names = enum_names(base, "zombie/input/JoypadAxis1d");
    let count = names.len();
    let b = expose_enum(t, JOYPAD_AXIS_1D, &names);
    b.method("getValue", |c| {
        let n = enum_name(c)?;
        Ok(c.push(f64::from(axis1d_value(n, int_arg(c, 1)))))
    });
    b.method("getDeadZone", |c| {
        let n = enum_name(c)?;
        let bind = int_arg(c, 1);
        let v = with(|input, jm| joypad(input, jm, bind).map_or(0.0, |j| jm.joypad_dead_zone(input, &j, dead_zone_axis(&j, n))));
        Ok(c.push(f64::from(v)))
    });
    b.method("setDeadZone", |c| {
        let n = enum_name(c)?;
        let (bind, v) = (int_arg(c, 1), num_arg(c, 2) as f32);
        with(|input, jm| {
            if let Some(j) = joypad(input, jm, bind) {
                set_dead_zone(input, j.id, dead_zone_axis(&j, n), v);
            }
        });
        Ok(0)
    });
    b.method("getNameTranslationKey", |c| {
        let n = enum_name(c)?;
        Ok(c.push(Value::str(&format!("UI_optionscreen_gamepad_JoypadAxis1d_name_{n}"))))
    });
    b.static_fn("getAxisCount", move |c| Ok(c.push(count as f64)));
    let n2 = names.clone();
    b.static_fn("getAxes", move |c| {
        let tbl = pz_kahlua::TableRef::new();
        for (i, n) in n2.iter().enumerate() {
            tbl.set_int(i as i64 + 1, crate::expose::enum_value(JOYPAD_AXIS_1D, n));
        }
        Ok(c.push(tbl))
    });
    let n3 = names;
    b.static_fn("fromIndex", move |c| {
        let i = int_arg(c, 0);
        let n = usize::try_from(i).ok().and_then(|k| n3.get(k)).ok_or_else(|| LuaError::Runtime {
            message: format!("Index out of range: {i}. Expected 0 - {}", n3.len()),
            class: "java.lang.ArrayIndexOutOfBoundsException",
        })?;
        Ok(c.push(crate::expose::enum_value(JOYPAD_AXIS_1D, n)))
    });

    let names = enum_names(base, "zombie/input/JoypadAxis2d");
    let b = expose_enum(t, JOYPAD_AXIS_2D, &names);
    b.method("getValueX", |c| {
        let n = enum_name(c)?;
        Ok(c.push(f64::from(jm_stick(int_arg(c, 1), n == "LeftStick", true))))
    });
    b.method("getValueY", |c| {
        let n = enum_name(c)?;
        Ok(c.push(f64::from(jm_stick(int_arg(c, 1), n == "LeftStick", false))))
    });
    b.method("getLength", |c| {
        let n = enum_name(c)?;
        let bind = int_arg(c, 1);
        let (x, y) = (jm_stick(bind, n == "LeftStick", true), jm_stick(bind, n == "LeftStick", false));
        Ok(c.push(f64::from((x * x + y * y).sqrt())))
    });
    b.method("isApplied", |c| {
        let n = enum_name(c)?;
        Ok(c.push(jm_applied(int_arg(c, 1), n == "LeftStick")))
    });
    b.method("getNameTranslationKey", |c| {
        let n = enum_name(c)?;
        Ok(c.push(Value::str(&format!("UI_optionscreen_gamepad_JoypadAxis2d_name_{n}"))))
    });
    let n2 = names;
    b.static_fn("getAxes", move |c| {
        let tbl = pz_kahlua::TableRef::new();
        for (i, n) in n2.iter().enumerate() {
            tbl.set_int(i as i64 + 1, crate::expose::enum_value(JOYPAD_AXIS_2D, n));
        }
        Ok(c.push(tbl))
    });
}

fn controller_str(c: &NativeCall<'_>, f: impl Fn(&pz_input::controller::Controller) -> String) -> Value {
    let i = int_arg(c, 0);
    let s = ST.with(|s| s.borrow().input.controller(i).map(&f));
    Value::str(&s.unwrap_or_else(|| "???".into()))
}

fn register_globals(t: &mut Thread) {
    global(t, "getControllerCount", |c| Ok(c.push(pz_input::controller::MAX_CONTROLLERS as f64)));
    global(t, "isControllerConnected", |c| {
        let i = int_arg(c, 0);
        Ok(c.push(ST.with(|s| s.borrow().input.controller(i).is_some())))
    });
    global(t, "isJoypadConnected", |c| {
        let i = int_arg(c, 0);
        Ok(c.push(ST.with(|s| s.borrow().input.controller(i).is_some())))
    });
    global(t, "getControllerGUID", |c| Ok(c.push(controller_str(c, |ct| ct.guid.clone()))));
    global(t, "getControllerName", |c| Ok(c.push(controller_str(c, |ct| ct.gamepad_name.clone()))));
    global(t, "getControllerAxisValue", |c| {
        let (i, a) = (int_arg(c, 0), int_arg(c, 1));
        let v = ST.with(|s| s.borrow().input.controller(i).filter(|ct| a >= 0 && (a as usize) < ct.axis_count()).map_or(0.0, |ct| ct.axis_value(a)));
        Ok(c.push(f64::from(v)))
    });
    global(t, "getControllerPovX", |c| {
        let i = int_arg(c, 0);
        Ok(c.push(f64::from(ST.with(|s| s.borrow().input.controller(i).map_or(0.0, |ct| ct.pov_x())))))
    });
    global(t, "getControllerPovY", |c| {
        let i = int_arg(c, 0);
        Ok(c.push(f64::from(ST.with(|s| s.borrow().input.controller(i).map_or(0.0, |ct| ct.pov_y())))))
    });
    global(t, "getControllerDeadZone", |c| {
        let (i, a) = (int_arg(c, 0), int_arg(c, 1));
        let v = with(|input, jm| {
            if a < 0 || a as usize >= input.axis_count(i) {
                return 0.0;
            }
            joypad(input, jm, i).map_or(0.0, |j| jm.joypad_dead_zone(input, &j, a))
        });
        Ok(c.push(f64::from(v)))
    });
    global(t, "setControllerDeadZone", |c| {
        let (i, a, v) = (int_arg(c, 0), int_arg(c, 1), num_arg(c, 2) as f32);
        with(|input, jm| {
            if a >= 0 && (a as usize) < input.axis_count(i) && jm.check(input, i).is_some() {
                set_dead_zone(input, i as usize, a, v);
            }
        });
        Ok(0)
    });
    global(t, "saveControllerSettings", |c| {
        let i = int_arg(c, 0);
        if (0..pz_input::controller::MAX_CONTROLLERS as i32).contains(&i) {
            with(|input, jm| jm.save_controller_settings(input, i));
        }
        Ok(0)
    });
    global(t, "reloadControllerConfigFiles", |_c| {
        with(|input, jm| jm.reload_controller_files(input));
        Ok(0)
    });
    for (name, b) in [
        ("isJoypadUp", Btn::Up),
        ("isJoypadDown", Btn::Down),
        ("isJoypadLeft", Btn::Left),
        ("isJoypadRight", Btn::Right),
        ("isJoypadLBPressed", Btn::LB),
        ("isJoypadRBPressed", Btn::RB),
        ("isJoypadLeftStickButtonPressed", Btn::L3),
        ("isJoypadRightStickButtonPressed", Btn::R3),
    ] {
        global(t, name, move |c| Ok(c.push(jm_pressed(int_arg(c, 0), b))));
    }
    global(t, "isJoypadLTPressed", |c| Ok(c.push(jm_trigger_pressed(int_arg(c, 0), true))));
    global(t, "isJoypadRTPressed", |c| Ok(c.push(jm_trigger_pressed(int_arg(c, 0), false))));
    global(t, "getJoypadMovementAxisX", |c| Ok(c.push(f64::from(jm_stick(int_arg(c, 0), true, true)))));
    global(t, "getJoypadMovementAxisY", |c| Ok(c.push(f64::from(jm_stick(int_arg(c, 0), true, false)))));
    global(t, "getJoypadAimingAxisX", |c| Ok(c.push(f64::from(jm_stick(int_arg(c, 0), false, true)))));
    global(t, "getJoypadAimingAxisY", |c| Ok(c.push(f64::from(jm_stick(int_arg(c, 0), false, false)))));
    global(t, "isXBOXController", |c| {
        Ok(c.push(ST.with(|s| s.borrow().input.controllers.iter().flatten().any(|ct| ct.gamepad_name.contains("XBOX 360")))))
    });
    global(t, "isPlaystationController", |c| {
        let i = int_arg(c, 0);
        let r = ST.with(|s| s.borrow().input.controller(i).is_some_and(|ct| ct.joystick_name.contains("Playstation") || ct.joystick_name.contains("Dualshock")));
        Ok(c.push(r))
    });
    global(t, "setDebugToggleControllerPluggedIn", |_c| Ok(0));
    global(t, "reactivateJoypadAfterResetLua", |c| {
        let a = ST.with(|s| s.borrow().activated);
        Ok(c.push(a.is_some()))
    });
    global(t, "wasMouseActiveMoreRecentlyThanJoypad", |c| {
        // No player before a game loads: compare with the activated joypad.
        let r = ST.with(|s| {
            let s = s.borrow();
            match s.activated.and_then(|i| s.jm.by_controller[i].as_ref()) {
                Some(j) if !j.disabled => s.jm.last_activity(j.id) < crate::bind_input::mouse_last_activity(),
                _ => true,
            }
        });
        Ok(c.push(r))
    });
}

/// The activated joypad (`GameWindow.activatedJoyPad`), as a controller slot.
pub fn activated_joypad() -> Option<usize> {
    ST.with(|s| s.borrow().activated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_input::controller::Controller;
    use std::rc::Rc;

    #[test]
    fn controller_globals_and_activation() {
        let dir = std::env::temp_dir().join(format!("pz-bind-joypad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("game/media")).unwrap();
        let mut gfs = pz_fs::GameFs::new(dir.join("game"));
        gfs.set_cache_dir(dir.join("user"));
        let mut mgr = LuaManager::init(Rc::new(RefCell::new(gfs))).unwrap();
        let guid = "030000005e0400008e02000014010000";
        let slot = with_controllers(|c| {
            let mut ct = Controller::new(0, guid, Some("Xbox 360 Controller"), Some("XBOX 360 Controller"), true);
            ct.state.polled = true;
            ct.state.axes[4] = -1.0;
            ct.state.axes[5] = -1.0;
            c.connect(ct).unwrap()
        });
        update(&mut mgr);
        with(|_, jm| jm.set_controller_active(guid, true));
        let call = |mgr: &mut LuaManager, g: &str, args: &[Value]| {
            let f = mgr.thread.globals().get_str(g);
            mgr.thread.call(&f, args).unwrap().into_iter().next().unwrap_or_default()
        };
        let s = Value::Num(slot as f64);
        assert!(call(&mut mgr, "isControllerConnected", std::slice::from_ref(&s)).truthy());
        assert_eq!(call(&mut mgr, "getControllerGUID", std::slice::from_ref(&s)).as_str(), Some(guid));
        assert!(call(&mut mgr, "isXBOXController", &[]).truthy());
        with_controllers(|c| {
            let st = &mut c.slots[slot].as_mut().unwrap().state;
            st.buttons[0] = true;
            st.hat = 4;
            st.axes[0] = 1.0;
        });
        update(&mut mgr);
        assert!(call(&mut mgr, "isJoypadDown", std::slice::from_ref(&s)).truthy());
        assert!((call(&mut mgr, "getJoypadMovementAxisX", std::slice::from_ref(&s)).as_num().unwrap() - 1.0).abs() < 1e-6);
        assert!(!call(&mut mgr, "isJoypadLTPressed", std::slice::from_ref(&s)).truthy());
        after_ui_update(&mut mgr);
        assert_eq!(activated_joypad(), Some(slot));
        assert!(dir.join("user/joypads").join(format!("{guid}.config")).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Reads the joypad enums from an installed game:
/// `PZ_GAME_DIR=... cargo test -p pz-lua-api -- --ignored`.
#[cfg(test)]
#[test]
#[ignore]
fn installed_game_joypad_enums() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
    let b = enum_names(Path::new(&dir), "zombie/input/JoypadButton");
    let a = enum_names(Path::new(&dir), "zombie/input/JoypadAxis1d");
    let s = enum_names(Path::new(&dir), "zombie/input/JoypadAxis2d");
    eprintln!("JoypadButton {b:?}\nJoypadAxis1d {a:?}\nJoypadAxis2d {s:?}");
    assert_eq!((b.len(), a.len(), s.len()), (15, 6, 2));
}
