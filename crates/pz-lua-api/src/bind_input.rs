//! Keyboard and mouse: `org.lwjglx.input.Keyboard`, `zombie.input.GameKeyboard`,
//! `zombie.input.Mouse` and the `GlobalObject` key/mouse predicates.
//!
//! Raw state comes from `pz_input` (fed by the window); this module holds
//! the game-side statics (`GameKeyboard.down/lastDown/eatKey`, `Mouse`'s
//! button arrays) and runs their per-frame updates:
//!
//! 1. the window feeds events into `pz_input::with_input` as they arrive;
//! 2. once per game tick the engine calls [`update`] (`GameKeyboard.update`
//!    then `Mouse.update`), which dispatches key events to `UIManager` and
//!    Lua and fills `UIManager`'s mouse state;
//! 3. after `UIManager.update`, [`after_ui_update`] picks up the buttons the
//!    UI consumed and activates a joypad that pressed A.
//!
//! The `KEY_*` and `BTN_*` constants are read from the installed
//! `projectzomboid.jar`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use pz_input::keys::{self, KeyClass};
use pz_input::{with_input, WindowRequest, KEYBOARD_SIZE, MOUSE_BUTTONS};
use pz_kahlua::{LuaError, NativeCall, Thread, Value};
use pz_ui::{with_ui, UIManager};

use crate::bind_core::{shared_core, KEYBIND_ID_CLASS};
use crate::expose::{bool_arg, expose_class, global, int_arg, str_arg};
use crate::LuaManager;

pub const KEYBOARD: &str = "org.lwjglx.input.Keyboard";
pub const GAME_KEYBOARD: &str = "zombie.input.GameKeyboard";
pub const MOUSE: &str = "zombie.input.Mouse";

/// `Mouse.BTN_OFFSET`: key codes at or above it are mouse buttons.
const BTN_OFFSET: i32 = 10000;

// ---- constants from the jar ---------------------------------------------------

/// `public static final int` constants of `class`, in declaration order.
fn int_constants(base: &Path, class: &str) -> Vec<(String, i32)> {
    let jar = match pz_classfile::Jar::open(base.join("projectzomboid.jar")) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("{class}: cannot open projectzomboid.jar: {e}");
            return Vec::new();
        }
    };
    let cf = match pz_classfile::load_class(&jar, class) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{class}: {e}");
            return Vec::new();
        }
    };
    const PUBLIC_STATIC_FINAL: u16 = 0x0001 | 0x0008 | 0x0010;
    cf.fields
        .iter()
        .filter(|f| f.access & PUBLIC_STATIC_FINAL == PUBLIC_STATIC_FINAL && f.descriptor == "I")
        .filter_map(|f| match cf.constant(f.constant_value?).ok()? {
            pz_classfile::class::Constant::Integer(v) => Some((f.name.clone(), *v)),
            _ => None,
        })
        .collect()
}

/// `Keyboard.keyName` / `keyMap`.
#[derive(Default)]
struct KeyNames {
    constants: Vec<(String, i32)>,
    names: Vec<Option<String>>,
    map: HashMap<String, i32>,
}

impl KeyNames {
    /// `Keyboard.initKeyNames()`: `KEY_*` fields (not `*WIN`), keypad keys
    /// as `KP_*`, printable keys by the layout's character.
    fn init(&mut self) {
        self.names = vec![None; KEYBOARD_SIZE];
        self.map.clear();
        let layout = with_input(|i| i.layout_names.clone());
        for (field, key) in &self.constants {
            if !field.starts_with("KEY_") || field.ends_with("WIN") {
                continue;
            }
            let Some(slot) = usize::try_from(*key).ok().filter(|&k| k < KEYBOARD_SIZE) else { continue };
            let mut name = field["KEY_".len()..].to_string();
            match keys::class(*key) {
                KeyClass::KeypadDigit => name = name.replace("NUMPAD", "KP_"),
                KeyClass::KeypadOther => name = format!("KP_{}", name.replace("NUMPAD", "")),
                KeyClass::Printable(us) => {
                    name = layout.iter().find(|(k, _)| k == key).map_or_else(|| us.to_string(), |(_, n)| n.clone());
                }
                KeyClass::Named | KeyClass::Unmapped => {}
            }
            self.names[slot] = Some(name.clone());
            self.map.insert(name, *key);
        }
    }
}

// ---- game-side state ----------------------------------------------------------

