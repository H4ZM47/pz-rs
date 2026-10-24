//! Raw keyboard and mouse input as the original receives it from GLFW
//! through `org.lwjglx.input` (`Keyboard`, `Mouse`, `KeyEventQueue`): which
//! keys and buttons are held, the key/char event queue, cursor position and
//! accumulated scroll.
//!
//! Key codes are LWJGL 2 key codes (PC set-1 scan codes), the values of
//! `Keyboard.KEY_*` that Lua uses. The window layer feeds events in (see the
//! `winit` module); the game-facing logic (`GameKeyboard`, `zombie.input.Mouse`)
//! lives in `pz-lua-api`'s `bind_input`.

pub mod bindings;
pub mod controller;
#[cfg(feature = "gilrs")]
pub mod gamepads;
pub mod keys;
#[cfg(feature = "winit")]
pub mod winit;

use std::cell::RefCell;
use std::time::Instant;

/// `Keyboard.KEYBOARD_SIZE`.
pub const KEYBOARD_SIZE: usize = 256;
/// `org.lwjglx.input.Mouse.getButtonCount()`.
pub const MOUSE_BUTTONS: usize = 8;
/// `KeyEventQueue.MAX_EVENTS`.
pub const MAX_EVENTS: usize = 32;

/// `org.lwjglx.input.EventQueue`: a ring that drops the oldest event when
/// full.
#[derive(Clone, Debug)]
pub struct EventQueue {
    max: usize,
    current: isize,
    next: usize,
}

impl EventQueue {
    pub fn new(max: usize) -> Self {
        EventQueue { max, current: -1, next: 0 }
    }

    pub fn next_pos(&self) -> usize {
        self.next
    }

    /// The current event's slot; `None` before the first `next`.
    pub fn current_pos(&self) -> Option<usize> {
        usize::try_from(self.current).ok()
    }

    /// `add()`.
    pub fn add(&mut self) {
        self.next += 1;
        if self.next == self.max {
            self.next = 0;
        }
        if self.next as isize == self.current {
            self.current += 1;
            if self.current == self.max as isize {
                self.current = 0;
            }
        }
    }

    /// `next()`.
    pub fn advance(&mut self) -> bool {
        if self.current == self.next as isize - 1 {
            return false;
        }
        if self.next == 0 && self.current == self.max as isize - 1 {
            return false;
        }
        self.current += 1;
        if self.current == self.max as isize {
            self.current = 0;
        }
        true
    }
}

/// `org.lwjglx.input.KeyEventQueue`.
#[derive(Clone, Debug)]
pub struct KeyEventQueue {
    queue: EventQueue,
    keys: [i32; MAX_EVENTS],
    states: [bool; MAX_EVENTS],
    nanos: [i64; MAX_EVENTS],
    chars: [char; MAX_EVENTS],
}

impl Default for KeyEventQueue {
    fn default() -> Self {
        KeyEventQueue { queue: EventQueue::new(MAX_EVENTS), keys: [0; MAX_EVENTS], states: [false; MAX_EVENTS], nanos: [0; MAX_EVENTS], chars: ['\0'; MAX_EVENTS] }
    }
}

impl KeyEventQueue {
    /// `addKeyEvent(key, status)` with the key already as an LWJGL code;
    /// status 0 = release, 1 = press, 2 = repeat (kept only with repeat
    /// events enabled).
    pub fn add_key_event(&mut self, key: i32, status: i32, repeat_events: bool, nanos: i64) {
        match status {
            2 if !repeat_events => {}
            0..=2 => {
                let p = self.queue.next_pos();
                self.keys[p] = key;
                self.states[p] = status == 1 || status == 2;
                self.chars[p] = '\0';
                self.nanos[p] = nanos;
                self.queue.add();
            }
            _ => {}
        }
    }

    /// `addCharEvent(c)`.
    pub fn add_char_event(&mut self, c: char, nanos: i64) {
        let p = self.queue.next_pos();
        self.keys[p] = 0;
        self.states[p] = true;
        self.chars[p] = c;
        self.nanos[p] = nanos;
        self.queue.add();
    }

    /// `next()`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        self.queue.advance()
    }

    fn cur(&self) -> usize {
        self.queue.current_pos().unwrap_or(0)
    }

    pub fn event_key(&self) -> i32 {
        self.keys[self.cur()]
    }

    pub fn event_character(&self) -> char {
        self.chars[self.cur()]
    }

    pub fn event_key_state(&self) -> bool {
        self.states[self.cur()]
    }

    pub fn event_nanoseconds(&self) -> i64 {
        self.nanos[self.cur()]
    }
}

/// Requests from game code to the window (`Mouse.setGrabbed`,
/// `setCursorPosition`, cursor visibility), drained by the window layer.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowRequest {
    SetCursorPosition(i32, i32),
    SetGrabbed(bool),
    SetCursorVisible(bool),
}

