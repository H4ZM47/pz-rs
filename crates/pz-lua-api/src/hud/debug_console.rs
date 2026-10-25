//! `zombie.ui.UIDebugConsole`: the in-game Lua console (output log with a
//! scroll bar, a command line with history and method suggestions, edge
//! resizing, the captured debug log).

use super::host::{with_host, ConsoleMethod};
use super::new_window::{th, WindowCore};
use super::scroll_bar::ScrollBar;
use crate::bind_uitextbox2::{TextBoxHooks, UINineGrid, UITextBox2};
use pz_kahlua::{Thread, Value};
use pz_render::Rgba;
use pz_text::UIFont;
use pz_ui::draw::{font_height, measure_string_x};
use pz_ui::{with_ui, UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub const UI_DEBUG_CONSOLE: &str = "zombie.ui.UIDebugConsole";
pub const COMMAND_ENTRY: &str = "zombie.ui.UIDebugConsole$CommandEntry";
const SUPERS: &[&str] = &[super::new_window::NEW_WINDOW, "zombie.ui.UIElement"];
const FONT: Option<UIFont> = Some(UIFont::DebugConsole);
const CONSOLE_VERSION: &str = "v1.1.0";
/// `outputBB`'s capacity.
const OUTPUT_CAPACITY: usize = 8192;

/// The original's command list (shown nowhere; kept for `availableCommands`).
pub const AVAILABLE_COMMANDS: [&str; 6] =
    ["?", "help", "commands", "clr", "AddInvItem", "SpawnZombie"];

thread_local! {
    /// `UIDebugConsole.instance`.
    static INSTANCE: RefCell<Weak<UIElement>> = const { RefCell::new(Weak::new()) };
    /// `outputBB` and `outputChanged`.
    static OUTPUT: RefCell<(Vec<u8>, bool)> = const { RefCell::new((Vec::new(), false)) };
}

/// `UIDebugConsole.instance`.
pub fn instance() -> Option<Rc<UIElement>> {
    INSTANCE.with(|i| i.borrow().upgrade())
}

fn with_console<R>(f: impl FnOnce(&Rc<UIElement>, &UIDebugConsole) -> R) -> Option<R> {
    let e = instance()?;
    let c = e.ext::<UIDebugConsole>()?;
    Some(f(&e, c))
}

/// `UIDebugConsole.instance != null && el == instance.outputLog`.
pub fn is_output_log(el: &UIElement) -> bool {
    with_console(|_, c| std::ptr::eq(Rc::as_ptr(&c.output_log), el)).unwrap_or(false)
}

/// `UIManager.getDebugConsole() != null && el == getDebugConsole().commandLine`.
pub fn is_command_line(el: &UIElement) -> bool {
    let Some(e) = with_ui(|m| m.debug_console.clone()) else {
        return false;
    };
    e.ext::<UIDebugConsole>()
        .is_some_and(|c| std::ptr::eq(Rc::as_ptr(&c.command_line), el))
}

/// `UITextBox2.onKeyEnter`'s permission check before `ProcessCommand`.
pub fn may_process_commands() -> bool {
    with_host(|h| h.network.may_process_console_commands())
}

/// `UIManager.getDebugConsole().ProcessCommand()`.
pub fn process_command(t: &mut Thread) {
    if let Some(e) = with_ui(|m| m.debug_console.clone()) {
        if let Some(c) = e.ext::<UIDebugConsole>() {
            c.process_command(&e, t);
        }
    }
}

/// `DebugLog.OutputStreamWrapper.write`: the debug log's bytes reach the
/// console when `Core.debug`, a console exists and the option is on.
pub fn debug_log_write(b: &[u8]) {
    let debug = crate::bind_core::shared_core().borrow().debug;
    if debug && instance().is_some() && with_host(|h| h.debug.ui_debug_console_debug_log()) {
        add_output(b);
    }
}

/// `addOutput(b, off, len)`: append to the 8 KiB buffer (dropped whole
/// when it would overflow); a trailing newline marks it changed.
pub fn add_output(b: &[u8]) {
    if b.is_empty() {
        return;
    }
    let b = if b.len() > OUTPUT_CAPACITY {
        &b[b.len() - OUTPUT_CAPACITY..]
    } else {
        b
    };
    OUTPUT.with(|o| {
        let mut o = o.borrow_mut();
        if o.0.len() + b.len() > OUTPUT_CAPACITY {
            o.0.clear();
        }
        o.0.extend_from_slice(b);
        if b[b.len() - 1] == b'\n' {
            o.1 = true;
        }
    });
}

/// Java's `String.split(regex)` for a literal one-character separator:
/// trailing empty strings are removed.
fn java_split(s: &str, sep: char) -> Vec<String> {
    let mut v: Vec<String> = s.split(sep).map(str::to_string).collect();
    while v.len() > 1 && v.last().is_some_and(|l| l.is_empty()) {
        v.pop();
    }
    if v.len() == 1 && v[0].is_empty() && !s.is_empty() {
        v.clear();
    }
    v
}

/// `levenshteinDistance(lhs, rhs)` (over UTF-16 units, as Java's
/// `CharSequence`).
pub fn levenshtein_distance(lhs: &str, rhs: &str) -> i32 {
    let l: Vec<u16> = lhs.encode_utf16().collect();
    let r: Vec<u16> = rhs.encode_utf16().collect();
    let len0 = l.len() + 1;
    let mut cost: Vec<i32> = (0..len0 as i32).collect();
    let mut newcost = vec![0i32; len0];
    for j in 1..r.len() + 1 {
        newcost[0] = j as i32;
        for i in 1..len0 {
            let m = if l[i - 1] == r[j - 1] { 0 } else { 1 };
            let replace = cost[i - 1] + m;
            let insert = cost[i] + 1;
            let delete = newcost[i - 1] + 1;
            newcost[i] = insert.min(delete).min(replace);
        }
        std::mem::swap(&mut cost, &mut newcost);
    }
    cost[len0 - 1]
}

/// `UIDebugConsole$CommandEntry`: the command line's history and Tab keys.
struct CommandEntry;

impl TextBoxHooks for CommandEntry {
    fn class_name(&self) -> Option<&'static str> {
        Some(COMMAND_ENTRY)
    }
    fn on_press_up(&self, _el: &UIElement, t: &mut Thread) -> bool {
        with_console(|e, c| c.history_prev(e, t));
        true
    }
    fn on_press_down(&self, _el: &UIElement, t: &mut Thread) -> bool {
        with_console(|e, c| c.history_next(e, t));
        true
    }
    fn on_other_key(&self, _el: &UIElement, t: &mut Thread, key: i32) -> bool {
        with_console(|_, c| c.on_other_key(t, key));
        true
    }
}

