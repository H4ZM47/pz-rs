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

use pz_input::controller::{set_dead_zone, ControllerEvent, Input, Joypad, JoypadEvent, JoypadManager};
use pz_input::joypad_eval::{self, pressed, Button as Btn};
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

/// Runs `f` on the live state after `checkJoypad(c)` (which creates the
/// joypad on first use); the evaluation itself is `pz_input::joypad_eval`.
fn eval<R>(c: i32, f: impl FnOnce(&Input, &JoypadManager) -> R) -> R {
    with(|input, jm| {
        jm.check(input, c);
        f(input, jm)
    })
}

fn jm_pressed(c: i32, b: Btn) -> bool {
    eval(c, |input, jm| joypad_eval::button_pressed(input, jm, c, b))
}

fn jm_trigger_pressed(c: i32, left: bool) -> bool {
    eval(c, |input, jm| joypad_eval::trigger_pressed(input, jm, c, left))
}

fn jm_stick(c: i32, movement: bool, want_x: bool) -> f32 {
    eval(c, |input, jm| joypad_eval::stick(input, jm, c, movement, want_x))
}

fn jm_applied(c: i32, movement: bool) -> bool {
    eval(c, |input, jm| joypad_eval::stick_applied(input, jm, c, movement))
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
    eval(c, |input, jm| joypad_eval::axis1d(input, jm, name, c))
}