/// Everything GLFW would report.
#[derive(Debug)]
pub struct InputState {
    /// Keys held (`glfwGetKey`), by LWJGL code.
    pub keys_down: [bool; KEYBOARD_SIZE],
    /// `Keyboard.enableRepeatEvents`.
    pub repeat_events: bool,
    /// The queue `GameKeyboard.getEventQueue()` reads.
    pub key_events: KeyEventQueue,
    /// Cursor position from the last move event, in window pixels with y
    /// measured from the top.
    pub cursor_x: f64,
    pub cursor_y: f64,
    /// `glfwGetMouseButton`.
    pub buttons_down: [bool; MOUSE_BUTTONS],
    /// `Mouse.scrollypos`: accumulated vertical scroll in lines.
    pub scroll_y: f64,
    pub focused: bool,
    pub requests: Vec<WindowRequest>,
    /// Printable key names learned from the keyboard layout (LWJGL code ->
    /// upper-cased character), as `glfwGetKeyName` reports them.
    pub layout_names: Vec<(i32, String)>,
    start: Instant,
}

impl Default for InputState {
    fn default() -> Self {
        InputState {
            keys_down: [false; KEYBOARD_SIZE],
            repeat_events: false,
            key_events: KeyEventQueue::default(),
            cursor_x: 0.0,
            cursor_y: 0.0,
            buttons_down: [false; MOUSE_BUTTONS],
            scroll_y: 0.0,
            focused: true,
            requests: Vec::new(),
            layout_names: Vec::new(),
            start: Instant::now(),
        }
    }
}

impl InputState {
    /// `Sys.getNanoTime()`.
    pub fn nanos(&self) -> i64 {
        self.start.elapsed().as_nanos() as i64
    }

    /// A GLFW key callback: press = 1, release = 0, repeat = 2.
    pub fn key(&mut self, lwjgl: i32, status: i32) {
        if let Some(d) = usize::try_from(lwjgl).ok().and_then(|k| self.keys_down.get_mut(k)) {
            *d = status != 0;
        }
        let (repeat, nanos) = (self.repeat_events, self.nanos());
        self.key_events.add_key_event(lwjgl, status, repeat, nanos);
    }

    /// A GLFW char callback.
    pub fn char(&mut self, c: char) {
        let nanos = self.nanos();
        self.key_events.add_char_event(c, nanos);
    }

    pub fn cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_x = x;
        self.cursor_y = y;
    }

    pub fn mouse_button(&mut self, button: usize, pressed: bool) {
        if let Some(b) = self.buttons_down.get_mut(button) {
            *b = pressed;
        }
    }

    /// A GLFW scroll callback (vertical offset in lines).
    pub fn scroll(&mut self, dy: f64) {
        self.scroll_y += dy;
    }

    /// `org.lwjglx.input.Mouse.getDWheel()`: whole lines, then reset.
    pub fn take_wheel(&mut self) -> i32 {
        let w = self.scroll_y as i32;
        self.scroll_y = 0.0;
        w
    }

    /// GLFW stops reporting held keys and buttons when focus is lost.
    pub fn focus(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.keys_down = [false; KEYBOARD_SIZE];
            self.buttons_down = [false; MOUSE_BUTTONS];
        }
    }

    pub fn request(&mut self, r: WindowRequest) {
        self.requests.push(r);
    }

    /// Records the layout's character for a printable key.
    pub fn learn_key_name(&mut self, lwjgl: i32, c: char) {
        if !keys::is_printable(lwjgl) || c.is_control() || c == ' ' {
            return;
        }
        let name: String = c.to_uppercase().collect();
        match self.layout_names.iter_mut().find(|(k, _)| *k == lwjgl) {
            Some(e) => e.1 = name,
            None => self.layout_names.push((lwjgl, name)),
        }
    }
}

thread_local! {
    static INPUT: RefCell<InputState> = RefCell::new(InputState::default());
    static CONTROLLERS: RefCell<controller::Controllers> = RefCell::new(controller::Controllers::default());
}

/// The controllers the backend (`gamepads`) keeps up to date.
pub fn with_controllers<R>(f: impl FnOnce(&mut controller::Controllers) -> R) -> R {
    CONTROLLERS.with(|c| f(&mut c.borrow_mut()))
}

/// The input state of this (the game) thread.
pub fn with_input<R>(f: impl FnOnce(&mut InputState) -> R) -> R {
    INPUT.with(|i| f(&mut i.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_queue_matches_lwjglx() {
        let mut q = KeyEventQueue::default();
        assert!(!q.next());
        q.add_key_event(30, 1, false, 0);
        q.add_key_event(30, 2, false, 0);
        q.add_char_event('a', 0);
        q.add_key_event(30, 0, false, 0);
        let mut seen = Vec::new();
        while q.next() {
            seen.push((q.event_key(), q.event_key_state(), q.event_character()));
        }
        assert_eq!(seen, [(30, true, '\0'), (0, true, 'a'), (30, false, '\0')]);
    }

    #[test]
    fn full_queue_drops_the_oldest() {
        let mut q = KeyEventQueue::default();
        for i in 0..40 {
            q.add_key_event(i, 1, false, 0);
        }
        let mut seen = Vec::new();
        while q.next() {
            seen.push(q.event_key());
        }
        assert_eq!(seen.last(), Some(&39));
        assert!(seen.len() < MAX_EVENTS);
    }

    #[test]
    fn wheel_truncates_and_resets() {
        let mut s = InputState::default();
        s.scroll(0.6);
        s.scroll(0.6);
        assert_eq!(s.take_wheel(), 1);
        assert_eq!(s.take_wheel(), 0);
    }
}