/// `zombie.ui.UIDebugConsole`.
pub struct UIDebugConsole {
    pub core: WindowCore,
    pub scroll_bar_v: Rc<UIElement>,
    pub output_log: Rc<UIElement>,
    pub command_line: Rc<UIElement>,
    pub autosuggest: Rc<UIElement>,
    resizing: Cell<bool>,
    resize_width: Cell<bool>,
    resize_height: Cell<bool>,
    resize_start_x: Cell<i32>,
    resize_start_y: Cell<i32>,
    resize_start_width: Cell<i32>,
    resize_start_height: Cell<i32>,
    inputlength: Cell<usize>,
    previous: RefCell<Vec<String>>,
    global_lua_methods: RefCell<Vec<ConsoleMethod>>,
    pub previous_index: Cell<i32>,
    prev_suggestion: RefCell<Option<ConsoleMethod>>,
}

fn tb(e: &UIElement) -> &UITextBox2 {
    e.ext::<UITextBox2>().expect("UITextBox2")
}

fn sb(e: &UIElement) -> &ScrollBar {
    e.ext::<ScrollBar>().expect("ScrollBar")
}

fn mouse_xa_ya() -> (i32, i32) {
    with_ui(|m| (m.mouse.x, m.mouse.y))
}

impl UIDebugConsole {
    /// `new UIDebugConsole(x, y)`.
    pub fn create(t: &mut Thread, x: i32, y: i32) -> Rc<UIElement> {
        let core = WindowCore::new();
        let (screen_w, version, debug) = {
            let c = crate::bind_core::shared_core();
            let c = c.borrow();
            (c.get_screen_width(), c.get_version(), c.debug)
        };
        let width = 640.max(screen_w / 3);
        let font_hgt = font_height(FONT);
        let title_h = th(&core.title_middle) as i32;
        let banner = format!(
            "Project Zomboid - {version}\nDebug Console - {CONSOLE_VERSION}\n(C) Indie Stone Studios 2025\n---------------------------------------------------------------------------------------------------------------------------\n\n"
        );
        let output_log = UITextBox2::create(
            t,
            FONT,
            5,
            title_h + font_hgt,
            width - 10,
            font_hgt * 11 + 10,
            &banner,
            true,
        );
        {
            let mut s = tb(&output_log).state();
            s.multiple_line = true;
            s.always_paginate = false;
            s.wrap_lines = false;
        }
        let out_bottom = (output_log.get_y() + output_log.get_height()) as i32;
        let command_line = UITextBox2::create(
            t,
            FONT,
            5,
            out_bottom + font_hgt,
            width - 10,
            font_hgt + 10,
            "",
            true,
        );
        tb(&command_line).set_hooks(Rc::new(CommandEntry));
        {
            let mut s = tb(&command_line).state();
            s.is_editable = true;
            s.text_entry_max_length = 256;
        }
        let as_h = font_hgt + 10;
        let autosuggest = UITextBox2::create(t, FONT, 5, out_bottom - as_h, 15, as_h, "", true);
        if let Some(f) = tb(&autosuggest).state().frame.clone() {
            if let Some(g) = f.ext::<UINineGrid>() {
                g.color.set(Rgba(1.0, 1.0, 1.0, 1.0));
            }
        }
        let scroll = ScrollBar::create(
            "UIDebugConsoleScrollbar",
            (output_log.get_x() + output_log.get_width()) as i32 - 14,
            output_log.get_y() as i32 + 4,
            output_log.get_height() as i32 - 8,
        );
        sb(&scroll).set_parent_text_box(&output_log);
        let c = UIDebugConsole {
            core,
            scroll_bar_v: scroll.clone(),
            output_log: output_log.clone(),
            command_line: command_line.clone(),
            autosuggest: autosuggest.clone(),
            resizing: Cell::new(false),
            resize_width: Cell::new(false),
            resize_height: Cell::new(false),
            resize_start_x: Cell::new(0),
            resize_start_y: Cell::new(0),
            resize_start_width: Cell::new(0),
            resize_start_height: Cell::new(0),
            inputlength: Cell::new(0),
            previous: RefCell::default(),
            global_lua_methods: RefCell::default(),
            previous_index: Cell::new(0),
            prev_suggestion: RefCell::new(None),
        };
        let e = UIElement::new_ext(None, Box::new(c));
        let me = e.ext::<UIDebugConsole>().unwrap();
        // super(x, y, 10, 10, true) runs first in the original; the text
        // boxes only read the title texture, so the order is the same.
        me.core.init(&e, x, y, 10, 10, true);
        me.core.resize_to_fit_y.set(false);
        e.set_visible(true);
        INSTANCE.with(|i| *i.borrow_mut() = Rc::downgrade(&e));
        {
            let mut s = e.st_mut();
            s.width = width as f32;
            s.height = ((command_line.get_y() + command_line.get_height()) as i32 + 6) as f32;
        }
        for c in [&output_log, &scroll, &command_line, &autosuggest] {
            e.add_child(c);
        }
        *me.global_lua_methods.borrow_mut() = with_host(|h| h.console_methods.global_methods());
        if debug {
            pz_kahlua::stdlib::set_print_callback(Some(Rc::new(|t: &mut Thread, s: &str| {
                if let Some((e, _)) = instance().map(|e| (e, ())) {
                    e.ext::<UIDebugConsole>().unwrap().spool_text(t, s);
                }
            })));
        }
        e
    }