fn button_down(name: &str, c: i32) -> bool {
    eval(c, |input, jm| joypad_eval::button_down(input, jm, name, c))
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
    init_bindings(&base, &cache);
    let t = &mut mgr.thread;
    register_enums(t, &base);
    register_globals(t);
    register_character_bindings(t);
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

/// A copy of the game-thread controller state (`GameWindow.GameInput`) and
/// `JoypadManager.instance`, for readers outside Lua that evaluate joypad
/// bindings over one frame (`pz-player-input`).
pub fn joypad_snapshot() -> (Input, JoypadManager) {
    ST.with(|s| {
        let s = s.borrow();
        (s.input.clone(), s.jm.clone())
    })
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

// ---- character bindings ------------------------------------------------------------

pub const BUTTON_BINDING: &str = "zombie.characters.CharacterJoypadButtonBinding";
pub const AXIS2D_BINDING: &str = "zombie.characters.CharacterJoypadAxis2dBinding";
pub const BINDING_SET: &str = "zombie.characters.CharacterInputBindingSet";

use pz_input::bindings::{Axis2dBinding, BindingSet, Bindings, ButtonBinding, IsDown};
use std::rc::Rc;

thread_local! {
    static BINDINGS: RefCell<Bindings> = RefCell::new(Bindings::default());
    /// Lua objects for loaded sets, by (reload generation, index).
    static SET_OBJECTS: RefCell<(u64, Vec<Value>)> = const { RefCell::new((0, Vec::new())) };
}

/// A `CharacterInputBindingSet` handed to Lua (index into the loaded sets).
pub struct LuaBindingSet {
    generation: u64,
    index: usize,
}

impl pz_kahlua::value::LuaObject for LuaBindingSet {
    fn class_name(&self) -> &str {
        BINDING_SET
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The enum constants' constructor arguments: `(name, extra args)`.
fn enum_ctor_args(base: &Path, class: &str) -> Vec<(String, Vec<pz_classfile::Value>)> {
    let r = pz_classfile::Jar::open(base.join("projectzomboid.jar")).and_then(|j| pz_classfile::self_constructed_constants(&j, class));
    match r {
        // Enum constructors take the implicit name and ordinal first.
        Ok(v) => v.into_iter().map(|(n, a)| (n, a.into_iter().skip(2).collect())).collect(),
        Err(e) => {
            eprintln!("{class}: {e}");
            Vec::new()
        }
    }
}

fn static_name(v: &pz_classfile::Value, class: &str) -> Option<String> {
    match v {
        pz_classfile::Value::Static { class: c, name } if c == class => Some(name.clone()),
        _ => None,
    }
}

fn float_of(v: Option<&pz_classfile::Value>) -> Option<f32> {
    match v? {
        pz_classfile::Value::Float(f) => Some(*f as f32),
        pz_classfile::Value::Int(i) => Some(*i as f32),
        _ => None,
    }
}

/// A `CharacterJoypadButtonBinding` constructor's default binding.
fn default_is_down(args: &[pz_classfile::Value]) -> Option<IsDown> {
    let first = args.first()?;
    if let Some(b) = static_name(first, "zombie/input/JoypadButton") {
        return Some(IsDown::Button(b));
    }
    let (min, max) = (float_of(args.get(1)).unwrap_or(0.0), float_of(args.get(2)).unwrap_or(f32::MAX));
    if let Some(a) = static_name(first, "zombie/input/JoypadAxis1d") {
        return Some(IsDown::Axis1d(a, min, max));
    }
    static_name(first, "zombie/input/JoypadAxis2d").map(|a| IsDown::Axis2d(a, min, max))
}

fn init_bindings(base: &Path, cache: &Path) {
    let buttons: Vec<ButtonBinding> = enum_ctor_args(base, "zombie/characters/CharacterJoypadButtonBinding")
        .into_iter()
        .map(|(name, args)| {
            let d = default_is_down(&args);
            ButtonBinding { name, default: d.clone(), binding: d }
        })
        .collect();
    let axes: Vec<Axis2dBinding> = enum_ctor_args(base, "zombie/characters/CharacterJoypadAxis2dBinding")
        .into_iter()
        .map(|(name, args)| {
            let d = args.first().and_then(|a| static_name(a, "zombie/input/JoypadAxis2d"));
            Axis2dBinding { name, default: d.clone(), binding: d }
        })
        .collect();
    let names = |c| enum_names(base, c).into_iter().map(str::to_string).collect();
    BINDINGS.with(|b| {
        *b.borrow_mut() = Bindings {
            buttons,
            axes,
            button_names: names("zombie/input/JoypadButton"),
            axis1d_names: names("zombie/input/JoypadAxis1d"),
            axis2d_names: names("zombie/input/JoypadAxis2d"),
            loaded: None,
            dir: cache.join("InputBindings"),
        }
    });
}

/// A copy of the current `CharacterJoypadButtonBinding` and
/// `CharacterJoypadAxis2dBinding` bindings (`pz-player-input`).
pub fn character_bindings() -> (Vec<ButtonBinding>, Vec<Axis2dBinding>) {
    BINDINGS.with(|b| {
        let b = b.borrow();
        (b.buttons.clone(), b.axes.clone())
    })
}

/// `IsDownBinding.isDown(joypadBind)`.
fn is_down(b: &IsDown, bind: i32) -> bool {
    eval(bind, |input, jm| joypad_eval::is_down(input, jm, b, bind))
}

/// A Lua enum argument of `class`: its constant name.
fn enum_arg(c: &NativeCall<'_>, i: usize, class: &str) -> Option<&'static str> {
    match c.arg(i) {
        Value::Object(o) => o.as_any().downcast_ref::<JavaEnum>().filter(|e| e.class == class).map(|e| e.name),
        _ => None,
    }
}

fn set_value(generation: u64, index: usize) -> Value {
    SET_OBJECTS.with(|s| {
        let mut s = s.borrow_mut();
        if s.0 != generation {
            *s = (generation, Vec::new());
        }
        while s.1.len() <= index {
            let i = s.1.len();
            s.1.push(Value::Object(Rc::new(LuaBindingSet { generation, index: i })));
        }
        s.1[index].clone()
    })
}

thread_local! {
    static GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn sets_table() -> Value {
    let n = BINDINGS.with(|b| b.borrow_mut().loaded_sets().len());
    let g = GENERATION.with(|g| g.get());
    let t = pz_kahlua::TableRef::new();
    for i in 0..n {
        t.set_int(i as i64 + 1, set_value(g, i));
    }
    Value::Table(t)
}

fn with_set<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut Bindings, usize) -> R) -> Result<R, LuaError> {
    let (gen, idx) = with_this::<LuaBindingSet, _>(c, |s| (s.generation, s.index))?;
    if gen != GENERATION.with(|g| g.get()) {
        return Err(LuaError::runtime("binding set was reloaded"));
    }
    Ok(BINDINGS.with(|b| f(&mut b.borrow_mut(), idx)))
}

fn translated(t: &mut Thread, key: &str) -> String {
    let g = t.globals().get_str("getText");
    if g.is_function() {
        if let Ok(r) = t.call(&g, &[Value::str(key)]) {
            if let Some(s) = r.first().and_then(|v| v.as_str()) {
                return s.to_string();
            }
        }
    }
    key.to_string()
}

fn enum_list(class: &'static str, names: Vec<String>) -> Value {
    let t = pz_kahlua::TableRef::new();
    for (i, n) in names.iter().enumerate() {
        t.set_int(i as i64 + 1, crate::expose::enum_value(class, n));
    }
    Value::Table(t)
}

fn register_character_bindings(t: &mut Thread) {
    // CharacterInputBindingSet
    let b = crate::expose::expose_class(t, BINDING_SET, None);
    b.static_fn("getLoadedBindingSets", |c| Ok(c.push(sets_table())));
    b.static_fn("reloadAll", |c| {
        GENERATION.with(|g| g.set(g.get() + 1));
        BINDINGS.with(|b| {
            b.borrow_mut().reload_all();
        });
        Ok(c.push(sets_table()))
    });
    b.static_fn("saveAll", |c| Ok(c.push(BINDINGS.with(|b| b.borrow_mut().save_all()))));
    b.static_fn("resetAllToDefault", |_c| {
        BINDINGS.with(|b| b.borrow_mut().reset_all_to_default());
        Ok(0)
    });
    // Lua calls these with `:` as well as `.`: skip a leading class table.
    fn string_param(c: &NativeCall<'_>) -> Option<String> {
        let i = usize::from(matches!(c.arg(0), Value::Table(_)));
        crate::expose::str_arg(c, i).map(|s| s.to_string())
    }
    b.static_fn("containsSetName", |c| {
        let n = string_param(c).unwrap_or_default();
        Ok(c.push(BINDINGS.with(|b| b.borrow_mut().contains_set_name(&n))))
    });
    b.static_fn("getUniqueSetName", |c| {
        let n = string_param(c);
        let custom = translated(c.thread, "IGUI_Custom");
        let r = BINDINGS.with(|b| b.borrow_mut().unique_set_name(n.as_deref(), &custom));
        Ok(c.push(Value::str(&r)))
    });
    b.static_fn("createNewFromCurrent", |c| {
        let n = string_param(c);
        let custom = translated(c.thread, "IGUI_Custom");
        let i = BINDINGS.with(|b| b.borrow_mut().create_new_from_current(n.as_deref(), &custom));
        Ok(c.push(set_value(GENERATION.with(|g| g.get()), i)))
    });
    b.method("getName", |c| {
        let n = with_set(c, |b, i| b.loaded_sets()[i].name.clone())?;
        Ok(c.push(Value::str(&n)))
    });
    b.method("getDescription", |c| {
        let d = with_set(c, |b, i| b.loaded_sets()[i].description.clone())?;
        Ok(c.push(d.map_or(Value::Nil, |d| Value::str(&d))))
    });
    b.method("apply", |c| {
        with_set(c, |b, i| {
            let set: BindingSet = b.loaded_sets()[i].clone();
            b.apply(&set);
        })?;
        Ok(0)
    });
    b.method("setBindingsToCurrent", |c| {
        with_set(c, |b, i| {
            let e = b.current_entries();
            b.loaded_sets()[i].entries = e;
        })?;
        Ok(0)
    });
    b.method("save", |c| {
        let ok = with_set(c, |b, i| {
            let set = b.loaded_sets()[i].clone();
            b.save(&set)
        })?;
        Ok(c.push(ok))
    });

    // CharacterJoypadButtonBinding
    let names: Vec<&'static str> = BINDINGS.with(|b| b.borrow().buttons.iter().map(|x| &*Box::leak(x.name.clone().into_boxed_str())).collect());
    let b = expose_enum(t, BUTTON_BINDING, &names);
    fn btn<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut ButtonBinding) -> R) -> Result<R, LuaError> {
        let n = enum_name(c)?;
        Ok(BINDINGS.with(|b| {
            let mut b = b.borrow_mut();
            let i = b.button(n).expect("constant");
            f(&mut b.buttons[i])
        }))
    }
    b.static_fn("allBindings", |c| {
        let n = BINDINGS.with(|b| b.borrow().buttons.iter().map(|x| x.name.clone()).collect());
        Ok(c.push(enum_list(BUTTON_BINDING, n)))
    });
    b.static_fn("setAllToDefault", |_c| {
        BINDINGS.with(|b| b.borrow_mut().buttons.iter_mut().for_each(|x| x.binding = x.default.clone()));
        Ok(0)
    });
    b.static_fn("fromString", |c| {
        let s = crate::expose::str_arg(c, 0).unwrap_or_default();
        let found = BINDINGS.with(|b| b.borrow().button(&s).is_some());
        Ok(c.push(if found { crate::expose::enum_value(BUTTON_BINDING, &s) } else { Value::Nil }))
    });
    let matching = |c: &NativeCall<'_>| -> Vec<String> {
        let (button, a1, a2) = (enum_arg(c, 0, JOYPAD_BUTTON), enum_arg(c, 0, JOYPAD_AXIS_1D), enum_arg(c, 0, JOYPAD_AXIS_2D));
        BINDINGS.with(|b| {
            b.borrow()
                .buttons
                .iter()
                .filter(|x| match (&x.binding, button, a1, a2) {
                    (Some(IsDown::Button(n)), Some(want), _, _) => n == want,
                    (Some(IsDown::Axis1d(n, ..)), _, Some(want), _) => n == want,
                    (Some(IsDown::Axis2d(n, ..)), _, _, Some(want)) => n == want,
                    (None, None, None, None) => false,
                    _ => false,
                })
                .map(|x| x.name.clone())
                .collect()
        })
    };
    b.static_fn("findBindings", move |c| {
        let n = matching(c);
        Ok(c.push(enum_list(BUTTON_BINDING, n)))
    });
    b.static_fn("findBinding", move |c| {
        let n = matching(c);
        Ok(c.push(n.first().map_or(Value::Nil, |n| crate::expose::enum_value(BUTTON_BINDING, n))))
    });
    b.method("isDown", |c| {
        let bind = int_arg(c, 1);
        let d = btn(c, |x| x.binding.clone())?;
        Ok(c.push(d.is_some_and(|d| is_down(&d, bind))))
    });
    b.method("getNameTranslationKey", |c| {
        let n = enum_name(c)?;
        Ok(c.push(Value::str(&format!("UI_optionscreen_gamepad_CharacterJoypadButtonBinding_name_{n}"))))
    });
    b.method("getJoypadButton", |c| {
        let d = btn(c, |x| x.binding.clone())?;
        Ok(c.push(match d {
            Some(IsDown::Button(n)) => crate::expose::enum_value(JOYPAD_BUTTON, &n),
            _ => Value::Nil,
        }))
    });
    b.method("getJoypadAxis1d", |c| {
        let d = btn(c, |x| x.binding.clone())?;
        Ok(c.push(match d {
            Some(IsDown::Axis1d(n, ..)) => crate::expose::enum_value(JOYPAD_AXIS_1D, &n),
            _ => Value::Nil,
        }))
    });
    b.method("getJoypadAxis2d", |c| {
        let d = btn(c, |x| x.binding.clone())?;
        Ok(c.push(match d {
            Some(IsDown::Axis2d(n, ..)) => crate::expose::enum_value(JOYPAD_AXIS_2D, &n),
            _ => Value::Nil,
        }))
    });
    b.method("getAxisMinThreshold", |c| Ok(c.push(f64::from(btn(c, |x| x.binding.as_ref().map_or(0.0, IsDown::min))?))));
    b.method("getAxisMaxThreshold", |c| Ok(c.push(f64::from(btn(c, |x| x.binding.as_ref().map_or(0.0, IsDown::max))?))));
    b.method("isAxisMaxThresholdInfinity", |c| Ok(c.push(btn(c, |x| x.binding.as_ref().map_or(0.0, IsDown::max))? > 1.0)));
    // The binding a Lua argument names, as `setBinding` would store it.
    fn arg_binding(c: &NativeCall<'_>, default_min: Option<f32>) -> Option<Option<IsDown>> {
        if c.arg(1).is_nil() {
            return Some(None);
        }
        let min = if c.nargs() > 2 { Some(num_arg(c, 2) as f32) } else { default_min };
        let max = if c.nargs() > 3 { num_arg(c, 3) as f32 } else { f32::MAX };
        if let Some(n) = enum_arg(c, 1, JOYPAD_BUTTON) {
            return Some(Some(IsDown::Button(n.to_string())));
        }
        if let Some(n) = enum_arg(c, 1, JOYPAD_AXIS_1D) {
            return Some(Some(IsDown::Axis1d(n.to_string(), min?, max)));
        }
        enum_arg(c, 1, JOYPAD_AXIS_2D).and_then(|n| Some(Some(IsDown::Axis2d(n.to_string(), min?, max))))
    }
    fn same_target(a: &Option<IsDown>, b: &Option<IsDown>) -> bool {
        match (a, b) {
            (Some(IsDown::Button(x)), Some(IsDown::Button(y))) => x == y,
            (Some(IsDown::Axis1d(x, ..)), Some(IsDown::Axis1d(y, ..))) => x == y,
            (Some(IsDown::Axis2d(x, ..)), Some(IsDown::Axis2d(y, ..))) => x == y,
            _ => false,
        }
    }
    b.method("setBinding", |c| {
        // `setBinding(axis, min)` stores max = Float.MAX_VALUE; a lone axis
        // (Lua passing no threshold) is a null binding in Java.
        if let Some(nb) = arg_binding(c, None) {
            btn(c, |x| x.binding = nb)?;
        }
        Ok(0)
    });
    b.method("addBinding", |c| {
        if let Some(nb) = arg_binding(c, Some(0.5)) {
            btn(c, |x| {
                if !same_target(&x.binding, &nb) {
                    x.binding = nb;
                }
            })?;
        }
        Ok(0)
    });
    b.method("removeBinding", |c| {
        if let Some(nb) = arg_binding(c, Some(0.0)) {
            btn(c, |x| {
                if same_target(&x.binding, &nb) {
                    x.binding = None;
                }
            })?;
        }
        Ok(0)
    });
    b.method("containsBinding", |c| {
        let nb = arg_binding(c, Some(0.0)).flatten();
        let r = btn(c, |x| same_target(&x.binding, &nb))?;
        Ok(c.push(r))
    });
    b.method("moveBindingFrom", |c| {
        let to = enum_name(c)?;
        let Some(from) = enum_arg(c, 1, BUTTON_BINDING) else { return Ok(0) };
        if to != from {
            BINDINGS.with(|b| {
                let mut b = b.borrow_mut();
                let (ti, fi) = (b.button(to).unwrap(), b.button(from).unwrap());
                let v = b.buttons[fi].binding.take();
                b.buttons[ti].binding = v;
            });
        }
        Ok(0)
    });
    b.method("setDefault", |c| {
        btn(c, |x| x.binding = x.default.clone())?;
        Ok(0)
    });

    // CharacterJoypadAxis2dBinding
    let names: Vec<&'static str> = BINDINGS.with(|b| b.borrow().axes.iter().map(|x| &*Box::leak(x.name.clone().into_boxed_str())).collect());
    let b = expose_enum(t, AXIS2D_BINDING, &names);
    fn axis<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut Axis2dBinding) -> R) -> Result<R, LuaError> {
        let n = enum_name(c)?;
        Ok(BINDINGS.with(|b| {
            let mut b = b.borrow_mut();
            let i = b.axis(n).expect("constant");
            f(&mut b.axes[i])
        }))
    }
    b.static_fn("allBindings", |c| {
        let n = BINDINGS.with(|b| b.borrow().axes.iter().map(|x| x.name.clone()).collect());
        Ok(c.push(enum_list(AXIS2D_BINDING, n)))
    });
    b.static_fn("setAllToDefault", |_c| {
        BINDINGS.with(|b| b.borrow_mut().axes.iter_mut().for_each(|x| x.binding = x.default.clone()));
        Ok(0)
    });
    b.static_fn("fromString", |c| {
        let s = crate::expose::str_arg(c, 0).unwrap_or_default();
        let found = BINDINGS.with(|b| b.borrow().axis(&s).is_some());
        Ok(c.push(if found { crate::expose::enum_value(AXIS2D_BINDING, &s) } else { Value::Nil }))
    });
    b.static_fn("findBindings", |c| {
        let want = enum_arg(c, 0, JOYPAD_AXIS_2D);
        let n = BINDINGS.with(|b| b.borrow().axes.iter().filter(|x| x.binding.as_deref() == want).map(|x| x.name.clone()).collect());
        Ok(c.push(enum_list(AXIS2D_BINDING, n)))
    });
    b.method("getNameTranslationKey", |c| {
        let n = enum_name(c)?;
        Ok(c.push(Value::str(&format!("UI_optionscreen_gamepad_CharacterJoypadAxis2dBinding_name_{n}"))))
    });
    b.method("getJoypadAxis", |c| {
        let a = axis(c, |x| x.binding.clone())?;
        Ok(c.push(a.map_or(Value::Nil, |n| crate::expose::enum_value(JOYPAD_AXIS_2D, &n))))
    });
    b.method("setBinding", |c| {
        let n = enum_arg(c, 1, JOYPAD_AXIS_2D).map(str::to_string);
        axis(c, |x| x.binding = n)?;
        Ok(0)
    });
    b.method("removeBinding", |c| {
        let n = enum_arg(c, 1, JOYPAD_AXIS_2D).map(str::to_string);
        axis(c, |x| {
            if x.binding == n {
                x.binding = None;
            }
        })?;
        Ok(0)
    });
    b.method("moveBindingFrom", |c| {
        let to = enum_name(c)?;
        let Some(from) = enum_arg(c, 1, AXIS2D_BINDING) else { return Ok(0) };
        if to != from {
            BINDINGS.with(|b| {
                let mut b = b.borrow_mut();
                let (ti, fi) = (b.axis(to).unwrap(), b.axis(from).unwrap());
                let v = b.axes[fi].binding.take();
                b.axes[ti].binding = v;
            });
        }
        Ok(0)
    });
    b.method("setDefault", |c| {
        axis(c, |x| x.binding = x.default.clone())?;
        Ok(0)
    });
    let value = |c: &NativeCall<'_>, want_x: Option<bool>| -> Result<f32, LuaError> {
        let a = axis(c, |x| x.binding.clone())?;
        let bind = int_arg(c, 1);
        Ok(a.map_or(0.0, |n| {
            let m = n == "LeftStick";
            match want_x {
                Some(wx) => jm_stick(bind, m, wx),
                None => {
                    let (x, y) = (jm_stick(bind, m, true), jm_stick(bind, m, false));
                    (x * x + y * y).sqrt()
                }
            }
        }))
    };
    b.method("getValueX", move |c| Ok(c.push(f64::from(value(c, Some(true))?))));
    b.method("getValueY", move |c| Ok(c.push(f64::from(value(c, Some(false))?))));
    b.method("getLength", move |c| Ok(c.push(f64::from(value(c, None)?))));
    b.method("isApplied", |c| {
        let a = axis(c, |x| x.binding.clone())?;
        Ok(c.push(a.is_some_and(|n| jm_applied(int_arg(c, 1), n == "LeftStick"))))
    });
}