/// `GameKeyboard`'s statics.
struct GameKeyboard {
    down: Option<Vec<bool>>,
    last_down: Vec<bool>,
    eat_key: Vec<bool>,
    no_events_while_loading: bool,
    do_lua_key_pressed: bool,
}

/// `zombie.input.Mouse`'s statics.
struct MouseStatics {
    x: i32,
    y: i32,
    down: Option<Vec<bool>>,
    prev: Vec<bool>,
    wheel_delta: i32,
    time_right_pressed: f32,
    last_activity: i64,
    ui_captured: [bool; 10],
    cursor_visible: bool,
}

thread_local! {
    static NAMES: RefCell<KeyNames> = RefCell::new(KeyNames::default());
    static GK: RefCell<GameKeyboard> = const {
        RefCell::new(GameKeyboard { down: None, last_down: Vec::new(), eat_key: Vec::new(), no_events_while_loading: false, do_lua_key_pressed: true })
    };
    static MS: RefCell<MouseStatics> = const {
        RefCell::new(MouseStatics {
            x: 0,
            y: 0,
            down: None,
            prev: Vec::new(),
            wheel_delta: 0,
            time_right_pressed: 0.0,
            last_activity: 0,
            ui_captured: [false; 10],
            cursor_visible: true,
        })
    };
}

/// `Keyboard.isKeyDown(key)`: the physical key, if GLFW maps it.
pub fn keyboard_is_key_down(key: i32) -> bool {
    if keys::class(key) == KeyClass::Unmapped {
        return false;
    }
    with_input(|i| usize::try_from(key).ok().and_then(|k| i.keys_down.get(k).copied()).unwrap_or(false))
}