    fn set_new_size(&self, el: &UIElement, t: &mut Thread, new_w: i32, new_h: i32) {
        let (sw, sh) = {
            let c = crate::bind_core::shared_core();
            let c = c.borrow();
            (c.get_screen_width(), c.get_screen_height())
        };
        let new_w = pz_clamp(new_w, 640, sw - 50);
        let mut new_h = pz_clamp(new_h, 200, sh - 50);
        let out = &self.output_log;
        let other_h = el.get_height() as i32 - out.get_height() as i32;
        let output_h = new_h - other_h;
        let (font, inset) = {
            let s = tb(out).state();
            (s.font, s.inset())
        };
        let line_h = font_height(font);
        let num_lines = if line_h == 0 {
            0
        } else {
            (output_h - inset * 2) / line_h
        };
        new_h = other_h + num_lines * line_h + inset * 2;
        if new_w != el.get_width() as i32 {
            el.set_width(new_w as f64);
            out.set_width(el.get_width() - 10.0);
            if let Some(f) = tb(out).state().frame.clone() {
                f.set_width(out.get_width());
            }
            self.scroll_bar_v
                .set_x(((out.get_x() + out.get_width()) as i32 - 14) as f64);
            self.command_line.set_width(el.get_width() - 10.0);
            if let Some(f) = tb(&self.command_line).state().frame.clone() {
                f.set_width(self.command_line.get_width());
            }
        }
        if new_h != el.get_height() as i32 {
            let font_hgt = font_height(FONT);
            el.set_height(new_h as f64);
            let cl = &self.command_line;
            cl.set_y((el.get_height() as i32 - 6) as f64 - cl.get_height());
            if let Some(f) = tb(cl).state().frame.clone() {
                f.set_y(0.0);
            }
            out.set_height(cl.get_y() - font_hgt as f64 - out.get_y());
            if let Some(f) = tb(out).state().frame.clone() {
                f.set_height(out.get_height());
            }
            out.update(t);
            let bar = sb(&self.scroll_bar_v);
            bar.set_height(&self.scroll_bar_v, (out.get_height() as i32 - 8) as f64);
            self.scroll_bar_v.update(t);
            bar.scroll_to_bottom(&self.scroll_bar_v);
            self.autosuggest
                .set_y(out.get_y() + out.get_height() - self.autosuggest.get_height());
        }
    }

    /// `ProcessCommand()`: run the command line's text as Lua in the global
    /// environment; `clear` empties the log.
    pub fn process_command(&self, _el: &UIElement, t: &mut Thread) {
        let cl = &self.command_line;
        let text: String = {
            let mut s = tb(cl).state();
            let v: String = s.internal.iter().collect();
            s.internal.clear();
            v
        };
        let process = text.trim_matches(|c: char| c <= ' ').to_string();
        {
            let mut prev = self.previous.borrow_mut();
            if prev.last() != Some(&process) {
                prev.push(process.clone());
            }
            self.previous_index.set(prev.len() as i32);
        }
        tb(cl).state().doing_text_entry = true;
        crate::bind_uitextbox2::set_current(Some(cl.clone()));
        crate::bind_uitextbox2::sync_core();
        if process == "clear" {
            tb(&self.output_log).state().text_changed = true;
            tb(&self.output_log).clear_input(&self.output_log, t);
            return;
        }
        if with_host(|h| h.debug.ui_debug_console_echo_command()) {
            self.spool_text(t, &format!("[USER] - \"{process}\"."));
        }
        let env = t.globals().clone();
        match pz_kahlua::stdlib::load_string(&process, Some("console"), env) {
            Ok(f) => {
                // LuaCaller.protectedCall: a runtime error is reported by
                // the thread (to the debug log), not here.
                let _ = t.pcall(&f, &[]);
            }
            Err(e) => {
                let msg = match &e {
                    pz_kahlua::LuaError::Kahlua(v) => pz_kahlua::exec::java_value_of(v),
                    pz_kahlua::LuaError::Runtime { message, .. } => message.clone(),
                };
                self.spool_text(t, &msg);
            }
        }
    }

    fn history_prev(&self, _el: &UIElement, t: &mut Thread) {
        let prev = self.previous.borrow().clone();
        let mut i = self.previous_index.get() - 1;
        if i < 0 {
            i = 0;
        }
        self.previous_index.set(i);
        if i >= 0 && (i as usize) < prev.len() {
            let v: Vec<char> = prev[i as usize].chars().collect();
            tb(&self.command_line).set_text(&self.command_line, t, &v);
        }
    }

    fn history_next(&self, _el: &UIElement, t: &mut Thread) {
        let prev = self.previous.borrow().clone();
        let mut i = self.previous_index.get() + 1;
        if i >= prev.len() as i32 {
            i = prev.len() as i32 - 1;
        }
        self.previous_index.set(i);
        if i >= 0 && (i as usize) < prev.len() {
            let v: Vec<char> = prev[i as usize].chars().collect();
            tb(&self.command_line).set_text(&self.command_line, t, &v);
        }
    }

