//! Feeds winit window events into [`crate::InputState`] the way GLFW's
//! key, char, cursor, button and scroll callbacks feed `org.lwjglx.input`.

use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key, KeyCode, PhysicalKey};
#[cfg(not(target_arch = "wasm32"))]
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::InputState;

/// Lines per pixel for trackpad (pixel-delta) scrolling; GLFW on macOS
/// reports such deltas in tenths of a line.
const PIXELS_PER_LINE: f64 = 10.0;

/// The LWJGL key code of a physical key, if GLFW has one.
pub fn lwjgl_key(code: KeyCode) -> Option<i32> {
    use KeyCode::*;
    Some(match code {
        Escape => 1,
        Digit1 => 2,
        Digit2 => 3,
        Digit3 => 4,
        Digit4 => 5,
        Digit5 => 6,
        Digit6 => 7,
        Digit7 => 8,
        Digit8 => 9,
        Digit9 => 10,
        Digit0 => 11,
        Minus => 12,
        Equal => 13,
        Backspace => 14,
        Tab => 15,
        KeyQ => 16,
        KeyW => 17,
        KeyE => 18,
        KeyR => 19,
        KeyT => 20,
        KeyY => 21,
        KeyU => 22,
        KeyI => 23,
        KeyO => 24,
        KeyP => 25,
        BracketLeft => 26,
        BracketRight => 27,
        Enter => 28,
        ControlLeft => 29,
        KeyA => 30,
        KeyS => 31,
        KeyD => 32,
        KeyF => 33,
        KeyG => 34,
        KeyH => 35,
        KeyJ => 36,
        KeyK => 37,
        KeyL => 38,
        Semicolon => 39,
        Quote => 40,
        Backquote => 41,
        ShiftLeft => 42,
        Backslash => 43,
        KeyZ => 44,
        KeyX => 45,
        KeyC => 46,
        KeyV => 47,
        KeyB => 48,
        KeyN => 49,
        KeyM => 50,
        Comma => 51,
        Period => 52,
        Slash => 53,
        ShiftRight => 54,
        NumpadMultiply => 55,
        AltLeft => 56,
        Space => 57,
        CapsLock => 58,
        F1 => 59,
        F2 => 60,
        F3 => 61,
        F4 => 62,
        F5 => 63,
        F6 => 64,
        F7 => 65,
        F8 => 66,
        F9 => 67,
        F10 => 68,
        NumLock => 69,
        ScrollLock => 70,
        Numpad7 => 71,
        Numpad8 => 72,
        Numpad9 => 73,
        NumpadSubtract => 74,
        Numpad4 => 75,
        Numpad5 => 76,
        Numpad6 => 77,
        NumpadAdd => 78,
        Numpad1 => 79,
        Numpad2 => 80,
        Numpad3 => 81,
        Numpad0 => 82,
        NumpadDecimal => 83,
        F11 => 87,
        F12 => 88,
        F13 => 100,
        F14 => 101,
        F15 => 102,
        F16 => 103,
        F17 => 104,
        F18 => 105,
        NumpadEqual => 141,
        NumpadEnter => 156,
        ControlRight => 157,
        NumpadDivide => 181,
        PrintScreen => 183,
        AltRight => 184,
        Pause => 197,
        Home => 199,
        ArrowUp => 200,
        PageUp => 201,
        ArrowLeft => 203,
        ArrowRight => 205,
        End => 207,
        ArrowDown => 208,
        PageDown => 209,
        Insert => 210,
        Delete => 211,
        SuperLeft => 219,
        SuperRight => 220,
        ContextMenu => 221,
        _ => return None,
    })
}

/// GLFW's mouse button numbering.
pub fn glfw_button(b: MouseButton) -> Option<usize> {
    match b {
        MouseButton::Left => Some(0),
        MouseButton::Right => Some(1),
        MouseButton::Middle => Some(2),
        MouseButton::Back => Some(3),
        MouseButton::Forward => Some(4),
        MouseButton::Other(n) => usize::from(n).checked_add(5).filter(|&n| n < crate::MOUSE_BUTTONS),
    }
}

/// Applies a window event. Returns true if it was an input event.
pub fn handle(state: &mut InputState, event: &WindowEvent) -> bool {
    match event {
        WindowEvent::KeyboardInput { event, is_synthetic: false, .. } => {
            key_event(state, event);
            true
        }
        WindowEvent::CursorMoved { position, .. } => {
            state.cursor_moved(position.x, position.y);
            true
        }
        WindowEvent::MouseInput { state: s, button, .. } => {
            if let Some(b) = glfw_button(*button) {
                state.mouse_button(b, *s == ElementState::Pressed);
            }
            true
        }
        WindowEvent::MouseWheel { delta, .. } => {
            let dy = match delta {
                MouseScrollDelta::LineDelta(_, y) => f64::from(*y),
                MouseScrollDelta::PixelDelta(p) => p.y / PIXELS_PER_LINE,
            };
            state.scroll(dy);
            true
        }
        WindowEvent::Focused(f) => {
            state.focus(*f);
            true
        }
        _ => false,
    }
}

fn key_event(state: &mut InputState, e: &KeyEvent) {
    let PhysicalKey::Code(code) = e.physical_key else { return };
    let Some(k) = lwjgl_key(code) else { return };
    // The layout's character for this key, as glfwGetKeyName reports it.
    if let Key::Character(s) = &e.key_without_modifiers() {
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            state.learn_key_name(k, c);
        }
    }
    let status = match (e.state, e.repeat) {
        (ElementState::Released, _) => 0,
        (ElementState::Pressed, false) => 1,
        (ElementState::Pressed, true) => 2,
    };
    state.key(k, status);
    // The char callback: typed text, without control characters.
    if e.state == ElementState::Pressed {
        if let Some(text) = &e.text {
            for c in text.chars().filter(|c| !c.is_control()) {
                state.char(c);
            }
        }
    }
}