fn doing_text_entry() -> bool {
    shared_core().borrow().current_text_entry_box.as_ref().is_some_and(|b| b.doing_text_entry)
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// `GameKeyboard.isKeyDown(int)`.
pub fn is_key_down(key: i32) -> bool {
    if key >= BTN_OFFSET {
        return mouse_is_button_down_ui_check(key - BTN_OFFSET);
    }
    if doing_text_entry() {
        return false;
    }
    GK.with(|g| g.borrow().down.as_ref().and_then(|d| usize::try_from(key).ok().and_then(|k| d.get(k).copied())).unwrap_or(false))
}

/// `GameKeyboard.isKeyDownRaw(int)`: the state read at the last update,
/// ignoring text entry.
pub fn is_key_down_raw(key: i32) -> bool {
    GK.with(|g| g.borrow().down.as_ref().and_then(|d| usize::try_from(key).ok().and_then(|k| d.get(k).copied())).unwrap_or(false))
}

/// `GameKeyboard.eatKeyPress(int)`.
pub fn eat_key_press(key: i32) {
    GK.with(|g| {
        let mut g = g.borrow_mut();
        if let Some(e) = usize::try_from(key).ok().and_then(|i| g.eat_key.get_mut(i)) {
            *e = true;
        }
    });
}

/// `GameKeyboard.wasKeyDown(int)`.
pub fn was_key_down(key: i32) -> bool {
    if key >= BTN_OFFSET {
        return mouse_was_button_down(key - BTN_OFFSET);
    }
    if doing_text_entry() {
        return false;
    }
    GK.with(|g| {
        let g = g.borrow();
        g.down.as_ref().and_then(|_| usize::try_from(key).ok().and_then(|k| g.last_down.get(k).copied())).unwrap_or(false)
    })
}

/// `GameKeyboard.isKeyPressed(int)`.
pub fn is_key_pressed(key: i32) -> bool {
    is_key_down(key) && !was_key_down(key)
}

fn binding(name: &str) -> (i32, i32, bool) {
    let core = shared_core();
    let core = core.borrow();
    let b = core.get_key_binding(name);
    let invalid = core.invalid_binding_shift_ctrl(&b);
    (b.key_value, b.alt_key, invalid)
}

/// `GameKeyboard.isKeyDown(String)`.
pub fn is_key_down_named(name: &str) -> bool {
    let (k, alt, invalid) = binding(name);
    !invalid && (is_key_down(k) || is_key_down(alt))
}

/// `GameKeyboard.wasKeyDown(String)`.
pub fn was_key_down_named(name: &str) -> bool {
    let core = shared_core();
    let (k, alt) = {
        let c = core.borrow();
        (c.get_key(name), c.get_alt_key(name))
    };
    was_key_down(k) || was_key_down(alt)
}

fn mouse_is_button_down(b: i32) -> bool {
    MS.with(|m| m.borrow().down.as_ref().and_then(|d| usize::try_from(b).ok().and_then(|i| d.get(i).copied())).unwrap_or(false))
}

fn mouse_was_button_down(b: i32) -> bool {
    MS.with(|m| {
        let m = m.borrow();
        m.down.as_ref().and_then(|_| usize::try_from(b).ok().and_then(|i| m.prev.get(i).copied())).unwrap_or(false)
    })
}

fn mouse_is_button_pressed(b: i32) -> bool {
    MS.with(|m| m.borrow().down.is_some()) && mouse_is_button_down(b) && !mouse_was_button_down(b)
}

fn mouse_is_button_released(b: i32) -> bool {
    MS.with(|m| m.borrow().down.is_some()) && !mouse_is_button_down(b) && mouse_was_button_down(b)
}

/// `Mouse.isRightDelay()`: the right button held for 0.15 s and not
/// consumed by the UI.
pub fn mouse_is_right_delay() -> bool {
    MS.with(|m| {
        let m = m.borrow();
        !m.ui_captured[1] && m.down.as_ref().is_some_and(|d| d[1]) && m.time_right_pressed >= 0.15
    })
}

/// `Mouse.isButtonDownUICheck(number)`.
pub fn mouse_is_button_down_ui_check(n: i32) -> bool {
    let Ok(i) = usize::try_from(n) else { return false };
    let state = MS.with(|m| {
        let mut m = m.borrow_mut();
        let down = m.down.as_ref().and_then(|d| d.get(i).copied())?;
        if !down {
            if let Some(c) = m.ui_captured.get_mut(i) {
                *c = false;
            }
        } else if m.ui_captured.get(i).copied().unwrap_or(false) {
            return Some(false);
        }
        Some(down)
    });
    match state {
        None => false,
        Some(false) => false,
        Some(true) if n == 1 => mouse_is_right_delay(),
        Some(true) => true,
    }
}

// ---- per-frame updates ------------------------------------------------------------

/// `GameKeyboard.update()` and `Mouse.update()`. `seconds` is
/// `GameTime.getRealworldSecondsSinceLastUpdate()`.
pub fn update(mgr: &mut LuaManager, seconds: f32) {
    // `GameWindow.logic`: Mouse, then GameKeyboard, then GameInput.
    mouse_update(seconds);
    game_keyboard_update(mgr);
    crate::bind_joypad::update(mgr);
}

/// `Mouse.lastActivity`.
pub fn mouse_last_activity() -> i64 {
    MS.with(|m| m.borrow().last_activity)
}

fn game_keyboard_update(mgr: &mut LuaManager) {
    let c = KEYBOARD_SIZE;
    GK.with(|g| {
        let mut g = g.borrow_mut();
        if g.down.is_none() {
            g.down = Some(vec![false; c]);
            g.last_down = vec![false; c];
            g.eat_key = vec![false; c];
        }
    });
    let text_entry = doing_text_entry();
    let no_events = GK.with(|g| g.borrow().no_events_while_loading);
    for n in 1..c {
        let now = keyboard_is_key_down(n as i32);
        let (down, last) = GK.with(|g| {
            let mut g = g.borrow_mut();
            let prev = g.down.as_ref().unwrap()[n];
            g.last_down[n] = prev;
            g.down.as_mut().unwrap()[n] = now;
            (now, prev)
        });
        let do_lua = GK.with(|g| g.borrow().do_lua_key_pressed);
        let key = Value::Num(n as f64);
        if !down && last {
            let eaten = GK.with(|g| std::mem::replace(&mut g.borrow_mut().eat_key[n], false));
            if eaten {
                continue;
            }
            if no_events || text_entry || UIManager::on_key_release(&mut mgr.thread, n as i32) {
                continue;
            }
            if do_lua {
                mgr.trigger_event("OnKeyPressed", std::slice::from_ref(&key));
            }
            mgr.trigger_event("OnCustomUIKey", std::slice::from_ref(&key));
            mgr.trigger_event("OnCustomUIKeyReleased", std::slice::from_ref(&key));
        }
        if down && last {
            if no_events || text_entry || UIManager::on_key_repeat(&mut mgr.thread, n as i32) {
                continue;
            }
            if do_lua {
                mgr.trigger_event("OnKeyKeepPressed", std::slice::from_ref(&key));
            }
        }
        let eaten = GK.with(|g| g.borrow().eat_key[n]);
        if !down || last || no_events || text_entry || eaten || UIManager::on_key_press(&mut mgr.thread, n as i32) || GK.with(|g| g.borrow().eat_key[n]) {
            continue;
        }
        if do_lua {
            mgr.trigger_event("OnKeyStartPressed", std::slice::from_ref(&key));
        }
        mgr.trigger_event("OnCustomUIKeyPressed", std::slice::from_ref(&key));
    }
    // `Core.invalidBindingShiftCtrl` reads `GameKeyboard.isKeyDownRaw`.
    let raw: std::collections::HashSet<i32> =
        GK.with(|g| g.borrow().down.as_ref().map(|d| d.iter().enumerate().filter(|(_, &b)| b).map(|(i, _)| i as i32).collect()).unwrap_or_default());
    shared_core().borrow_mut().keys_down_raw = raw;
}

fn mouse_update(seconds: f32) {
    let (cx, cy, wheel, buttons) = with_input(|i| (i.cursor_x, i.cursor_y, i.take_wheel(), i.buttons_down));
    let core = shared_core();
    let zoom = core.borrow().get_zoom(0);
    MS.with(|m| {
        let mut m = m.borrow_mut();
        let (last_x, last_y) = (m.x, m.y);
        // `x = state.getX(); y = screenHeight - state.getY() - 1` where the
        // LWJGL y is already flipped: one pixel above the cursor.
        m.x = cx as i32;
        m.y = cy as i32 - 1;
        m.wheel_delta = wheel;
        let mut activity = last_x != m.x || last_y != m.y || wheel != 0;
        if m.down.is_none() {
            m.down = Some(vec![false; MOUSE_BUTTONS]);
            m.prev = vec![false; MOUSE_BUTTONS];
        }
        m.prev = m.down.clone().unwrap();
        for (i, &b) in buttons.iter().enumerate() {
            if m.prev[i] != b {
                activity = true;
            }
        }
        m.down = Some(buttons.to_vec());
        m.time_right_pressed = if buttons[1] { m.time_right_pressed + seconds } else { 0.0 };
        if activity {
            m.last_activity = now_ms();
        }
    });
    // `UIManager` reads Mouse through its MouseState.
    let (x, y, down, prev, wd) = MS.with(|m| {
        let m = m.borrow();
        (m.x, m.y, m.down.clone().unwrap_or_default(), m.prev.clone(), m.wheel_delta)
    });
    let right_delay = mouse_is_right_delay();
    with_ui(|u| {
        u.mouse.x = x;
        u.mouse.y = y;
        u.mouse.world_x = (x as f32 * zoom) as i32;
        u.mouse.world_y = (y as f32 * zoom) as i32;
        u.mouse.down = down;
        u.mouse.prev_down = prev;
        u.mouse.wheel = wd;
        u.mouse.right_delay = right_delay;
        u.mouse.ui_blocked.resize(MOUSE_BUTTONS, false);
    });
    core.borrow_mut().mouse_buttons_down =
        MS.with(|m| m.borrow().down.as_ref().map(|d| d.iter().enumerate().filter(|(_, &b)| b).map(|(i, _)| i as i32).collect()).unwrap_or_default());
}

/// After `UIManager.update`: buttons the UI blocked
/// (`Mouse.UIBlockButtonDown`), then joypad activation.
pub fn after_ui_update(mgr: &mut LuaManager) {
    crate::bind_joypad::after_ui_update(mgr);
    let blocked = with_ui(|u| u.mouse.ui_blocked.clone());
    MS.with(|m| {
        let mut m = m.borrow_mut();
        for (i, b) in blocked.iter().enumerate() {
            if *b {
                if let Some(c) = m.ui_captured.get_mut(i) {
                    *c = true;
                }
            }
        }
    });
}

// ---- Lua ------------------------------------------------------------------------

fn key_arg(c: &NativeCall<'_>, i: usize) -> KeyArg {
    match c.arg(i) {
        Value::Str(s) => KeyArg::Name(s.to_string()),
        Value::Object(o) if o.class_name() == KEYBIND_ID_CLASS => KeyArg::Name(o.to_lua_string()),
        _ => KeyArg::Code(int_arg(c, i)),
    }
}

enum KeyArg {
    Code(i32),
    Name(String),
}

fn key_down_any(k: KeyArg) -> bool {
    match k {
        KeyArg::Code(n) => is_key_down(n),
        KeyArg::Name(s) => is_key_down_named(&s),
    }
}

fn was_down_any(k: KeyArg) -> bool {
    match k {
        KeyArg::Code(n) => was_key_down(n),
        KeyArg::Name(s) => was_key_down_named(&s),
    }
}

fn pressed_any(k: KeyArg) -> bool {
    match k {
        KeyArg::Code(n) => is_key_pressed(n),
        KeyArg::Name(s) => is_key_down_named(&s) && !was_key_down_named(&s),
    }
}

fn index_error(i: i32, len: usize) -> LuaError {
    LuaError::Runtime { message: format!("Index {i} out of bounds for length {len}"), class: "java.lang.ArrayIndexOutOfBoundsException" }
}

pub fn register(mgr: &mut LuaManager) {
    let base = mgr.fs.borrow().base.clone();
    NAMES.with(|n| {
        let mut n = n.borrow_mut();
        n.constants = int_constants(&base, "org/lwjglx/input/Keyboard");
        n.init();
    });
    let mouse_consts = int_constants(&base, "zombie/input/Mouse");
    register_keyboard(&mut mgr.thread);
    register_game_keyboard(&mut mgr.thread);
    register_mouse(&mut mgr.thread, &mouse_consts);
    register_globals(&mut mgr.thread);
}

fn register_keyboard(t: &mut Thread) {
    let b = expose_class(t, KEYBOARD, None);
    for (name, v) in NAMES.with(|n| n.borrow().constants.clone()) {
        b.field(&name, Value::Num(f64::from(v)));
    }
    b.static_fn("isCreated", |c| Ok(c.push(true)));
    b.static_fn("create", |_c| {
        NAMES.with(|n| n.borrow_mut().init());
        Ok(0)
    });
    b.static_fn("initKeyNames", |_c| {
        NAMES.with(|n| n.borrow_mut().init());
        Ok(0)
    });
    b.static_fn("poll", |_c| Ok(0));
    b.static_fn("destroy", |_c| Ok(0));
    b.static_fn("isKeyDown", |c| {
        let k = int_arg(c, 0);
        Ok(c.push(keyboard_is_key_down(k)))
    });
    b.static_fn("getKeyName", |c| {
        let k = int_arg(c, 0);
        let name = NAMES.with(|n| {
            let n = n.borrow();
            usize::try_from(k).ok().and_then(|i| n.names.get(i).cloned()).ok_or_else(|| index_error(k, n.names.len()))
        })?;
        Ok(c.push(name.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.static_fn("getKeyIndex", |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let k = NAMES.with(|n| n.borrow().map.get(&*name).copied().unwrap_or(0));
        Ok(c.push(f64::from(k)))
    });
    b.static_fn("enableRepeatEvents", |c| {
        let on = bool_arg(c, 0);
        with_input(|i| i.repeat_events = on);
        Ok(0)
    });
    b.static_fn("areRepeatEventsEnabled", |c| Ok(c.push(with_input(|i| i.repeat_events))));
    b.static_fn("isRepeatEvent", |c| Ok(c.push(with_input(|i| i.repeat_events))));
    b.static_fn("next", |c| Ok(c.push(with_input(|i| i.key_events.next()))));
    b.static_fn("getEventKey", |c| Ok(c.push(f64::from(with_input(|i| i.key_events.event_key())))));
    b.static_fn("getEventCharacter", |c| {
        let ch = with_input(|i| i.key_events.event_character());
        Ok(c.push(Value::str(&ch.to_string())))
    });
    b.static_fn("getEventKeyState", |c| Ok(c.push(with_input(|i| i.key_events.event_key_state()))));
    b.static_fn("getEventNanoseconds", |c| Ok(c.push(with_input(|i| i.key_events.event_nanoseconds()) as f64)));
    // `addKeyEvent(glfwKey, status)` takes a GLFW key in the original; the
    // queue here works in LWJGL codes.
    b.static_fn("addKeyEvent", |c| {
        let (k, s) = (int_arg(c, 0), int_arg(c, 1));
        with_input(|i| i.key(k, s));
        Ok(0)
    });
    b.static_fn("addCharEvent", |c| {
        if let Some(ch) = str_arg(c, 0).and_then(|s| s.chars().next()) {
            with_input(|i| i.char(ch));
        }
        Ok(0)
    });
}

fn register_game_keyboard(t: &mut Thread) {
    let b = expose_class(t, GAME_KEYBOARD, None);
    b.field("noEventsWhileLoading", false);
    b.field("doLuaKeyPressed", true);
    b.static_fn("isKeyDown", |c| {
        let k = key_arg(c, 0);
        Ok(c.push(key_down_any(k)))
    });
    b.static_fn("wasKeyDown", |c| {
        let k = key_arg(c, 0);
        Ok(c.push(was_down_any(k)))
    });
    b.static_fn("isKeyPressed", |c| {
        let k = key_arg(c, 0);
        Ok(c.push(pressed_any(k)))
    });
    b.static_fn("isKeyDownRaw", |c| {
        let k = int_arg(c, 0);
        let r = GK.with(|g| g.borrow().down.as_ref().and_then(|d| usize::try_from(k).ok().and_then(|i| d.get(i).copied())).unwrap_or(false));
        Ok(c.push(r))
    });
    b.static_fn("wasKeyDownRaw", |c| {
        let k = int_arg(c, 0);
        let r = GK.with(|g| usize::try_from(k).ok().and_then(|i| g.borrow().last_down.get(i).copied()).unwrap_or(false));
        Ok(c.push(r))
    });
    fn which(c: &NativeCall<'_>, test: impl Fn(i32) -> bool, gate: impl Fn(&str) -> bool) -> f64 {
        let name = match key_arg(c, 0) {
            KeyArg::Name(s) => s,
            KeyArg::Code(n) => n.to_string(),
        };
        if !gate(&name) {
            return 0.0;
        }
        let core = shared_core();
        let (k, alt) = {
            let c = core.borrow();
            (c.get_key(&name), c.get_alt_key(&name))
        };
        if test(k) {
            f64::from(k)
        } else if test(alt) {
            f64::from(alt)
        } else {
            0.0
        }
    }
    b.static_fn("whichKeyDown", |c| Ok(c.push(which(c, is_key_down, is_key_down_named))));
    b.static_fn("whichKeyPressed", |c| Ok(c.push(which(c, is_key_pressed, |s| is_key_down_named(s) && !was_key_down_named(s)))));
    b.static_fn("whichKeyWasDown", |c| Ok(c.push(which(c, was_key_down, |_| true))));
    b.static_fn("whichKeyDownIgnoreMouse", |c| Ok(c.push(which(c, |k| k < BTN_OFFSET && is_key_down(k), |_| true))));
    b.static_fn("eatKeyPress", |c| {
        let k = int_arg(c, 0);
        GK.with(|g| {
            let mut g = g.borrow_mut();
            if let Some(e) = usize::try_from(k).ok().and_then(|i| g.eat_key.get_mut(i)) {
                *e = true;
            }
        });
        Ok(0)
    });
    b.static_fn("setDoLuaKeyPressed", |c| {
        let on = bool_arg(c, 0);
        GK.with(|g| g.borrow_mut().do_lua_key_pressed = on);
        Ok(0)
    });
    // The engine runs these through `bind_input::update`.
    b.static_fn("update", |_c| Ok(0));
    b.static_fn("poll", |_c| Ok(0));
}

fn register_mouse(t: &mut Thread, consts: &[(String, i32)]) {
    let b = expose_class(t, MOUSE, None);
    for (name, v) in consts {
        b.field(name, Value::Num(f64::from(*v)));
    }
    b.static_fn("getXA", |c| Ok(c.push(f64::from(MS.with(|m| m.borrow().x)))));
    b.static_fn("getYA", |c| Ok(c.push(f64::from(MS.with(|m| m.borrow().y)))));
    b.static_fn("getX", |c| {
        let z = shared_core().borrow().get_zoom(0);
        Ok(c.push(f64::from((MS.with(|m| m.borrow().x) as f32 * z) as i32)))
    });
    b.static_fn("getY", |c| {
        let z = shared_core().borrow().get_zoom(0);
        Ok(c.push(f64::from((MS.with(|m| m.borrow().y) as f32 * z) as i32)))
    });
    b.static_fn("getWheelState", |c| Ok(c.push(f64::from(MS.with(|m| m.borrow().wheel_delta)))));
    b.static_fn("getButtonCount", |c| Ok(c.push(MOUSE_BUTTONS as f64)));
    b.static_fn("isButtonKey", |c| Ok(c.push(int_arg(c, 0) >= BTN_OFFSET)));
    b.static_fn("isButtonDown", |c| Ok(c.push(mouse_is_button_down(int_arg(c, 0)))));
    b.static_fn("wasButtonDown", |c| Ok(c.push(mouse_was_button_down(int_arg(c, 0)))));
    b.static_fn("isButtonPressed", |c| Ok(c.push(mouse_is_button_pressed(int_arg(c, 0)))));
    b.static_fn("isButtonReleased", |c| Ok(c.push(mouse_is_button_released(int_arg(c, 0)))));
    b.static_fn("isButtonDownUICheck", |c| Ok(c.push(mouse_is_button_down_ui_check(int_arg(c, 0)))));
    b.static_fn("UIBlockButtonDown", |c| {
        let n = int_arg(c, 0);
        let ok = MS.with(|m| usize::try_from(n).ok().and_then(|i| m.borrow_mut().ui_captured.get_mut(i).map(|v| *v = true)).is_some());
        if !ok {
            return Err(index_error(n, 10));
        }
        Ok(0)
    });
    b.static_fn("isRightDelay", |c| Ok(c.push(mouse_is_right_delay())));
    for (name, btn, f) in [
        ("isLeftDown", 0, 0),
        ("isLeftPressed", 0, 1),
        ("isLeftReleased", 0, 2),
        ("isLeftUp", 0, 3),
        ("isMiddleDown", 2, 0),
        ("isMiddlePressed", 2, 1),
        ("isMiddleReleased", 2, 2),
        ("isMiddleUp", 2, 3),
        ("isRightDown", 1, 0),
        ("isRightPressed", 1, 1),
        ("isRightReleased", 1, 2),
        ("isRightUp", 1, 3),
    ] {
        b.static_fn(name, move |c| {
            let r = match f {
                0 => mouse_is_button_down(btn),
                1 => mouse_is_button_pressed(btn),
                2 => mouse_is_button_released(btn),
                _ => !mouse_is_button_down(btn),
            };
            Ok(c.push(r))
        });
    }
    b.static_fn("setCursorVisible", |c| {
        let v = bool_arg(c, 0);
        MS.with(|m| m.borrow_mut().cursor_visible = v);
        Ok(0)
    });
    b.static_fn("isCursorVisible", |c| Ok(c.push(MS.with(|m| m.borrow().cursor_visible))));
    b.static_fn("setXY", |c| {
        // `setCursorPosition(x, offscreenHeight - 1 - y)` in GLFW's top-left
        // window coordinates: the same point from the top.
        let (x, y) = (int_arg(c, 0), int_arg(c, 1));
        with_input(|i| i.request(WindowRequest::SetCursorPosition(x, y)));
        Ok(0)
    });
    // Hardware cursors and the cursor texture belong to the window layer.
    b.static_fn("initCustomCursor", |_c| Ok(0));
    b.static_fn("loadCursor", |c| Ok(c.push(Value::Nil)));
    b.static_fn("renderCursorTexture", |_c| Ok(0));
    b.static_fn("update", |_c| Ok(0));
    b.static_fn("poll", |_c| Ok(0));
}

fn register_globals(t: &mut Thread) {
    global(t, "isKeyDown", |c| {
        let k = key_arg(c, 0);
        Ok(c.push(key_down_any(k)))
    });
    global(t, "wasKeyDown", |c| {
        let k = key_arg(c, 0);
        Ok(c.push(was_down_any(k)))
    });
    global(t, "isKeyPressed", |c| {
        let k = key_arg(c, 0);
        Ok(c.push(pressed_any(k)))
    });
    global(t, "isShiftKeyDown", |c| Ok(c.push(is_key_down(42) || is_key_down(54))));
    global(t, "isCtrlKeyDown", |c| Ok(c.push(is_key_down(29) || is_key_down(157))));
    global(t, "isAltKeyDown", |c| Ok(c.push(is_key_down(56) || is_key_down(184))));
    global(t, "isMetaKeyDown", |c| Ok(c.push(is_key_down(219) || is_key_down(220))));
    global(t, "getMouseX", |c| Ok(c.push(f64::from(MS.with(|m| m.borrow().x)))));
    global(t, "getMouseY", |c| Ok(c.push(f64::from(MS.with(|m| m.borrow().y)))));
    global(t, "getMouseXScaled", |c| {
        let z = shared_core().borrow().get_zoom(0);
        Ok(c.push(f64::from((MS.with(|m| m.borrow().x) as f32 * z) as i32)))
    });
    global(t, "getMouseYScaled", |c| {
        let z = shared_core().borrow().get_zoom(0);
        Ok(c.push(f64::from((MS.with(|m| m.borrow().y) as f32 * z) as i32)))
    });
    global(t, "setMouseXY", |c| {
        let (x, y) = (int_arg(c, 0), int_arg(c, 1));
        with_input(|i| i.request(WindowRequest::SetCursorPosition(x, y)));
        Ok(0)
    });
    global(t, "isMouseButtonDown", |c| Ok(c.push(mouse_is_button_down(int_arg(c, 0)))));
    global(t, "isMouseButtonPressed", |c| Ok(c.push(mouse_is_button_pressed(int_arg(c, 0)))));
    // `queueKeyEvent(lwjglKey)`: a press and a release on the next poll.
    global(t, "queueKeyEvent", |c| {
        let k = int_arg(c, 0);
        with_input(|i| {
            let nanos = i.nanos();
            let repeat = i.repeat_events;
            i.key_events.add_key_event(k, 1, repeat, nanos);
            i.key_events.add_key_event(k, 0, repeat, nanos);
        });
        Ok(0)
    });
    global(t, "queueCharEvent", |c| {
        let ch = str_arg(c, 0).and_then(|s| s.chars().next());
        let Some(ch) = ch else {
            return Err(LuaError::Runtime { message: "Index 0 out of bounds for length 0".into(), class: "java.lang.StringIndexOutOfBoundsException" });
        };
        with_input(|i| i.char(ch));
        Ok(0)
    });
}

/// Window requests (`setMouseXY`, cursor visibility, grabbing) for the
/// window layer.
pub fn take_window_requests() -> Vec<WindowRequest> {
    let vis = MS.with(|m| m.borrow().cursor_visible);
    with_input(|i| {
        let mut r = std::mem::take(&mut i.requests);
        r.push(WindowRequest::SetCursorVisible(vis));
        r
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_follow_init_key_names() {
        NAMES.with(|n| {
            let mut n = n.borrow_mut();
            n.constants = vec![
                ("KEY_W".into(), 17),
                ("KEY_NUMPAD7".into(), 71),
                ("KEY_NUMPADENTER".into(), 156),
                ("KEY_SPACE".into(), 57),
                ("KEY_LWIN".into(), 219),
                ("KEY_LMETA".into(), 219),
                ("KEY_MINUS".into(), 12),
            ];
            n.init();
            assert_eq!(n.names[17].as_deref(), Some("W"));
            assert_eq!(n.names[71].as_deref(), Some("KP_7"));
            assert_eq!(n.names[156].as_deref(), Some("KP_ENTER"));
            assert_eq!(n.names[57].as_deref(), Some("SPACE"));
            assert_eq!(n.names[219].as_deref(), Some("LMETA"));
            assert_eq!(n.names[12].as_deref(), Some("-"));
            assert_eq!(n.map.get("KP_7"), Some(&71));
        });
    }

    #[test]
    fn mouse_ui_check_and_right_delay() {
        with_input(|i| i.mouse_button(1, true));
        mouse_update(0.1);
        assert!(mouse_is_button_down(1));
        assert!(!mouse_is_button_down_ui_check(1), "right needs 0.15 s");
        mouse_update(0.1);
        assert!(mouse_is_button_down_ui_check(1));
        MS.with(|m| m.borrow_mut().ui_captured[1] = true);
        assert!(!mouse_is_button_down_ui_check(1));
        with_input(|i| i.mouse_button(1, false));
        mouse_update(0.1);
        assert!(!mouse_is_button_down_ui_check(1));
        assert!(!MS.with(|m| m.borrow().ui_captured[1]), "released clears the capture");
    }
}

/// Reads the `KEY_*` constants from an installed game:
/// `PZ_GAME_DIR=... cargo test -p pz-lua-api -- --ignored`.
#[cfg(test)]
#[test]
#[ignore]
fn installed_game_key_constants() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
    let consts = int_constants(Path::new(&dir), "org/lwjglx/input/Keyboard");
    let keys = consts.iter().filter(|(n, _)| n.starts_with("KEY_")).count();
    let mouse = int_constants(Path::new(&dir), "zombie/input/Mouse");
    eprintln!("Keyboard: {} constants ({keys} KEY_*); Mouse: {}", consts.len(), mouse.len());
    assert!(keys > 100);
    assert!(mouse.iter().any(|(n, _)| n == "BTN_OFFSET"));
}