    /// `onOtherKey(key)`: Tab completes the last suggestion.
    pub fn on_other_key(&self, t: &mut Thread, key: i32) {
        if key != 15 {
            return;
        }
        let Some(sug) = self.prev_suggestion.borrow().clone() else {
            return;
        };
        let text = self.command_line_text();
        let mut cmd = java_split(&text, ':');
        let mut out = String::new();
        if !cmd.is_empty() {
            let n = cmd.len();
            cmd[n - 1] = sug.name.clone();
            out = cmd.join(":");
        }
        out.push_str(if sug.param_count == 0 { "()" } else { "(" });
        let v: Vec<char> = out.chars().collect();
        tb(&self.command_line).set_text(&self.command_line, t, &v);
    }

    fn command_line_text(&self) -> String {
        tb(&self.command_line).state().text.iter().collect()
    }

    /// `UpdateViewPos()`.
    fn update_view_pos(&self) {
        {
            let mut s = tb(&self.output_log).state();
            s.top_line_index = (s.lines.len() as i32 - s.num_visible_lines).max(0);
        }
        sb(&self.scroll_bar_v).scroll_to_bottom(&self.scroll_bar_v);
    }

    /// `SpoolText(line)`: append a line to the log and scroll to it.
    pub fn spool_text(&self, t: &mut Thread, line: &str) {
        let out = &self.output_log;
        let v: Vec<char> = {
            let mut s = tb(out).state();
            s.text_changed = true;
            s.text
                .iter()
                .copied()
                .chain(line.chars())
                .chain(std::iter::once('\n'))
                .collect()
        };
        tb(out).set_text(out, t, &v);
        self.update_view_pos();
    }

    /// `ClearConsole()`.
    pub fn clear_console(&self, t: &mut Thread) {
        tb(&self.output_log).state().text_changed = true;
        tb(&self.output_log).set_text(&self.output_log, t, &[]);
        self.update_view_pos();
    }

    /// `SuggestionEngine(input, methods)`: the least Levenshtein distance,
    /// the first on ties.
    fn suggestion_engine(&self, input: &str, methods: &[ConsoleMethod]) -> Option<ConsoleMethod> {
        let mut lowest = 0;
        let mut m: Option<&ConsoleMethod> = None;
        for f in methods {
            let cost = levenshtein_distance(input, &f.name);
            if m.is_none() || cost < lowest {
                lowest = cost;
                m = Some(f);
            }
        }
        let m = m.cloned();
        *self.prev_suggestion.borrow_mut() = m.clone();
        m
    }

    /// `handleOutput()`: move the captured debug log into the output log,
    /// keeping its last 8192 characters from a line start.
    fn handle_output(&self, t: &mut Thread) {
        let bytes = OUTPUT.with(|o| {
            let mut o = o.borrow_mut();
            if !o.1 {
                return None;
            }
            o.1 = false;
            Some(std::mem::take(&mut o.0))
        });
        let Some(bytes) = bytes else { return };
        let text = String::from_utf8_lossy(&bytes);
        let out = &self.output_log;
        let all: Vec<char> = {
            let mut s = tb(out).state();
            s.text_changed = true;
            s.text.iter().copied().chain(text.chars()).collect()
        };
        tb(out).set_text(out, t, &all);
        let cur: Vec<char> = tb(out).state().text.clone();
        if cur.len() > OUTPUT_CAPACITY {
            let mut start = cur.len() - OUTPUT_CAPACITY;
            while start < cur.len() && cur[start] != '\n' {
                start += 1;
            }
            // With no newline the original's substring throws and the
            // (swallowed) exception keeps the long text.
            if start < cur.len() {
                tb(out).state().text_changed = true;
                tb(out).set_text(out, t, &cur[start + 1..]);
            }
        }
        self.update_view_pos();
    }

    /// The suggestion part of `update()`; a suggestion to show is left in
    /// `PENDING` for `update`, which has the thread.
    fn update_suggestion(&self) {
        let text = self.command_line_text();
        let len = text.encode_utf16().count();
        if len != self.inputlength.get() && !text.is_empty() {
            self.inputlength.set(len);
            let cmd = java_split(&text, ':');
            let mut input = String::new();
            if !cmd.is_empty() {
                input = cmd[cmd.len() - 1].clone();
                if input.is_empty() && self.autosuggest.is_visible() {
                    self.autosuggest.set_visible(false);
                    return;
                }
            }
            let mut m = None;
            if cmd.len() > 1
                && cmd[0].find(')').is_some_and(|i| i > 0)
                && !cmd[cmd.len() - 1].contains('(')
            {
                let mut methods = self.global_lua_methods.borrow().clone();
                for s in &cmd {
                    if !s.find(')').is_some_and(|i| i > 0) {
                        continue;
                    }
                    let s = s.split('(').next().unwrap_or("");
                    // The original replaces the list while walking a
                    // snapshot of it (it raises no
                    // ConcurrentModificationException), so a later
                    // overload of the same name wins.
                    let snapshot = methods.clone();
                    for f in &snapshot {
                        if f.name != s {
                            continue;
                        }
                        methods =
                            with_host(|h| h.console_methods.public_methods_of(&f.return_type));
                    }
                }
                m = self.suggestion_engine(&input, &methods);
            } else if cmd.len() == 1 {
                let g = self.global_lua_methods.borrow().clone();
                m = self.suggestion_engine(&input, &g);
            }
            if let Some(m) = m {
                let mut rt = "void".to_string();
                if m.return_type != "void" {
                    rt = m.return_type.rsplit('.').next().unwrap_or("").to_string();
                }
                if !self.autosuggest.is_visible() {
                    self.autosuggest.set_visible(true);
                }
                let s = format!("<{rt}> {}", m.name);
                PENDING.with(|p| *p.borrow_mut() = Some((s, len)));
            }
        } else if text.is_empty() && self.autosuggest.is_visible() {
            self.autosuggest.set_visible(false);
        }
    }
}