/// Reads the character binding defaults from an installed game and drives
/// them from Lua: `PZ_GAME_DIR=... cargo test -p pz-lua-api -- --ignored`.
#[cfg(test)]
#[test]
#[ignore]
fn installed_game_character_bindings() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
    let tmp = std::env::temp_dir().join(format!("pz-char-bind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let mut gfs = pz_fs::GameFs::new(Path::new(&dir).to_path_buf());
    gfs.set_cache_dir(tmp.clone());
    let mut mgr = LuaManager::init(Rc::new(RefCell::new(gfs))).unwrap();
    let (nb, na) = BINDINGS.with(|b| (b.borrow().buttons.len(), b.borrow().axes.len()));
    eprintln!("{nb} button bindings, {na} axis bindings");
    assert!(nb > 0 && na > 0);
    let src = r#"
        local B = CharacterJoypadButtonBinding
        local n = 0
        for _, b in ipairs(B.allBindings()) do n = n + 1 end
        assert(n > 0)
        assert(B.fromString("NoSuchBinding") == nil)
        local first = B.allBindings()[1]
        local key = first:getNameTranslationKey()
        assert(key:find("UI_optionscreen_gamepad_CharacterJoypadButtonBinding_name_", 1, true))
        first:setBinding(nil)
        assert(first:getJoypadButton() == nil)
        first:setDefault()
        local sets = CharacterInputBindingSet.getLoadedBindingSets()
        local s = CharacterInputBindingSet.createNewFromCurrent("Mine")
        assert(s:getName() == "Mine")
        assert(s:save())
        CharacterInputBindingSet.reloadAll()
        assert(CharacterInputBindingSet.containsSetName("Mine"))
        for _, a in ipairs(CharacterJoypadAxis2dBinding.allBindings()) do
            assert(a:getValueX(0) == 0)
        end
    "#;
    let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
    let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), mgr.thread.globals().clone()));
    if let Err((e, tr)) = mgr.thread.pcall(&f, &[]) {
        panic!("lua error: {e:?}\n{tr}");
    }
    std::fs::remove_dir_all(&tmp).unwrap();
}