thread_local! {
    /// A suggestion to show, set by `update_suggestion` and applied by
    /// `update` (which has the thread).
    static PENDING: RefCell<Option<(String, usize)>> = const { RefCell::new(None) };
}

fn pz_clamp(v: i32, lo: i32, hi: i32) -> i32 {
    // PZMath.clamp(int, min, max): min wins when max < min.
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

impl UIElementExt for UIDebugConsole {
    fn class_name(&self) -> &'static str {
        UI_DEBUG_CONSOLE
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn button_clicked(&self, el: &UIElement, name: Option<Rc<str>>) {
        self.core.button_clicked(el, name)
    }
    fn on_mouse_move_outside(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) {
        self.core.on_mouse_move_outside(el, t, dx, dy)
    }

    /// A press on the right or bottom 10 pixels starts a resize.
    fn on_mouse_down(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        self.core.on_mouse_down(el, t, x, y);
        if self
            .scroll_bar_v
            .is_point_over(el.get_absolute_x() + x, el.get_absolute_y() + y)
        {
            return false;
        }
        let (w, h) = (el.get_width(), el.get_height());
        if !self.core.moving.get() && (x >= w - 10.0 || y >= h - 10.0) {
            self.resizing.set(true);
            self.resize_width.set(x >= w - 10.0);
            self.resize_height.set(y >= h - 10.0);
            let (mx, my) = mouse_xa_ya();
            self.resize_start_x.set(mx);
            self.resize_start_y.set(my);
            self.resize_start_width.set(w as i32);
            self.resize_start_height.set(h as i32);
            el.set_capture(true);
            return true;
        }
        false
    }

    fn on_mouse_up(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        self.core.on_mouse_up(el, t, x, y);
        if self.resizing.get() {
            self.resizing.set(false);
            el.set_capture(false);
            return true;
        }
        false
    }

    fn on_mouse_up_outside(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) {
        self.on_mouse_up(el, t, x, y);
    }

    fn on_mouse_move(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        self.core.on_mouse_move(el, t, dx, dy);
        if self.resizing.get() {
            let (mx, my) = mouse_xa_ya();
            if self.resize_width.get() {
                let d = mx - self.resize_start_x.get();
                self.set_new_size(
                    el,
                    t,
                    self.resize_start_width.get() + d,
                    el.get_height() as i32,
                );
            }
            if self.resize_height.get() {
                let d = my - self.resize_start_y.get();
                self.set_new_size(
                    el,
                    t,
                    el.get_width() as i32,
                    self.resize_start_height.get() + d,
                );
            }
            return true;
        }
        false
    }

    /// `render()`: the window, the three labels, the resize edges.
    fn render(&self, el: &UIElement, t: &mut Thread) {
        if !el.is_visible() {
            return;
        }
        self.core.render(el, t);
        let tr = crate::bind_text::translate;
        el.draw_text_centre(
            FONT,
            &tr("IGUI_CommandConsole"),
            el.get_width() / 2.0,
            2.0,
            1.0,
            1.0,
            1.0,
            1.0,
        );
        el.draw_text(
            FONT,
            Some(&tr("IGUI_OutputLog")),
            7.0,
            th(&self.core.title_middle),
            0.7f32 as f64,
            0.7f32 as f64,
            1.0,
            1.0,
        );
        let out = &self.output_log;
        el.draw_text(
            FONT,
            Some(&tr("IGUI_LuaCommandLine")),
            7.0,
            out.get_y() + out.get_height() + 1.0,
            0.7f32 as f64,
            0.7f32 as f64,
            1.0,
            1.0,
        );
        let bar = sb(&self.scroll_bar_v);
        let resizing = self.resizing.get();
        if resizing
            || el.is_mouse_over() && !self.scroll_bar_v.is_mouse_over() && !bar.is_being_dragged()
        {
            let (mx, my) = mouse_xa_ya();
            let mx = mx - el.get_absolute_x() as i32;
            let my = my - el.get_absolute_y() as i32;
            let (w, h) = (el.get_width(), el.get_height());
            let rgb = if resizing { 1.0 } else { 0.66 };
            if resizing && self.resize_width.get() || !resizing && mx >= w as i32 - 10 && my >= 18 {
                el.draw_texture_scaled_col(None, w - 10.0, 0.0, 10.0, h, rgb, rgb, rgb, 0.66);
            }
            if resizing && self.resize_height.get() || !resizing && my >= h as i32 - 10 {
                el.draw_texture_scaled_col(None, 0.0, h - 10.0, w, 10.0, rgb, rgb, rgb, 0.66);
            }
        }
    }

    /// `update()`: keep the size on whole lines, take the debug log, the
    /// window update, then the method suggestion for the command line.
    fn update(&self, el: &UIElement, t: &mut Thread) {
        if !el.is_visible() {
            return;
        }
        self.set_new_size(el, t, el.get_width() as i32, el.get_height() as i32);
        self.handle_output(t);
        self.core.update(el, t);
        PENDING.with(|p| p.borrow_mut().take());
        self.update_suggestion();
        if let Some((s, cmd_len)) = PENDING.with(|p| p.borrow_mut().take()) {
            let a = &self.autosuggest;
            let v: Vec<char> = s.chars().collect();
            tb(a).set_text(a, t, &v);
            a.set_x((5 * cmd_len) as f64);
            let (font, inset, text) = {
                let s = tb(a).state();
                (s.font, s.inset(), s.text.iter().collect::<String>())
            };
            a.set_width((inset * 2 + measure_string_x(font, &text)) as f64);
            if let Some(f) = tb(a).state().frame.clone() {
                f.st_mut().width = a.get_width() as f32;
            }
        }
    }
}

/// Lua: the methods of the exposed class.
pub(crate) fn register(thread: &mut Thread) {
    use crate::bind_ui::element_of;
    use crate::expose::{expose_class, int_arg};
    let b = expose_class(thread, UI_DEBUG_CONSOLE, Some(crate::bind_ui::UIELEMENT));
    fn console(c: &pz_kahlua::NativeCall<'_>) -> pz_kahlua::LuaResult<Rc<UIElement>> {
        let e = element_of(&c.arg(0))
            .ok_or_else(|| pz_kahlua::LuaError::runtime("Expected UIDebugConsole"))?;
        if e.ext::<UIDebugConsole>().is_none() {
            return Err(pz_kahlua::LuaError::runtime("Expected UIDebugConsole"));
        }
        Ok(e)
    }
    b.method("ProcessCommand", |c| {
        let e = console(c)?;
        e.ext::<UIDebugConsole>()
            .unwrap()
            .process_command(&e, c.thread);
        Ok(0)
    });
    b.method("onOtherKey", |c| {
        let e = console(c)?;
        e.ext::<UIDebugConsole>()
            .unwrap()
            .on_other_key(c.thread, int_arg(c, 1));
        Ok(0)
    });
    b.method("levenshteinDistance", |c| {
        console(c)?;
        let s = |v: Value| pz_kahlua::exec::java_value_of(&v);
        let d = levenshtein_distance(&s(c.arg(1)), &s(c.arg(2)));
        Ok(c.push(d as f64))
    });
    b.method("setMovable", |c| {
        let e = console(c)?;
        e.ext::<UIDebugConsole>()
            .unwrap()
            .core
            .movable
            .set(crate::expose::bool_arg(c, 1));
        Ok(0)
    });
    b.method("Nest", |c| {
        let e = console(c)?;
        let child = element_of(&c.arg(1))
            .ok_or_else(|| pz_kahlua::LuaError::runtime("Expected UIElement"))?;
        let (tp, r, bt, l) = (int_arg(c, 2), int_arg(c, 3), int_arg(c, 4), int_arg(c, 5));
        e.ext::<UIDebugConsole>()
            .unwrap()
            .core
            .nest(&e, &child, c.thread, tp, r, bt, l);
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::super::host::{install, HudConsoleMethods, HudHost};
    use super::super::test_util::*;
    use super::*;
    use pz_ui::{TextAlign, UiDrawCmd};
    use std::collections::HashMap;

    /// The oracle's monospace `TextManager`: 8 px a character, 16 px lines
    /// (24 Large, 20 Medium); draws are recorded.
    struct Mono;
    impl pz_ui::TextRenderer for Mono {
        fn measure_x(&self, _: Option<UIFont>, s: &str) -> i32 {
            8 * s.encode_utf16().count() as i32
        }
        fn measure_y(&self, f: Option<UIFont>, s: &str) -> i32 {
            (1 + s.matches('\n').count() as i32) * self.font_height(f)
        }
        fn font_height(&self, f: Option<UIFont>) -> i32 {
            match f {
                Some(UIFont::Large) => 24,
                Some(UIFont::Medium) => 20,
                _ => 16,
            }
        }
        #[allow(clippy::too_many_arguments)]
        fn draw(
            &self,
            out: &mut pz_ui::SpriteRenderer,
            font: Option<UIFont>,
            x: f64,
            y: f64,
            zoom: f32,
            text: &str,
            color: Rgba,
            align: TextAlign,
            trimmed: bool,
        ) {
            out.push(UiDrawCmd::Text {
                font,
                x: x as f32,
                y: y as f32,
                zoom,
                text: Rc::from(text),
                color,
                align,
                trimmed,
            });
        }
    }

    /// `gm` / `pm` lines: GlobalObject's methods and the public methods of
    /// a few return types, as that run listed them.
    struct Methods {
        global: Vec<ConsoleMethod>,
        of: HashMap<String, Vec<ConsoleMethod>>,
    }
    impl HudConsoleMethods for Methods {
        fn global_methods(&self) -> Vec<ConsoleMethod> {
            self.global.clone()
        }
        fn public_methods_of(&self, rt: &str) -> Vec<ConsoleMethod> {
            self.of
                .get(&rt.replace(' ', "_"))
                .cloned()
                .unwrap_or_default()
        }
    }
    fn method(name: &str, n: &str, rt: &str) -> ConsoleMethod {
        ConsoleMethod {
            name: name.into(),
            return_type: rt.into(),
            param_count: n.parse().unwrap(),
        }
    }
    fn methods(text: &str) -> Methods {
        let mut m = Methods {
            global: Vec::new(),
            of: HashMap::new(),
        };
        for l in text.lines() {
            if let Some(r) = l.strip_prefix("gm ") {
                let p: Vec<&str> = r.splitn(3, ' ').collect();
                m.global.push(method(p[0], p[1], p[2]));
            } else if let Some(r) = l.strip_prefix("pm ") {
                let p: Vec<&str> = r.splitn(4, ' ').collect();
                m.of.entry(p[0].to_string())
                    .or_default()
                    .push(method(p[1], p[2], p[3]));
            }
        }
        m
    }

    fn lines() -> Vec<String> {
        with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Quad {
                        texture,
                        x,
                        y,
                        w,
                        h,
                        color,
                        ..
                    } => Some(format!(
                        "draw {} {} {} {} {} {} {} {} {}",
                        texture
                            .as_ref()
                            .map_or("-".to_string(), |t| t.name.to_string()),
                        *x as i32,
                        *y as i32,
                        *w as i32,
                        *h as i32,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3)
                    )),
                    UiDrawCmd::Text {
                        font,
                        x,
                        y,
                        text,
                        color,
                        align,
                        trimmed,
                        ..
                    } => Some(format!(
                        "text {:?} {} {} {} [{}] {} {} {} {}",
                        font.unwrap(),
                        match (align, trimmed) {
                            (_, false) => "untrimmed",
                            (TextAlign::Left, _) => "left",
                            (TextAlign::Centre, _) => "centre",
                            (TextAlign::Right, _) => "right",
                        },
                        jf(*x),
                        jf(*y),
                        text,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3)
                    )),
                    _ => None,
                })
                .collect()
        })
    }

    fn st(e: &UIElement, l: &str) -> String {
        format!(
            "{l} {} {} {} {} {}",
            jf(e.get_x() as f32),
            jf(e.get_y() as f32),
            jf(e.get_width() as f32),
            jf(e.get_height() as f32),
            e.is_visible()
        )
    }
    fn children(e: &UIElement, got: &mut Vec<String>) {
        for i in 0.. {
            let Some(c) = e.control(i) else { break };
            let n = pz_kahlua::LuaObject::class_name(&*c).to_string();
            got.push(st(
                &c,
                &format!("child {}", n.rsplit(['.', '$']).next().unwrap()),
            ));
        }
    }
    fn sbst(e: &UIElement, l: &str) -> String {
        let s = sb(e);
        format!(
            "{l} off {} len {} drag {} h {}",
            jf(s.button_offset()),
            jf(s.button_inside_length()),
            s.is_being_dragged(),
            jf(e.get_height() as f32)
        )
    }
    fn tbst(e: &UIElement, l: &str) -> String {
        let s = tb(e).state();
        let text: String = s.text.iter().collect();
        format!(
            "{l} top {} vis {} lines {} text [{}]",
            s.top_line_index,
            s.num_visible_lines,
            s.lines.len(),
            text.replace('\n', "|")
        )
    }
    fn text_of(e: &UIElement) -> String {
        tb(e).state().text.iter().collect()
    }
    fn mouse(x: i32, y: i32) {
        with_ui(|m| {
            m.mouse.x = x;
            m.mouse.y = y;
        });
    }
    fn flush(got: &mut Vec<String>, label: &str) {
        got.push(format!("case {label}"));
        got.extend(lines());
        with_ui(|m| m.sprite.clear());
    }

    /// Parity with the original `UIDebugConsole` and `ScrollBar` (oracle
    /// `console.txt`): layout, render (window, text boxes with their
    /// stencils, scroll bar, labels), spooled lines and the scroll bar's
    /// size, a press beside the button and a drag, resizing from the
    /// corner (whole lines, the edge highlights), hover, `clear` and the
    /// history keys, method suggestions on the real `GlobalObject` list
    /// (chained through return types), Tab completion, the distance.
    #[test]
    fn matches_console_oracle() {
        let Some(text) = oracle("console.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install_textures_ex(&oracle_textures_ex(&text));
        install(HudHost {
            console_methods: Box::new(methods(&text)),
            ..HudHost::default()
        });
        let mut t = thread();
        pz_ui::UIManager::set_text_renderer(Some(Rc::new(Mono)));
        crate::bind_core::shared_core()
            .borrow_mut()
            .set_screen_size(1280, 720);
        with_ui(|m| m.sprite.clear());
        let mut got = vec![format!(
            "version {}",
            crate::bind_core::shared_core().borrow().get_version()
        )];

        let c = UIDebugConsole::create(&mut t, 20, 720 - 265);
        with_ui(|m| m.debug_console = Some(c.clone()));
        c.set_y(720.0 - c.get_height() - 20.0);
        got.push(st(&c, "console"));
        children(&c, &mut got);
        let me = c.ext::<UIDebugConsole>().unwrap();
        let (log, bar, auto, cl) = (
            me.output_log.clone(),
            me.scroll_bar_v.clone(),
            me.autosuggest.clone(),
            me.command_line.clone(),
        );
        mouse(0, 0);
        c.update(&mut t);
        got.push(st(&c, "update"));
        got.push(tbst(&log, "log"));
        got.push(sbst(&bar, "sb"));
        with_ui(|m| m.sprite.clear());
        c.render(&mut t);
        flush(&mut got, "render");
        for i in 0..14 {
            me.spool_text(&mut t, &format!("line {i}"));
        }
        got.push(tbst(&log, "spooled"));
        got.push(sbst(&bar, "sb spooled"));
        c.update(&mut t);
        got.push(tbst(&log, "spool update"));
        got.push(sbst(&bar, "sb spool update"));
        c.render(&mut t);
        flush(&mut got, "render spooled");
        let (sx, sy) = (bar.get_absolute_x(), bar.get_absolute_y());
        bar.on_mouse_down(&mut t, 3.0, 10.0);
        got.push(sbst(&bar, "sb press"));
        c.update(&mut t);
        got.push(tbst(&log, "log press"));
        mouse(sx as i32 + 3, sy as i32 + 60);
        bar.on_mouse_down(&mut t, 3.0, 60.0);
        got.push(sbst(&bar, "sb grab"));
        mouse(sx as i32 + 3, sy as i32 + 40);
        c.update(&mut t);
        got.push(sbst(&bar, "sb dragged"));
        got.push(tbst(&log, "log dragged"));
        bar.on_mouse_up(&mut t, 3.0, 40.0);
        let (ax, ay) = (c.get_absolute_x() as i32, c.get_absolute_y() as i32);
        let (w, h) = (c.get_width() as i32, c.get_height() as i32);
        mouse(ax + w - 3, ay + h - 3);
        got.push(format!(
            "down {}",
            c.on_mouse_down(&mut t, (w - 3) as f64, (h - 3) as f64)
        ));
        c.render(&mut t);
        flush(&mut got, "render resizing");
        mouse(ax + w + 77, ay + h - 50);
        got.push(format!("move {}", c.on_mouse_move(&mut t, 80.0, -47.0)));
        got.push(st(&c, "resized"));
        children(&c, &mut got);
        got.push(sbst(&bar, "sb resized"));
        got.push(tbst(&log, "log resized"));
        c.on_mouse_up_outside(&mut t, 0.0, 0.0);
        mouse(ax + w + 75, ay + 60);
        c.render(&mut t);
        flush(&mut got, "render hover edge");
        mouse(ax + 100, ay + 100);
        c.render(&mut t);
        flush(&mut got, "render hover");
        tb(&cl).state().internal = "clear".chars().collect();
        me.process_command(&c, &mut t);
        got.push(tbst(&log, "cleared"));
        got.push(format!("prevIndex {}", me.previous_index.get()));
        tb(&cl).on_press_up(&cl, &mut t);
        got.push(format!("up [{}] {}", text_of(&cl), me.previous_index.get()));
        tb(&cl).on_press_down(&cl, &mut t);
        got.push(format!(
            "down [{}] {}",
            text_of(&cl),
            me.previous_index.get()
        ));
        for input in [
            "getPla",
            "getPlayer():getInv",
            "getPlayer():getInventory():getIt",
            "getCore():getScreenW",
            "isServ",
            ":",
            "getCore():",
            "x",
        ] {
            tb(&cl).set_text(&cl, &mut t, &input.chars().collect::<Vec<_>>());
            c.update(&mut t);
            got.push(format!(
                "suggest [{input}] visible {} [{}] {} {}",
                auto.is_visible(),
                text_of(&auto),
                jf(auto.get_x() as f32),
                jf(auto.get_width() as f32)
            ));
        }
        tb(&cl).set_text(&cl, &mut t, &"getPla".chars().collect::<Vec<_>>());
        c.update(&mut t);
        me.on_other_key(&mut t, 15);
        got.push(format!("tab [{}]", text_of(&cl)));
        tb(&cl).set_text(&cl, &mut t, &[]);
        c.update(&mut t);
        got.push(format!("empty visible {}", auto.is_visible()));
        got.push(format!(
            "lev {} {} {}",
            levenshtein_distance("kitten", "sitting"),
            levenshtein_distance("", "abc"),
            levenshtein_distance("abc", "")
        ));

        let want: Vec<String> = text
            .lines()
            .filter(|l| {
                !["tex ", "fake ", "gm ", "pm "]
                    .iter()
                    .any(|p| l.starts_with(p))
            })
            .map(normalise_oracle)
            .collect();
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "line {i}");
        }
        assert_eq!(got.len(), want.len());
    }

    /// The captured debug log: a trailing newline flushes it into the
    /// output log on the next update; an overflowing write drops the
    /// buffer; `print` reaches the log when `Core.debug`.
    #[test]
    fn debug_log_and_print() {
        install_textures(&[]);
        install(HudHost::default());
        let mut t = thread();
        pz_ui::UIManager::set_text_renderer(Some(Rc::new(Mono)));
        crate::bind_core::shared_core().borrow_mut().debug = true;
        let c = UIDebugConsole::create(&mut t, 0, 0);
        crate::bind_core::shared_core().borrow_mut().debug = false;
        let me = c.ext::<UIDebugConsole>().unwrap();
        tb(&me.output_log).set_text(&me.output_log, &mut t, &[]);
        add_output(b"partial");
        c.update(&mut t);
        assert_eq!(text_of(&me.output_log), "");
        add_output(b" line\n");
        c.update(&mut t);
        assert_eq!(text_of(&me.output_log), "partial line\n");
        add_output(&[b'x'; 8000]);
        add_output(b"y\n");
        c.update(&mut t);
        assert_eq!(
            text_of(&me.output_log),
            format!("partial line\n{}y\n", "x".repeat(8000))
        );
        run(&mut t, "print('hello', 1)");
        assert!(text_of(&me.output_log).ends_with("hello\t1\n"));
        pz_kahlua::stdlib::set_print_callback(None);
        // A compile error is spooled; `clear` empties the log.
        tb(&me.command_line).state().internal = "x = (".chars().collect();
        me.process_command(&c, &mut t);
        assert!(!text_of(&me.output_log).ends_with("hello\t1\n"));
        tb(&me.command_line).state().internal = "y = 42".chars().collect();
        me.process_command(&c, &mut t);
        assert_eq!(eval(&mut t, "y").as_str(), None);
        assert!(matches!(eval(&mut t, "y"), Value::Num(n) if n == 42.0));
        tb(&me.command_line).state().internal = "  clear ".chars().collect();
        me.process_command(&c, &mut t);
        assert_eq!(text_of(&me.output_log), "");
    }

    #[test]
    fn java_split_drops_trailing_empties() {
        assert_eq!(java_split("a:b:", ':'), vec!["a", "b"]);
        assert_eq!(java_split(":", ':'), Vec::<String>::new());
        assert_eq!(java_split(":a", ':'), vec!["", "a"]);
        assert_eq!(java_split("", ':'), vec![""]);
    }
}
