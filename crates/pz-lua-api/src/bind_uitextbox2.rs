//! `zombie.ui.UITextBox2` (the text entry behind `ISTextEntryBox`), its
//! frame `zombie.ui.UINineGrid`, `Core.currentTextEntryBox` and the text
//! entry half of `Core.updateKeyboard` — `UIElement` subclasses on the
//! pz-ui extension slot.
//!
//! Text is kept as `char`s and indexed like the original's `String`s
//! (identical for text in the Basic Multilingual Plane). Lua callbacks
//! (`onTextChange`, `onCommandEntered`, ...) run with no borrow of the box
//! held. `Core.currentTextEntryBox` is mirrored into
//! [`pz_core::core::TextEntryInfo`] for the code that reads it from
//! `Core` (`isDoingTextEntry`, `GameKeyboard`).

use crate::bind_core::{color_arg, shared_core};
use crate::bind_misc::{UITransition, CLIPBOARD};
use crate::bind_ui::{element_of, ui_font_from_name, UIELEMENT, UIFONT};
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, JavaEnum};
use pz_core::core::TextEntryInfo;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_render::{Rgba, Texture};
use pz_text::UIFont;
use pz_ui::draw::{draw_string, font_height, font_width_range, measure_string_x, measure_string_y};
use pz_ui::{with_ui, ColorObject, TextAlign, UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const UITEXTBOX2: &str = "zombie.ui.UITextBox2";
pub const UININEGRID: &str = "zombie.ui.UINineGrid";
const TEXTBOX_SUPERS: &[&str] = &[UIELEMENT, "zombie.ui.UITextEntryInterface"];
const NINEGRID_SUPERS: &[&str] = &[UIELEMENT];

/// `new Color(r, g, b, a)` with 0..255 components.
fn color(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    )
}

// ---- UINineGrid ----------------------------------------------------------------

const BOX_TEXTURES: [&str; 9] = [
    "media/ui/Box_TopLeft.png",
    "media/ui/Box_Top.png",
    "media/ui/Box_TopRight.png",
    "media/ui/Box_Left.png",
    "media/ui/Box_Center.png",
    "media/ui/Box_Right.png",
    "media/ui/Box_BottomLeft.png",
    "media/ui/Box_Bottom.png",
    "media/ui/Box_BottomRight.png",
];

/// `zombie.ui.UINineGrid`.
pub struct UINineGrid {
    /// TL, T, TR, L, C, R, BL, B, BR.
    grid: [Option<Texture>; 9],
    top_width: i32,
    left_width: i32,
    right_width: i32,
    bottom_width: i32,
    pub client_w: Cell<i32>,
    pub client_h: Cell<i32>,
    /// `nestedItems` (`Rectangle(l, t, r, b)` as x, y, width, height).
    nested: RefCell<Vec<[i32; 4]>>,
    pub color: Cell<Rgba>,
}

impl UINineGrid {
    /// `new UINineGrid(x, y, w, h, top, left, right, bottom, textures...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        top: i32,
        left: i32,
        right: i32,
        bottom: i32,
        tex: [&str; 9],
    ) -> Rc<UIElement> {
        let g = UINineGrid {
            grid: tex.map(pz_render::get_shared_texture),
            top_width: top,
            left_width: left,
            right_width: right,
            bottom_width: bottom,
            client_w: Cell::new(w),
            client_h: Cell::new(h),
            nested: RefCell::default(),
            color: Cell::new(color(50, 50, 50, 212)),
        };
        let e = UIElement::new_ext(None, Box::new(g));
        {
            let mut s = e.st_mut();
            s.x = x as f64;
            s.y = y as f64;
            s.width = w as f32;
            s.height = h as f32;
        }
        e
    }

    /// `Nest(el, t, r, b, l)`.
    #[allow(clippy::too_many_arguments)]
    pub fn nest(
        &self,
        me: &UIElement,
        el: &UIElement,
        t: &mut Thread,
        top: i32,
        r: i32,
        b: i32,
        l: i32,
    ) {
        me.add_child(el);
        self.nested.borrow_mut().push([l, top, r, b]);
        el.set_x(l as f64);
        el.set_y(top as f64);
        el.update(t);
    }

    pub fn set_alpha(&self, a: f32) {
        let mut c = self.color.get();
        c.3 = a;
        self.color.set(c);
    }

    pub fn get_alpha(&self) -> f32 {
        self.color.get().3
    }
}

impl UIElementExt for UINineGrid {
    fn class_name(&self) -> &'static str {
        UININEGRID
    }
    fn super_classes(&self) -> &'static [&'static str] {
        NINEGRID_SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn render(&self, el: &UIElement, t: &mut Thread) {
        let c = self.color.get();
        let (w, h) = (el.get_width(), el.get_height());
        let (tw, lw, rw, bw) = (
            self.top_width as f64,
            self.left_width as f64,
            self.right_width as f64,
            self.bottom_width as f64,
        );
        let g = &self.grid;
        el.draw_texture_scaled_col_c(g[0].as_ref(), 0.0, 0.0, lw, tw, c);
        el.draw_texture_scaled_col_c(g[1].as_ref(), lw, 0.0, w - (lw + rw), tw, c);
        el.draw_texture_scaled_col_c(g[2].as_ref(), w - rw, 0.0, rw, tw, c);
        el.draw_texture_scaled_col_c(g[3].as_ref(), 0.0, tw, lw, h - (tw + bw), c);
        el.draw_texture_scaled_col_c(g[4].as_ref(), lw, tw, w - (lw + rw), h - (tw + bw), c);
        el.draw_texture_scaled_col_c(g[5].as_ref(), w - rw, tw, rw, h - (tw + bw), c);
        el.draw_texture_scaled_col_c(g[6].as_ref(), 0.0, h - bw, lw, bw, c);
        el.draw_texture_scaled_col_c(g[7].as_ref(), lw, h - bw, w - (lw + rw), bw, c);
        el.draw_texture_scaled_col_c(g[8].as_ref(), w - rw, h - bw, rw, bw, c);
        el.render_base(t);
    }

    fn update(&self, el: &UIElement, t: &mut Thread) {
        el.update_base(t);
        let nested = self.nested.borrow().clone();
        let (cw, ch) = (self.client_w.get() as f64, self.client_h.get() as f64);
        for (n, r) in nested.iter().enumerate() {
            let Some(con) = el.control(n) else { break };
            let [x, y, w, h] = r.map(|v| v as f64);
            con.set_x(x);
            con.set_y(y);
            con.set_width(((cw - (x + w)) as i32) as f64);
            con.set_height(((ch - (y + h)) as i32) as f64);
            con.onresize();
        }
    }
}

// ---- Core.currentTextEntryBox ------------------------------------------------------

thread_local! {
    /// `Core.currentTextEntryBox`.
    static CURRENT: RefCell<Option<Rc<UIElement>>> = const { RefCell::new(None) };
}

/// `Core.currentTextEntryBox`.
pub fn current_text_entry_box() -> Option<Rc<UIElement>> {
    CURRENT.with(|c| c.borrow().clone())
}

fn set_current(el: Option<Rc<UIElement>>) {
    CURRENT.with(|c| *c.borrow_mut() = el);
    sync_core();
}

fn is_current(el: &UIElement) -> bool {
    CURRENT.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|e| std::ptr::eq(Rc::as_ptr(e), el))
    })
}

/// Mirrors `Core.currentTextEntryBox` into `Core`.
fn sync_core() {
    let info = current_text_entry_box().and_then(|e| {
        let b = e.ext::<UITextBox2>()?;
        let s = b.st.borrow();
        Some(TextEntryInfo {
            ui_name: e.get_ui_name(),
            editable: s.is_editable,
            doing_text_entry: s.doing_text_entry,
        })
    });
    if let Ok(mut c) = shared_core().try_borrow_mut() {
        c.current_text_entry_box = info;
    }
}

/// `Core.UnfocusActiveTextEntryBox()`.
pub fn unfocus_active_text_entry_box(t: &mut Thread) {
    let Some(e) = current_text_entry_box() else {
        return;
    };
    if e.get_ui_name().contains("chat text entry") {
        return;
    }
    if let Some(b) = e.ext::<UITextBox2>() {
        b.set_doing_text_entry(&e, t, false);
        b.reset_frame_colour();
    }
    set_current(None);
}

/// `Core.isDoingTextEntry()`.
pub fn is_doing_text_entry() -> bool {
    current_text_entry_box().is_some_and(|e| {
        e.ext::<UITextBox2>().is_some_and(|b| {
            let s = b.st.borrow();
            s.is_editable && s.doing_text_entry
        })
    })
}

/// `Core.isFunctionKey(key)`.
fn is_function_key(key: i32) -> bool {
    (59..=68).contains(&key) || (87..=105).contains(&key) || key == 113
}

/// `Core.updateKeyboard()` (called from `Core.DoFrameReady`): the queued key
/// and character events go to the focused text entry.
pub fn update_keyboard(t: &mut Thread) {
    if !is_doing_text_entry() {
        return;
    }
    while pz_input::with_input(|i| i.key_events.next()) {
        if !is_doing_text_entry() {
            continue;
        }
        let (key, state, ch) = pz_input::with_input(|i| {
            (
                i.key_events.event_key(),
                i.key_events.event_key_state(),
                i.key_events.event_character(),
            )
        });
        if !state {
            continue;
        }
        let Some(e) = current_text_entry_box() else {
            continue;
        };
        update_keyboard_aux(&e, t, key, ch);
    }
    if let Some(e) = current_text_entry_box() {
        if let Some(b) = e.ext::<UITextBox2>() {
            let mut s = b.st.borrow_mut();
            if s.ignore_first {
                s.ignore_first = false;
            }
        }
        sync_core();
    }
}

/// `Core.updateKeyboardAux(entry, eventKey)`.
fn update_keyboard_aux(e: &UIElement, t: &mut Thread, key: i32, ch: char) {
    let Some(b) = e.ext::<UITextBox2>() else {
        return;
    };
    let ctrl = crate::bind_input::is_key_down_raw(29) || crate::bind_input::is_key_down_raw(157);
    match key {
        28 | 156 => return b.on_key_enter(e, t),
        1 => {
            b.on_other_key(e, t, 1);
            crate::bind_input::eat_key_press(1);
            return;
        }
        15 => {
            b.on_other_key(e, t, 15);
            pz_ui::UIManager::trigger(t, "SwitchChatStream", &[]);
            return;
        }
        58 | 29 | 157 | 42 | 54 | 56 | 184 => return,
        199 => return b.on_key_home(),
        207 => return b.on_key_end(),
        200 => return b.on_key_up(e, t),
        208 => return b.on_key_down(e, t),
        203 => return b.on_key_left(),
        205 => return b.on_key_right(),
        _ => {}
    }
    if is_function_key(key) {
        return;
    }
    match key {
        211 => return b.on_key_delete(e, t),
        14 => return b.on_key_back(e, t),
        47 if ctrl => return b.paste_from_clipboard(e, t),
        46 if ctrl => return b.copy_to_clipboard(),
        45 if ctrl => return b.cut_to_clipboard(),
        30 if ctrl => return b.select_all(),
        _ => {}
    }
    if b.st.borrow().ignore_first || b.is_text_limit() {
        return;
    }
    if ch == '\0' {
        return;
    }
    let (only_numbers, only_text) = {
        let s = b.st.borrow();
        (s.only_numbers, s.only_text)
    };
    // Double.parseDouble of one character: only digits parse.
    if only_numbers && ch != '.' && ch != '-' && !ch.is_ascii_digit() {
        return;
    }
    if only_text && !ch.is_ascii_alphabetic() {
        return;
    }
    b.put_character(e, t, ch);
}

// ---- UITextBox2 ---------------------------------------------------------------

fn jstr(v: &[char]) -> String {
    v.iter().collect()
}

fn chars(v: &str) -> Vec<char> {
    v.chars().collect()
}

/// `UITextBox2`'s fields.
struct TextBoxState {
    lines: Vec<Vec<char>>,
    frame: Option<Rc<UIElement>>,
    text: Vec<char>,
    center_vertically: bool,
    doing_text_entry: bool,
    cursor: i32,
    text_entry_max_length: i32,
    is_editable: bool,
    is_selectable: bool,
    cursor_line: i32,
    multiple_line: bool,
    line_start: Vec<i32>,
    to_selection: i32,
    internal: Vec<char>,
    mask: bool,
    ignore_first: bool,
    font: Option<UIFont>,
    highlight_lines: Vec<i32>,
    has_frame: bool,
    num_visible_lines: i32,
    top_line_index: i32,
    blink_frame: f32,
    blink_state: bool,
    text_color: Rgba,
    selecting_range: bool,
    max_text_length: i32,
    force_upper_case: bool,
    x_offset: i32,
    max_lines: i32,
    only_numbers: bool,
    only_text: bool,
    clear_button_texture: Option<Texture>,
    clear_button_size: i32,
    clear_button: bool,
    clear_button_transition: Option<Rc<UITransition>>,
    wrap_lines: bool,
    placeholder: Option<String>,
    placeholder_color: Rgba,
    always_paginate: bool,
    text_changed: bool,
    paginate_width: i32,
    paginate_font: Option<Option<UIFont>>,
    /// `now_ms` of the last `update` (for the real-world seconds).
    last_update_ms: Option<i64>,
}

const STANDARD_FRAME: (u8, u8, u8, u8) = (50, 50, 50, 212);
const TEXT_ENTRY_FRAME: (u8, u8, u8, u8) = (50, 50, 127, 212);

fn c4(c: (u8, u8, u8, u8)) -> Rgba {
    color(c.0, c.1, c.2, c.3)
}

/// The `UITextBox2` part of the element.
pub struct UITextBox2 {
    st: RefCell<TextBoxState>,
}

fn line_height(font: Option<UIFont>) -> i32 {
    // `getFontFromEnum(font).getLineHeight()` == `getFontHeight(font)`.
    font_height(font)
}

impl TextBoxState {
    fn inset(&self) -> i32 {
        if self.has_frame {
            5
        } else {
            2
        }
    }

    fn line(&self, i: i32) -> &[char] {
        self.lines
            .get(i.max(0) as usize)
            .map_or(&[], |l| l.as_slice())
    }

    fn start(&self, i: i32) -> i32 {
        self.line_start.get(i.max(0) as usize).copied().unwrap_or(0)
    }

    fn masked(&self) -> Vec<char> {
        vec!['*'; self.internal.len()]
    }

    /// `Paginate()`.
    fn paginate(&mut self, width: f64) {
        let mut paginate = self.always_paginate;
        if !self.always_paginate {
            if self.paginate_font != Some(self.font) {
                self.paginate_font = Some(self.font);
                paginate = true;
            }
            if self.paginate_width != width as i32 {
                self.paginate_width = width as i32;
                paginate = true;
            }
            if self.text_changed {
                self.text_changed = false;
                paginate = true;
            }
            if !paginate {
                return;
            }
        }
        self.lines.clear();
        self.line_start.clear();
        if self.text.is_empty() {
            return;
        }
        if !self.multiple_line {
            self.lines.push(self.text.clone());
            self.line_start.push(0);
            return;
        }
        let inset = self.inset() as f64;
        let mut offset = 0i32;
        let all = self.text.clone();
        for seg in all.split(|&c| c == '\n') {
            let mut text: Vec<char> = seg.to_vec();
            let mut n: i32 = 0;
            if text.is_empty() {
                self.lines.push(Vec::new());
                self.line_start.push(offset);
                offset += 1;
                continue;
            }
            loop {
                // `text.indexOf(" ", n + 1)`.
                let from = (n + 1).max(0) as usize;
                let mut m: i32 = text
                    .iter()
                    .skip(from)
                    .position(|&c| c == ' ')
                    .map_or(-1, |p| (p + from) as i32);
                let z = if m == -1 { text.len() as i32 } else { m };
                let wid = measure_string_x(self.font, &jstr(&text[..z as usize]));
                if self.wrap_lines && wid as f64 >= width - inset * 2.0 - 17.0 && n > 0 {
                    let sub = text[..n as usize].to_vec();
                    text = text[(n + 1) as usize..].to_vec();
                    offset += sub.len() as i32 + 1;
                    self.lines.push(sub);
                    self.line_start
                        .push(offset - (self.lines.last().unwrap().len() as i32 + 1));
                    m = 0;
                } else if m == -1 {
                    self.line_start.push(offset);
                    offset += text.len() as i32 + 1;
                    self.lines.push(text);
                    break;
                }
                n = m;
                if text.is_empty() {
                    break;
                }
            }
        }
    }

    /// `toDisplayLine(textOffset)`.
    fn to_display_line(&self, off: i32) -> i32 {
        for i in 0..self.lines.len() {
            let st = self.line_start[i];
            if off < st || off > st + self.lines[i].len() as i32 {
                continue;
            }
            return i as i32;
        }
        0
    }

    /// `getCursorPosFromX(x)`.
    fn cursor_pos_from_x(&self, x: i32) -> i32 {
        if self.lines.is_empty() {
            return 0;
        }
        let text = self.line(self.cursor_line);
        let start = self.start(self.cursor_line);
        if text.is_empty() || x + self.x_offset < 0 {
            return start;
        }
        for n in 0..=text.len() {
            let wid = measure_string_x(self.font, &jstr(&text[..n]));
            if wid <= x + self.x_offset {
                continue;
            }
            return start + n as i32 - 1;
        }
        start + text.len() as i32
    }

    fn reset_blink(&mut self) {
        self.blink_state = true;
        self.blink_frame = 12.0;
    }

    fn sel_range(&self) -> (i32, i32) {
        (
            self.cursor.min(self.to_selection),
            self.cursor.max(self.to_selection),
        )
    }

    /// `internalText.substring(a, b)` with Java's bounds as a clamp.
    fn internal_sub(&self, a: i32, b: i32) -> Vec<char> {
        let n = self.internal.len() as i32;
        let (a, b) = (a.clamp(0, n), b.clamp(0, n));
        if a >= b {
            Vec::new()
        } else {
            self.internal[a as usize..b as usize].to_vec()
        }
    }
}

impl UITextBox2 {
    /// `new UITextBox2(font, x, y, width, height, text, hasFrame)`.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        t: &mut Thread,
        font: Option<UIFont>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        text: &str,
        has_frame: bool,
    ) -> Rc<UIElement> {
        let st = TextBoxState {
            lines: Vec::new(),
            frame: None,
            text: Vec::new(),
            center_vertically: false,
            doing_text_entry: false,
            cursor: 0,
            text_entry_max_length: 2000,
            is_editable: false,
            is_selectable: false,
            cursor_line: 0,
            multiple_line: false,
            line_start: Vec::new(),
            to_selection: 0,
            internal: Vec::new(),
            mask: false,
            ignore_first: false,
            font,
            highlight_lines: vec![0; 1000],
            has_frame: false,
            num_visible_lines: 0,
            top_line_index: 0,
            blink_frame: 12.0,
            blink_state: true,
            text_color: Rgba(1.0, 1.0, 1.0, 1.0),
            selecting_range: false,
            max_text_length: -1,
            force_upper_case: false,
            x_offset: 0,
            max_lines: 1,
            only_numbers: false,
            only_text: false,
            clear_button_texture: None,
            clear_button_size: 0,
            clear_button: false,
            clear_button_transition: None,
            wrap_lines: true,
            placeholder: None,
            placeholder_color: Rgba(0.5, 0.5, 0.5, 1.0),
            always_paginate: true,
            text_changed: false,
            paginate_width: -1,
            paginate_font: None,
            last_update_ms: None,
        };
        let el = UIElement::new_ext(
            None,
            Box::new(UITextBox2 {
                st: RefCell::new(st),
            }),
        );
        let b = el.ext::<UITextBox2>().unwrap();
        {
            let mut s = el.st_mut();
            s.x = x as f64;
            s.y = y as f64;
        }
        b.set_text(&el, t, &chars(text));
        {
            let mut s = el.st_mut();
            s.width = width as f32;
            s.height = height as f32;
        }
        {
            let mut s = b.st.borrow_mut();
            s.num_visible_lines = 10;
            s.top_line_index = 0;
        }
        set_current(Some(el.clone()));
        {
            let mut s = b.st.borrow_mut();
            s.highlight_lines.iter_mut().for_each(|h| *h = 0);
            s.has_frame = has_frame;
        }
        if has_frame {
            let f = UINineGrid::create(0, 0, width, height, 5, 5, 5, 5, BOX_TEXTURES);
            b.st.borrow_mut().frame = Some(f.clone());
            el.add_child(&f);
        }
        {
            let w = el.get_width();
            let mut s = b.st.borrow_mut();
            s.paginate(w);
            s.doing_text_entry = false;
            s.text_entry_max_length = 2000;
            s.cursor = 0;
            s.to_selection = 0;
            s.is_editable = false;
            s.clear_button_texture =
                pz_render::get_shared_texture("media/ui/inventoryPanes/Button_Close.png");
            s.clear_button_size = font_height(s.font) - 4;
        }
        pz_input::with_input(|i| i.repeat_events = true);
        sync_core();
        el
    }

    fn frame_ext(&self) -> Option<(Rc<UIElement>, ())> {
        self.st.borrow().frame.clone().map(|f| (f, ()))
    }

    fn set_frame_colour(&self, c: Rgba) {
        if let Some((f, _)) = self.frame_ext() {
            if let Some(g) = f.ext::<UINineGrid>() {
                g.color.set(c);
            }
        }
    }

    fn reset_frame_colour(&self) {
        self.set_frame_colour(c4(STANDARD_FRAME));
    }

    /// `pcall(tableget(table, name), table, args...)` when it exists.
    fn call_lua(el: &UIElement, t: &mut Thread, name: &str, args: &[Value]) {
        let Some(table) = el.get_table() else { return };
        let Some(f) = pz_ui::lua::tableget(t, &table, name) else {
            return;
        };
        let mut a = Vec::with_capacity(args.len() + 1);
        a.push(Value::Table(table));
        a.extend_from_slice(args);
        pz_ui::lua::pcall_void(t, &f, &a);
    }

    pub fn on_text_change(&self, el: &UIElement, t: &mut Thread) {
        Self::call_lua(el, t, "onTextChange", &[]);
    }
    pub fn on_command_entered(&self, el: &UIElement, t: &mut Thread) {
        Self::call_lua(el, t, "onCommandEntered", &[]);
    }
    pub fn on_press_up(&self, el: &UIElement, t: &mut Thread) {
        Self::call_lua(el, t, "onPressUp", &[]);
    }
    pub fn on_press_down(&self, el: &UIElement, t: &mut Thread) {
        Self::call_lua(el, t, "onPressDown", &[]);
    }
    pub fn on_other_key(&self, el: &UIElement, t: &mut Thread, key: i32) {
        Self::call_lua(el, t, "onOtherKey", &[Value::Num(key as f64)]);
    }
    pub fn on_lost_focus(&self, el: &UIElement, t: &mut Thread) {
        Self::call_lua(el, t, "onLostFocus", &[]);
    }

    /// `SetText(text)`.
    pub fn set_text(&self, el: &UIElement, t: &mut Thread, text: &[char]) {
        {
            let mut s = self.st.borrow_mut();
            s.internal = text.to_vec();
            s.text = if s.mask { s.masked() } else { text.to_vec() };
            let n = s.text.len() as i32;
            s.cursor = n;
            s.to_selection = n;
        }
        self.update_box(el, t);
        let mut s = self.st.borrow_mut();
        s.to_selection = 0;
        s.cursor = 0;
        if !s.lines.is_empty() {
            let n = s.lines.len() - 1;
            let p = s.line_start[n] + s.lines[n].len() as i32;
            s.cursor = p;
            s.to_selection = p;
        }
    }

    /// `updateText()`.
    pub fn update_text(&self) {
        let mut s = self.st.borrow_mut();
        s.text = if s.mask {
            s.masked()
        } else {
            s.internal.clone()
        };
    }

    /// `clearInput()`.
    pub fn clear_input(&self, el: &UIElement, t: &mut Thread) {
        {
            let mut s = self.st.borrow_mut();
            s.text.clear();
            s.internal.clear();
            s.cursor = 0;
            s.to_selection = 0;
        }
        self.update_box(el, t);
        self.on_text_change(el, t);
    }

    /// `focus()`.
    pub fn focus(&self, el: &UIElement) {
        self.st.borrow_mut().doing_text_entry = true;
        set_current(Some(el.rc()));
    }

    /// `unfocus()`.
    pub fn unfocus(&self, el: &UIElement, t: &mut Thread) {
        self.st.borrow_mut().doing_text_entry = false;
        if is_current(el) {
            set_current(None);
        }
        self.on_lost_focus(el, t);
    }

    /// `setDoingTextEntry(value)`.
    pub fn set_doing_text_entry(&self, el: &UIElement, t: &mut Thread, v: bool) {
        self.st.borrow_mut().doing_text_entry = v;
        sync_core();
        if !v {
            self.on_lost_focus(el, t);
        }
    }

    /// `update()` (no `super.update()`).
    fn update_box(&self, el: &UIElement, t: &mut Thread) {
        let now = pz_ui::manager::now_ms();
        let (lh, inset, frame) = {
            let mut s = self.st.borrow_mut();
            if s.max_text_length > -1 && s.internal.len() as i32 > s.max_text_length {
                let m = s.max_text_length as usize;
                s.internal.truncate(m);
            }
            if s.force_upper_case {
                s.internal = s.internal.iter().flat_map(|c| c.to_uppercase()).collect();
            }
            if s.mask {
                if s.internal.len() != s.text.len() {
                    let text = s.masked();
                    if s.doing_text_entry && s.text != text {
                        s.reset_blink();
                    }
                    s.text = text;
                }
            } else {
                if s.doing_text_entry && s.text != s.internal {
                    s.reset_blink();
                }
                s.text = s.internal.clone();
            }
            s.paginate(el.get_width());
            (line_height(s.font), s.inset(), s.frame.clone())
        };
        if (lh + inset * 2) as f64 > el.get_height() {
            el.set_height((lh + inset * 2) as f64);
        }
        if let Some(f) = frame {
            f.set_height(el.get_height());
        }
        let lh_div = lh.max(1);
        let y_scroll = {
            let mut s = self.st.borrow_mut();
            s.num_visible_lines = (el.get_height() - (inset * 2) as f64) as i32 / lh_div;
            // GameTime.getRealworldSecondsSinceLastUpdate() * 30.
            let secs = match s.last_update_ms {
                Some(p) => ((now - p).max(0) as f32 / 1000.0).min(1.0),
                None => 1.0 / 60.0,
            };
            s.last_update_ms = Some(now);
            if s.blink_frame > 0.0 {
                s.blink_frame -= secs * 30.0;
            } else {
                s.blink_state = !s.blink_state;
                s.blink_frame = if s.blink_state { 12.0 } else { 8.0 };
            }
            let lines = s.lines.len() as i32;
            if s.num_visible_lines * lh + inset * 2 < el.get_height() as i32 {
                if s.num_visible_lines < lines {
                    el.set_scroll_height(((lines + 1) * lh) as f64);
                }
                s.num_visible_lines += 1;
            } else {
                el.set_scroll_height((lines * lh) as f64);
            }
            // UIDebugConsole.instance is never set.
            s.top_line_index = ((-el.get_y_scroll() + inset as f64) as i32) / lh_div;
            (-s.top_line_index * lh) as f64
        };
        el.set_y_scroll(t, y_scroll);
    }

    /// `keepCursorVisible()`.
    fn keep_cursor_visible(s: &mut TextBoxState, width: f64) {
        if s.lines.is_empty() || !s.doing_text_entry || s.multiple_line {
            s.x_offset = 0;
            return;
        }
        if s.cursor > s.text.len() as i32 {
            s.cursor = s.text.len() as i32;
        }
        let text = s.lines[0].clone();
        let text_wid = measure_string_x(s.font, &jstr(&text));
        let inset = s.inset();
        let mut display_wid = width as i32 - inset * 2;
        if s.clear_button && s.clear_button_texture.is_some() {
            display_wid -= 2 + s.clear_button_size + 2;
        }
        if text_wid <= display_wid {
            s.x_offset = 0;
        } else if -s.x_offset + text_wid < display_wid {
            s.x_offset = text_wid - display_wid;
        }
        let cur = (s.cursor.max(0) as usize).min(text.len());
        let before = measure_string_x(s.font, &jstr(&text[..cur]));
        let mut cursor_x = -s.x_offset + inset + before - 1;
        if cursor_x < inset {
            s.x_offset = before;
        } else if cursor_x >= inset + display_wid {
            s.x_offset = 0;
            let pos = (s.cursor_pos_from_x(before - display_wid).max(0) as usize).min(text.len());
            s.x_offset = measure_string_x(s.font, &jstr(&text[..pos]));
            cursor_x = -s.x_offset + inset + before - 1;
            if cursor_x >= inset + display_wid {
                s.x_offset = measure_string_x(s.font, &jstr(&text[..(pos + 1).min(text.len())]));
            }
            if -s.x_offset + text_wid < display_wid {
                s.x_offset = text_wid - display_wid;
            }
        }
    }

    /// `render()`.
    fn render_box(&self, el: &UIElement, t: &mut Thread) {
        if !el.is_visible() {
            return;
        }
        if let Some(p) = el.get_parent() {
            let mdh = p.get_max_draw_height();
            if mdh != -1.0 && mdh <= el.get_y() {
                return;
            }
        }
        {
            let mut s = self.st.borrow_mut();
            if s.mask {
                if s.internal.len() != s.text.len() {
                    s.text = s.masked();
                }
            } else {
                s.text = s.internal.clone();
            }
        }
        el.render_base(t);
        let width = el.get_width();
        let height = el.get_height();
        let (ax, ay) = (el.get_absolute_x() as i32, el.get_absolute_y() as i32);
        let mut s = self.st.borrow_mut();
        s.paginate(width);
        let lh = line_height(s.font);
        let x_inset = s.inset();
        let mut y_inset = s.inset();
        if s.center_vertically {
            y_inset = ((height - measure_string_y(s.font, &jstr(&s.internal)) as f64) / 2.0) as i32;
        }
        Self::keep_cursor_visible(&mut s, width);
        let mut right = width as i32 - x_inset;
        if s.clear_button && s.clear_button_texture.is_some() && !s.lines.is_empty() {
            right -= 2 + s.clear_button_size + 2;
            let mut alpha = 0.5f32;
            let mx = with_ui(|m| m.mouse.x) as f64;
            if !s.selecting_range && el.is_mouse_over() && mx >= el.get_absolute_x() + right as f64
            {
                alpha = 1.0;
            }
            let frac = match &s.clear_button_transition {
                Some(tr) => {
                    tr.set_fade_in(alpha == 1.0);
                    tr.update();
                    tr.fraction()
                }
                None => 0.0,
            };
            let size = s.clear_button_size;
            el.draw_texture_scaled_aspect(
                s.clear_button_texture.as_ref(),
                (width as f32 - x_inset as f32 - 2.0 - size as f32) as f64,
                (y_inset + (lh - size) / 2) as f64,
                size as f64,
                size as f64,
                1.0,
                1.0,
                1.0,
                (alpha * frac + 0.35 * (1.0 - frac)) as f64,
            );
        }
        let sx1 = el.clamp_to_parent_x((ax + x_inset) as f64) as i32;
        let sx2 = el.clamp_to_parent_x((ax + right) as f64) as i32;
        let sy1 = el.clamp_to_parent_y((ay + y_inset) as f64) as i32;
        let sy2 = el.clamp_to_parent_y((ay + height as i32 - y_inset) as f64) as i32;
        let stencil = [
            (sx1 - ax) as f64,
            (sy1 - ay) as f64,
            (sx2 - sx1) as f64,
            (sy2 - sy1) as f64,
        ];
        el.set_stencil_rect(stencil[0], stencil[1], stencil[2], stencil[3]);
        let highlight = [
            Rgba(0.0, 0.0, 1.0, 33.0 / 255.0),
            Rgba(127.0 / 255.0, 0.0, 1.0, 33.0 / 255.0),
            Rgba(1.0, 0.0, 0.0, 33.0 / 255.0),
            Rgba(0.0, 1.0, 33.0 / 255.0, 1.0),
        ];
        if !s.lines.is_empty() {
            let mut y = y_inset;
            let mut i = s.top_line_index;
            while i < s.top_line_index + s.num_visible_lines && (i as usize) < s.lines.len() {
                if i >= 0 && (i as usize) < s.highlight_lines.len() {
                    let h = s.highlight_lines[i as usize];
                    if (1..=4).contains(&h) {
                        el.draw_texture_scaled_col_c(
                            None,
                            (x_inset - 1) as f64,
                            y as f64,
                            (width as i32 - x_inset * 2 + 2) as f64,
                            lh as f64,
                            highlight[(h - 1) as usize],
                        );
                    }
                }
                if i >= 0 {
                    let line = jstr(&s.lines[i as usize]);
                    let c = s.text_color;
                    draw_string(
                        s.font,
                        (-s.x_offset + ax + x_inset) as f64,
                        (ay + y) as f64,
                        1.0,
                        &line,
                        Rgba(c.0, c.1, c.2, 1.0),
                        TextAlign::Left,
                        true,
                    );
                }
                y += lh;
                i += 1;
            }
        }
        let n = s.text.len() as i32;
        if s.cursor > n {
            s.cursor = n;
        }
        if s.to_selection > n {
            s.to_selection = n;
        }
        s.cursor_line = s.to_display_line(s.cursor);
        if s.doing_text_entry {
            if s.blink_state {
                let mut off = 0;
                if !s.lines.is_empty() {
                    let line = s.line(s.cursor_line).to_vec();
                    let k = (s.cursor - s.start(s.cursor_line)).min(line.len() as i32);
                    off = font_width_range(s.font, &jstr(&line), 0, k - 1, true);
                    if off > 0 {
                        off -= 1;
                    }
                }
                let white = crate::bind_render::textures().borrow_mut().white();
                el.draw_texture_scaled_col_c(
                    Some(&white),
                    (-s.x_offset + x_inset + off) as f64,
                    (y_inset + s.cursor_line * lh) as f64,
                    1.0,
                    lh as f64,
                    color(170, 170, 220, 240),
                );
            }
            if !s.lines.is_empty() && s.to_selection != s.cursor {
                let (lo, hi) = s.sel_range();
                let (l1, l2) = (s.to_display_line(lo), s.to_display_line(hi));
                for i in l1..=l2 {
                    let st = s.start(i);
                    let line = s.line(i).to_vec();
                    let l = st.max(lo);
                    let h = (st + line.len() as i32).min(hi);
                    let txt = jstr(&line);
                    let lx = font_width_range(s.font, &txt, 0, l - st - 1, true);
                    let hx = font_width_range(s.font, &txt, 0, h - st - 1, true);
                    el.draw_texture_scaled_col_c(
                        None,
                        (-s.x_offset + x_inset + lx) as f64,
                        (y_inset + i * lh) as f64,
                        (hx - lx) as f64,
                        lh as f64,
                        color(100, 100, 220, 160),
                    );
                }
            }
        } else if s.internal.is_empty() {
            if let Some(p) = s.placeholder.clone() {
                let c = s.placeholder_color;
                draw_string(
                    s.font,
                    (-s.x_offset + ax + x_inset) as f64,
                    (ay + y_inset) as f64,
                    1.0,
                    &p,
                    Rgba(c.0, c.1, c.2, 1.0),
                    TextAlign::Left,
                    true,
                );
            }
        }
        drop(s);
        el.clear_stencil_rect();
        if with_ui(|m| m.sprite.stencil_level) > 0 {
            el.repaint_stencil_rect(stencil[0], stencil[1], stencil[2], stencil[3]);
        }
    }

    // ---- keys (UITextEntryInterface) ----

    fn shift() -> bool {
        crate::bind_input::is_key_down_raw(42) || crate::bind_input::is_key_down_raw(54)
    }

    /// `onKeyEnter()` (no debug console).
    pub fn on_key_enter(&self, el: &UIElement, t: &mut Thread) {
        let multiple = self.st.borrow().multiple_line;
        if !multiple {
            return self.on_command_entered(el, t);
        }
        let (lines, max) = {
            let s = self.st.borrow();
            (s.lines.len() as i32, s.max_lines)
        };
        if lines >= max {
            return;
        }
        let set_text = {
            let mut s = self.st.borrow_mut();
            if s.cursor != s.to_selection {
                let (l, h) = s.sel_range();
                s.internal = if !s.internal.is_empty() {
                    let mut v = s.internal_sub(0, l);
                    v.push('\n');
                    v.extend(s.internal_sub(h, i32::MAX));
                    v
                } else {
                    vec!['\n']
                };
                s.cursor = l + 1;
                None
            } else {
                let off = s.cursor;
                let mut v = s.internal_sub(0, off);
                v.push('\n');
                v.extend(s.internal_sub(off, i32::MAX));
                Some((v, off))
            }
        };
        if let Some((v, off)) = set_text {
            self.set_text(el, t, &v);
            self.st.borrow_mut().cursor = off + 1;
        }
        let mut s = self.st.borrow_mut();
        s.to_selection = s.cursor;
        s.cursor_line = s.to_display_line(s.cursor);
    }

    pub fn on_key_home(&self) {
        let shift = Self::shift();
        let mut s = self.st.borrow_mut();
        s.cursor = 0;
        if !s.lines.is_empty() {
            s.cursor = s.start(s.cursor_line);
        }
        if !shift {
            s.to_selection = s.cursor;
        }
        s.reset_blink();
    }

    pub fn on_key_end(&self) {
        let shift = Self::shift();
        let mut s = self.st.borrow_mut();
        s.cursor = s.internal.len() as i32;
        if !s.lines.is_empty() {
            s.cursor = s.start(s.cursor_line) + s.line(s.cursor_line).len() as i32;
        }
        if !shift {
            s.to_selection = s.cursor;
        }
        s.reset_blink();
    }

    pub fn on_key_up(&self, el: &UIElement, t: &mut Thread) {
        let shift = Self::shift();
        {
            let mut s = self.st.borrow_mut();
            if s.cursor_line > 0 {
                let mut col = s.cursor - s.start(s.cursor_line);
                s.cursor_line -= 1;
                col = col.min(s.line(s.cursor_line).len() as i32);
                s.cursor = s.start(s.cursor_line) + col;
                if !shift {
                    s.to_selection = s.cursor;
                }
            }
        }
        self.on_press_up(el, t);
    }

    pub fn on_key_down(&self, el: &UIElement, t: &mut Thread) {
        let shift = Self::shift();
        {
            let mut s = self.st.borrow_mut();
            if s.lines.len() as i32 - 1 > s.cursor_line && s.cursor_line + 1 < s.max_lines {
                let mut col = s.cursor - s.start(s.cursor_line);
                s.cursor_line += 1;
                col = col.min(s.line(s.cursor_line).len() as i32);
                s.cursor = s.start(s.cursor_line) + col;
                if !shift {
                    s.to_selection = s.cursor;
                }
            }
        }
        self.on_press_down(el, t);
    }

    pub fn on_key_left(&self) {
        let shift = Self::shift();
        let mut s = self.st.borrow_mut();
        s.cursor = (s.cursor - 1).max(0);
        if !shift {
            s.to_selection = s.cursor;
        }
        s.reset_blink();
    }

    pub fn on_key_right(&self) {
        let shift = Self::shift();
        let mut s = self.st.borrow_mut();
        s.cursor = (s.cursor + 1).min(s.internal.len() as i32);
        if !shift {
            s.to_selection = s.cursor;
        }
        s.reset_blink();
    }

    /// `onTextDelete()`.
    fn on_text_delete(&self, el: &UIElement, t: &mut Thread) {
        {
            let mut s = self.st.borrow_mut();
            let (l, h) = s.sel_range();
            let mut v = s.internal_sub(0, l);
            v.extend(s.internal_sub(h, i32::MAX));
            s.internal = v;
            s.cursor_line = s.to_display_line(l);
            s.to_selection = l;
            s.cursor = l;
        }
        self.on_text_change(el, t);
    }

    pub fn on_key_back(&self, el: &UIElement, t: &mut Thread) {
        let (sel, empty, cursor) = {
            let s = self.st.borrow();
            (s.cursor != s.to_selection, s.internal.is_empty(), s.cursor)
        };
        if sel {
            return self.on_text_delete(el, t);
        }
        if empty || cursor <= 0 {
            return;
        }
        {
            let mut s = self.st.borrow_mut();
            let n = s.internal.len() as i32;
            if s.cursor > n {
                s.internal.pop();
            } else {
                let off = s.cursor;
                let mut v = s.internal_sub(0, off - 1);
                v.extend(s.internal_sub(off, i32::MAX));
                s.internal = v;
            }
            s.cursor -= 1;
            s.to_selection = s.cursor;
        }
        self.on_text_change(el, t);
    }

    pub fn on_key_delete(&self, el: &UIElement, t: &mut Thread) {
        let (sel, empty, at_end) = {
            let s = self.st.borrow();
            (
                s.cursor != s.to_selection,
                s.internal.is_empty(),
                s.cursor >= s.internal.len() as i32,
            )
        };
        if sel {
            return self.on_text_delete(el, t);
        }
        if empty || at_end {
            return;
        }
        {
            let mut s = self.st.borrow_mut();
            s.internal = if s.cursor > 0 {
                let mut v = s.internal_sub(0, s.cursor);
                v.extend(s.internal_sub(s.cursor + 1, i32::MAX));
                v
            } else {
                s.internal_sub(1, i32::MAX)
            };
        }
        self.on_text_change(el, t);
    }

    pub fn paste_from_clipboard(&self, el: &UIElement, t: &mut Thread) {
        let clip: Vec<char> = chars(&CLIPBOARD.with(|c| c.borrow().clone()));
        {
            let mut s = self.st.borrow_mut();
            if s.cursor != s.to_selection {
                let (l, h) = s.sel_range();
                let mut v = s.internal_sub(0, l);
                v.extend(&clip);
                v.extend(s.internal_sub(h, i32::MAX));
                s.internal = v;
                s.to_selection = l + clip.len() as i32;
                s.cursor = l + clip.len() as i32;
            } else {
                s.internal = if (s.cursor as usize) < s.internal.len() {
                    let mut v = s.internal_sub(0, s.cursor);
                    v.extend(&clip);
                    v.extend(s.internal_sub(s.cursor, i32::MAX));
                    v
                } else {
                    let mut v = s.internal.clone();
                    v.extend(&clip);
                    v
                };
                s.cursor += clip.len() as i32;
                s.to_selection += clip.len() as i32;
            }
        }
        self.on_text_change(el, t);
    }

    /// The selected part of `text` (masked text copies asterisks).
    fn selection_text(&self) -> Option<String> {
        let s = self.st.borrow();
        if s.cursor == s.to_selection {
            return None;
        }
        let (l, h) = s.sel_range();
        let n = s.text.len() as i32;
        Some(jstr(
            &s.text[l.clamp(0, n) as usize..h.clamp(0, n) as usize],
        ))
    }

    pub fn cut_to_clipboard(&self) {
        if self.st.borrow().cursor == self.st.borrow().to_selection {
            return;
        }
        self.update_text();
        if let Some(clip) = self.selection_text().filter(|c| !c.is_empty()) {
            CLIPBOARD.with(|c| *c.borrow_mut() = clip);
        }
        let mut s = self.st.borrow_mut();
        let (l, h) = s.sel_range();
        let mut v = s.internal_sub(0, l);
        v.extend(s.internal_sub(h, i32::MAX));
        s.internal = v;
        s.to_selection = l;
        s.cursor = l;
    }

    pub fn copy_to_clipboard(&self) {
        if self.st.borrow().cursor == self.st.borrow().to_selection {
            return;
        }
        self.update_text();
        if let Some(clip) = self.selection_text().filter(|c| !c.is_empty()) {
            CLIPBOARD.with(|c| *c.borrow_mut() = clip);
        }
    }

    pub fn select_all(&self) {
        let mut s = self.st.borrow_mut();
        s.cursor = s.internal.len() as i32;
        s.to_selection = 0;
    }

    pub fn is_text_limit(&self) -> bool {
        let s = self.st.borrow();
        s.internal.len() as i32 >= s.text_entry_max_length
    }

    pub fn put_character(&self, el: &UIElement, t: &mut Thread, ch: char) {
        {
            let mut s = self.st.borrow_mut();
            if s.cursor == s.to_selection {
                let off = s.cursor;
                let mut v = s.internal_sub(0, off);
                v.push(ch);
                v.extend(s.internal_sub(off, i32::MAX));
                s.internal = v;
                s.cursor += 1;
                s.to_selection += 1;
            } else {
                let (l, h) = s.sel_range();
                s.internal = if !s.internal.is_empty() {
                    let mut v = s.internal_sub(0, l);
                    v.push(ch);
                    v.extend(s.internal_sub(h, i32::MAX));
                    v
                } else {
                    vec![ch]
                };
                s.to_selection = l + 1;
                s.cursor = l + 1;
            }
        }
        self.on_text_change(el, t);
    }

    // ---- getters for tests and the engine ----

    pub fn internal_text(&self) -> String {
        jstr(&self.st.borrow().internal)
    }
    pub fn cursor_pos(&self) -> i32 {
        self.st.borrow().cursor
    }
    pub fn selection_index(&self) -> i32 {
        self.st.borrow().to_selection
    }
    pub fn lines(&self) -> Vec<String> {
        self.st.borrow().lines.iter().map(|l| jstr(l)).collect()
    }
}

impl UIElementExt for UITextBox2 {
    fn class_name(&self) -> &'static str {
        UITEXTBOX2
    }
    fn super_classes(&self) -> &'static [&'static str] {
        TEXTBOX_SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn render(&self, el: &UIElement, t: &mut Thread) {
        self.render_box(el, t);
    }

    fn update(&self, el: &UIElement, t: &mut Thread) {
        self.update_box(el, t);
    }

    fn on_mouse_up(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        el.on_mouse_up_base(t, x, y);
        self.st.borrow_mut().selecting_range = false;
        true
    }

    fn on_mouse_up_outside(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) {
        if !el.is_visible() {
            return;
        }
        el.on_mouse_up_outside_base(t, x, y);
        self.st.borrow_mut().selecting_range = false;
    }

    fn on_mouse_move(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) -> bool {
        let (mx, my) = with_ui(|m| (m.mouse.x, m.mouse.y));
        if !el.is_visible() {
            return false;
        }
        let consume = el.is_consume_mouse_events();
        el.set_consume_mouse_events(false);
        let handled = el.on_mouse_move_base(t, dx, dy);
        el.set_consume_mouse_events(consume);
        if handled {
            return true;
        }
        let (ax, ay, ys) = (
            el.get_absolute_x(),
            el.get_absolute_y() as i32,
            el.get_y_scroll() as i32,
        );
        let mut s = self.st.borrow_mut();
        if (s.is_editable || s.is_selectable) && s.selecting_range {
            if s.multiple_line {
                let lh = line_height(s.font).max(1);
                s.cursor_line = (my - ay - s.inset() - ys) / lh;
                if s.cursor_line > s.lines.len() as i32 - 1 {
                    s.cursor_line = s.lines.len() as i32 - 1;
                }
            }
            s.cursor = s.cursor_pos_from_x((mx as f64 - ax) as i32);
            return true;
        }
        false
    }

    fn on_mouse_move_outside(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) {
        let (mx, my, down0) = with_ui(|m| {
            (
                m.mouse.x,
                m.mouse.y,
                m.mouse.down.first().copied().unwrap_or(false),
            )
        });
        if !down0 {
            self.st.borrow_mut().selecting_range = false;
        }
        if !el.is_visible() {
            return;
        }
        el.on_mouse_move_outside_base(t, dx, dy);
        let (ax, ay, ys) = (
            el.get_absolute_x(),
            el.get_absolute_y() as i32,
            el.get_y_scroll() as i32,
        );
        let mut s = self.st.borrow_mut();
        if (s.is_editable || s.is_selectable) && s.selecting_range {
            if s.multiple_line {
                let lh = line_height(s.font).max(1);
                s.cursor_line = ((my - ay - s.inset() - ys) / lh).max(0);
                if s.cursor_line > s.lines.len() as i32 - 1 {
                    s.cursor_line = s.lines.len() as i32 - 1;
                }
            }
            s.cursor = s.cursor_pos_from_x((mx as f64 - ax) as i32);
        }
    }

    fn on_mouse_down(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        let frame = self.st.borrow().frame.clone();
        let mut i = 0;
        while let Some(ui) = el.control(i) {
            i += 1;
            if frame.as_ref().is_some_and(|f| Rc::ptr_eq(f, &ui)) || !ui.is_mouse_over() {
                continue;
            }
            let lx = x - (ui.get_x_scrolled(Some(el)) as i32) as f64;
            let ly = y - (ui.get_y_scrolled(Some(el)) as i32) as f64;
            return ui.on_mouse_down(t, lx, ly);
        }
        let clear = {
            let s = self.st.borrow();
            s.clear_button && s.clear_button_texture.is_some() && !s.lines.is_empty() && {
                let right = el.get_width() as i32 - s.inset() - (2 + s.clear_button_size + 2);
                x >= right as f64
            }
        };
        if clear {
            self.clear_input(el, t);
            return true;
        }
        {
            let ys = el.get_y_scroll() as i32;
            let mut s = self.st.borrow_mut();
            if s.multiple_line {
                let lh = line_height(s.font).max(1);
                s.cursor_line = (y as i32 - s.inset() - ys) / lh;
                if s.cursor_line > s.lines.len() as i32 - 1 {
                    s.cursor_line = s.lines.len() as i32 - 1;
                }
            }
        }
        let (editable, selectable) = {
            let s = self.st.borrow();
            (s.is_editable, s.is_selectable)
        };
        if !(editable || selectable) {
            self.reset_frame_colour();
            self.st.borrow_mut().doing_text_entry = false;
            sync_core();
            return false;
        }
        if !is_current(el) {
            if let Some(cur) = current_text_entry_box() {
                if let Some(b) = cur.ext::<UITextBox2>() {
                    b.set_doing_text_entry(&cur, t, false);
                    if b.st.borrow().frame.is_some() {
                        b.reset_frame_colour();
                    }
                }
            }
            set_current(Some(el.rc()));
            self.st.borrow_mut().selecting_range = true;
        }
        if !self.st.borrow().doing_text_entry {
            self.focus(el);
            let mut s = self.st.borrow_mut();
            let p = s.cursor_pos_from_x(x as i32);
            s.cursor = p;
            s.to_selection = p;
            drop(s);
            self.set_frame_colour(c4(TEXT_ENTRY_FRAME));
        } else {
            let mut s = self.st.borrow_mut();
            let p = s.cursor_pos_from_x(x as i32);
            s.cursor = p;
            s.to_selection = p;
        }
        sync_core();
        true
    }
}

// ---- Lua ------------------------------------------------------------------------

fn font_arg(c: &NativeCall<'_>, i: usize) -> Option<UIFont> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == UIFONT)
            .and_then(|e| ui_font_from_name(e.name)),
        _ => None,
    }
}

fn this(c: &NativeCall<'_>) -> LuaResult<Rc<UIElement>> {
    let e = element_of(&c.arg(0)).ok_or_else(|| LuaError::runtime("Expected UITextBox2"))?;
    if e.ext::<UITextBox2>().is_none() {
        return Err(LuaError::runtime(format!(
            "Expected UITextBox2 but got {}",
            e.class_name()
        )));
    }
    Ok(e)
}

fn tb(e: &UIElement) -> &UITextBox2 {
    e.ext::<UITextBox2>().unwrap()
}

fn el_value(e: Option<Rc<UIElement>>) -> Value {
    e.map(|e| Value::Object(e as Rc<dyn LuaObject>))
        .unwrap_or_default()
}

/// Register `UITextBox2` and `UINineGrid` (after `UIElement`) and hook
/// `Core.UnfocusActiveTextEntryBox` into `UIManager`.
pub fn register(thread: &mut Thread) {
    pz_ui::with_ui(|m| m.unfocus_text_entry = Some(Rc::new(unfocus_active_text_entry_box)));
    register_nine_grid(thread);
    let b = expose_class(thread, UITEXTBOX2, Some(UIELEMENT));
    b.ctor(|c| {
        let font = font_arg(c, 0);
        let (x, y, w, h) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let text = str_arg(c, 5).map(|s| s.to_string()).unwrap_or_default();
        let frame = bool_arg(c, 6);
        let e = UITextBox2::create(c.thread, font, x, y, w, h, &text, frame);
        Ok(c.push(Value::Object(e as Rc<dyn LuaObject>)))
    });

    macro_rules! get {
        ($name:literal, |$s:ident| $e:expr) => {
            b.method($name, |c| {
                let el = this(c)?;
                let v = {
                    let $s = tb(&el).st.borrow();
                    $e
                };
                Ok(c.push(v))
            });
        };
    }
    macro_rules! set {
        ($name:literal, $arg:ident, |$s:ident, $v:ident| $e:expr) => {
            b.method($name, |c| {
                let el = this(c)?;
                let $v = $arg(c, 1);
                {
                    #[allow(unused_mut)]
                    let mut $s = tb(&el).st.borrow_mut();
                    $e;
                }
                Ok(0)
            });
        };
    }
    fn fnum(c: &NativeCall<'_>, i: usize) -> f32 {
        num_arg(c, i) as f32
    }

    b.method("setFont", |c| {
        let el = this(c)?;
        let f = font_arg(c, 1);
        let mut s = tb(&el).st.borrow_mut();
        s.font = f;
        s.clear_button_size = font_height(f) - 4;
        Ok(0)
    });
    b.method("ClearHighlights", |c| {
        let el = this(c)?;
        tb(&el)
            .st
            .borrow_mut()
            .highlight_lines
            .iter_mut()
            .for_each(|h| *h = 0);
        Ok(0)
    });
    b.method("setMasked", |c| {
        let el = this(c)?;
        let v = bool_arg(c, 1);
        let mut s = tb(&el).st.borrow_mut();
        if s.mask != v {
            s.mask = v;
            s.text = if v { s.masked() } else { s.internal.clone() };
        }
        Ok(0)
    });
    get!("isMasked", |s| s.mask);
    b.method("getFrameAlpha", |c| {
        let el = this(c)?;
        let f = tb(&el).st.borrow().frame.clone();
        let a = f
            .as_ref()
            .and_then(|f| f.ext::<UINineGrid>())
            .map(UINineGrid::get_alpha)
            .ok_or_else(|| LuaError::runtime("frame is null"))?;
        Ok(c.push(a as f64))
    });
    b.method("setFrameAlpha", |c| {
        let el = this(c)?;
        let a = fnum(c, 1);
        let f = tb(&el).st.borrow().frame.clone();
        match f.as_ref().and_then(|f| f.ext::<UINineGrid>()) {
            Some(g) => g.set_alpha(a),
            None => return Err(LuaError::runtime("frame is null")),
        }
        Ok(0)
    });
    b.method("setTextColor", |c| {
        let el = this(c)?;
        let ci = color_arg(c, 1)?;
        tb(&el).st.borrow_mut().text_color = Rgba(ci.r, ci.g, ci.b, ci.a);
        Ok(0)
    });
    b.method("setTextRGBA", |c| {
        let el = this(c)?;
        let v = Rgba(fnum(c, 1), fnum(c, 2), fnum(c, 3), fnum(c, 4));
        tb(&el).st.borrow_mut().text_color = v;
        Ok(0)
    });
    get!("getText", |s| jstr(&s.text));
    get!("getInternalText", |s| jstr(&s.internal));
    get!("getInset", |s| s.inset() as f64);
    b.method("setEditable", |c| {
        let el = this(c)?;
        tb(&el).st.borrow_mut().is_editable = bool_arg(c, 1);
        sync_core();
        Ok(0)
    });
    get!("isEditable", |s| s.is_editable);
    set!("setSelectable", bool_arg, |s, v| s.is_selectable = v);
    get!("isSelectable", |s| s.is_selectable);
    b.method("focus", |c| {
        let el = this(c)?;
        tb(&el).focus(&el);
        Ok(0)
    });
    b.method("unfocus", |c| {
        let el = this(c)?;
        tb(&el).unfocus(&el, c.thread);
        Ok(0)
    });
    b.method("ignoreFirstInput", |c| {
        let el = this(c)?;
        tb(&el).st.borrow_mut().ignore_first = true;
        Ok(0)
    });
    b.method("updateText", |c| {
        let el = this(c)?;
        tb(&el).update_text();
        Ok(0)
    });
    b.method("SetText", |c| {
        let el = this(c)?;
        let text = match c.arg(1) {
            Value::Nil => {
                return Err(LuaError::Runtime {
                    message: "text is null".into(),
                    class: "java.lang.NullPointerException",
                })
            }
            _ => str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default(),
        };
        tb(&el).set_text(&el, c.thread, &chars(&text));
        Ok(0)
    });
    b.method("setPlaceholderText", |c| {
        let el = this(c)?;
        let p = str_arg(c, 1)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());
        tb(&el).st.borrow_mut().placeholder = p;
        Ok(0)
    });
    get!("getPlaceholderText", |s| s
        .placeholder
        .as_deref()
        .map(Value::str)
        .unwrap_or_default());
    b.method("setPlaceholderTextColor", |c| {
        let el = this(c)?;
        let ci = color_arg(c, 1)?;
        tb(&el).st.borrow_mut().placeholder_color = Rgba(ci.r, ci.g, ci.b, ci.a);
        Ok(0)
    });
    b.method("setPlaceholderTextRGBA", |c| {
        let el = this(c)?;
        let v = Rgba(fnum(c, 1), fnum(c, 2), fnum(c, 3), fnum(c, 4));
        tb(&el).st.borrow_mut().placeholder_color = v;
        Ok(0)
    });
    b.method("clearInput", |c| {
        let el = this(c)?;
        tb(&el).clear_input(&el, c.thread);
        Ok(0)
    });
    macro_rules! lua_cb {
        ($name:literal, $f:ident) => {
            b.method($name, |c| {
                let el = this(c)?;
                tb(&el).$f(&el, c.thread);
                Ok(0)
            });
        };
    }
    lua_cb!("onPressUp", on_press_up);
    lua_cb!("onPressDown", on_press_down);
    lua_cb!("onCommandEntered", on_command_entered);
    lua_cb!("onTextChange", on_text_change);
    lua_cb!("onLostFocus", on_lost_focus);
    lua_cb!("onKeyEnter", on_key_enter);
    lua_cb!("onKeyUp", on_key_up);
    lua_cb!("onKeyDown", on_key_down);
    lua_cb!("onKeyBack", on_key_back);
    lua_cb!("onKeyDelete", on_key_delete);
    lua_cb!("pasteFromClipboard", paste_from_clipboard);
    b.method("onOtherKey", |c| {
        let el = this(c)?;
        let k = int_arg(c, 1);
        tb(&el).on_other_key(&el, c.thread, k);
        Ok(0)
    });
    macro_rules! plain {
        ($name:literal, $f:ident) => {
            b.method($name, |c| {
                let el = this(c)?;
                tb(&el).$f();
                Ok(0)
            });
        };
    }
    plain!("onKeyHome", on_key_home);
    plain!("onKeyEnd", on_key_end);
    plain!("onKeyLeft", on_key_left);
    plain!("onKeyRight", on_key_right);
    plain!("copyToClipboard", copy_to_clipboard);
    plain!("cutToClipboard", cut_to_clipboard);
    plain!("selectAll", select_all);
    b.method("resetBlink", |c| {
        let el = this(c)?;
        tb(&el).st.borrow_mut().reset_blink();
        Ok(0)
    });
    get!("getMaxTextLength", |s| s.max_text_length as f64);
    set!("setMaxTextLength", int_arg, |s, v| s.max_text_length = v);
    get!("getForceUpperCase", |s| s.force_upper_case);
    set!("setForceUpperCase", bool_arg, |s, v| s.force_upper_case = v);
    get!("getHasFrame", |s| s.has_frame);
    b.method("setHasFrame", |c| {
        let el = this(c)?;
        let v = bool_arg(c, 1);
        let b = tb(&el);
        if b.st.borrow().has_frame == v {
            return Ok(0);
        }
        b.st.borrow_mut().has_frame = v;
        if v {
            let f = UINineGrid::create(
                0,
                0,
                el.get_width() as i32,
                el.get_height() as i32,
                5,
                5,
                5,
                5,
                BOX_TEXTURES,
            );
            f.set_anchor_right(true);
            b.st.borrow_mut().frame = Some(f.clone());
            el.add_child(&f);
        } else {
            let f = b.st.borrow_mut().frame.take();
            if let Some(f) = f {
                el.remove_child(&f);
            }
        }
        Ok(0)
    });
    b.method("setClearButton", |c| {
        let el = this(c)?;
        let v = bool_arg(c, 1);
        let mut s = tb(&el).st.borrow_mut();
        s.clear_button = v;
        if v && s.clear_button_transition.is_none() {
            s.clear_button_transition = Some(Rc::new(UITransition::new()));
        }
        Ok(0)
    });
    get!("hasClearButton", |s| s.clear_button);
    b.method("toDisplayLine", |c| {
        let el = this(c)?;
        let v = tb(&el).st.borrow().to_display_line(int_arg(c, 1));
        Ok(c.push(v as f64))
    });
    set!("setMultipleLine", bool_arg, |s, v| s.multiple_line = v);
    get!("isMultipleLine", |s| s.multiple_line);
    get!("getCursorLine", |s| s.cursor_line as f64);
    set!("setCursorLine", int_arg, |s, v| s.cursor_line = v);
    get!("getCursorPos", |s| s.cursor as f64);
    b.method("setCursorPos", |c| {
        let el = this(c)?;
        let i = int_arg(c, 1);
        let mut s = tb(&el).st.borrow_mut();
        if s.multiple_line {
            if s.cursor_line >= 0 && (s.cursor_line as usize) < s.lines.len() {
                let n = s.line(s.cursor_line).len() as i32;
                s.cursor = i.clamp(0, n);
            }
        } else {
            let n = s.internal.len() as i32;
            s.cursor = i.clamp(0, n);
        }
        s.to_selection = s.cursor;
        Ok(0)
    });
    get!("getMaxLines", |s| s.max_lines as f64);
    set!("setMaxLines", int_arg, |s, v| s.max_lines = v);
    get!("isFocused", |s| s.doing_text_entry);
    get!("isOnlyNumbers", |s| s.only_numbers);
    set!("setOnlyNumbers", bool_arg, |s, v| s.only_numbers = v);
    get!("isOnlyText", |s| s.only_text);
    set!("setOnlyText", bool_arg, |s, v| s.only_text = v);
    get!("isDoingTextEntry", |s| s.doing_text_entry);
    b.method("setDoingTextEntry", |c| {
        let el = this(c)?;
        let v = bool_arg(c, 1);
        tb(&el).set_doing_text_entry(&el, c.thread, v);
        Ok(0)
    });
    b.method("getFrame", |c| {
        let el = this(c)?;
        let f = tb(&el).st.borrow().frame.clone();
        Ok(c.push(el_value(f)))
    });
    get!("isIgnoreFirst", |s| s.ignore_first);
    set!("setIgnoreFirst", bool_arg, |s, v| s.ignore_first = v);
    set!("setSelectingRange", bool_arg, |s, v| s.selecting_range = v);
    b.method("getStandardFrameColour", |c| {
        let c4v = c4(STANDARD_FRAME);
        Ok(c.push(Value::Object(Rc::new(ColorObject::new(
            c4v.0, c4v.1, c4v.2, c4v.3,
        )))))
    });
    b.method("isTextLimit", |c| {
        let el = this(c)?;
        let v = tb(&el).is_text_limit();
        Ok(c.push(v))
    });
    b.method("putCharacter", |c| {
        let el = this(c)?;
        let ch = match c.arg(1) {
            Value::Str(s) => s.chars().next().unwrap_or('\0'),
            other => char::from_u32(pz_kahlua::exec::raw_tonumber(&other).unwrap_or(0.0) as u32)
                .unwrap_or('\0'),
        };
        tb(&el).put_character(&el, c.thread, ch);
        Ok(0)
    });
    set!("setWrapLines", bool_arg, |s, v| s.wrap_lines = v);
    set!("setCentreVertically", bool_arg, |s, v| s
        .center_vertically =
        v);
}

fn register_nine_grid(thread: &mut Thread) {
    let b = expose_class(thread, UININEGRID, Some(UIELEMENT));
    b.ctor(|c| {
        let tex: [String; 9] =
            std::array::from_fn(|i| str_arg(c, 8 + i).map(|s| s.to_string()).unwrap_or_default());
        let tex_refs: [&str; 9] = std::array::from_fn(|i| tex[i].as_str());
        let e = UINineGrid::create(
            int_arg(c, 0),
            int_arg(c, 1),
            int_arg(c, 2),
            int_arg(c, 3),
            int_arg(c, 4),
            int_arg(c, 5),
            int_arg(c, 6),
            int_arg(c, 7),
            tex_refs,
        );
        Ok(c.push(Value::Object(e as Rc<dyn LuaObject>)))
    });
    fn grid(c: &NativeCall<'_>) -> LuaResult<Rc<UIElement>> {
        let e = element_of(&c.arg(0)).ok_or_else(|| LuaError::runtime("Expected UINineGrid"))?;
        if e.ext::<UINineGrid>().is_none() {
            return Err(LuaError::runtime(format!(
                "Expected UINineGrid but got {}",
                e.class_name()
            )));
        }
        Ok(e)
    }
    b.method("setAlpha", |c| {
        let e = grid(c)?;
        e.ext::<UINineGrid>()
            .unwrap()
            .set_alpha(num_arg(c, 1) as f32);
        Ok(0)
    });
    b.method("getAlpha", |c| {
        let e = grid(c)?;
        let a = e.ext::<UINineGrid>().unwrap().get_alpha();
        Ok(c.push(a as f64))
    });
    b.method("Nest", |c| {
        let e = grid(c)?;
        let child = element_of(&c.arg(1)).ok_or_else(|| LuaError::runtime("Expected UIElement"))?;
        let (top, r, bt, l) = (int_arg(c, 2), int_arg(c, 3), int_arg(c, 4), int_arg(c, 5));
        e.ext::<UINineGrid>()
            .unwrap()
            .nest(&e, &child, c.thread, top, r, bt, l);
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;
    use pz_ui::{SpriteRenderer, TextRenderer, UIManager, UiDrawCmd};

    /// A monospace font: 8 px per character, 16 px lines.
    struct Mono;

    impl TextRenderer for Mono {
        fn measure_x(&self, _: Option<UIFont>, s: &str) -> i32 {
            8 * s.chars().count() as i32
        }
        fn measure_y(&self, _: Option<UIFont>, s: &str) -> i32 {
            16 * (s.matches('\n').count() as i32 + 1)
        }
        fn font_height(&self, _: Option<UIFont>) -> i32 {
            16
        }
        fn draw(
            &self,
            out: &mut SpriteRenderer,
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

    fn setup() -> Thread {
        UIManager::reset();
        UIManager::set_text_renderer(Some(Rc::new(Mono)));
        with_ui(|m| m.time_override_ms = Some(1_000_000));
        CURRENT.with(|c| *c.borrow_mut() = None);
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_ui::register(&mut t);
        register(&mut t);
        run(
            &mut t,
            r#"
            log = {}
            Box = {}
            Box.__index = Box
            function Box:onTextChange() log[#log + 1] = "change " .. self.javaObject:getInternalText() end
            function Box:onCommandEntered() log[#log + 1] = "command " .. self.javaObject:getInternalText() end
            function Box:onOtherKey(k) log[#log + 1] = "other " .. k end
            function Box:onLostFocus() log[#log + 1] = "lost" end
            function newBox(x, y, w, h, text)
                local o = setmetatable({}, Box)
                o.javaObject = UITextBox2.new(UIFont.Small, x, y, w, h, text, true)
                o.javaObject:setTable(o)
                o.javaObject:setEditable(true)
                UIManager.AddUI(o.javaObject)
                return o
            end
        "#,
        );
        t
    }

    fn run(t: &mut Thread, src: &str) {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        if let Err((e, tr)) = t.pcall(&f, &[]) {
            panic!("lua error: {e:?}\n{tr}");
        }
    }

    fn eval(t: &mut Thread, expr: &str) -> Value {
        run(t, &format!("__r = {expr}"));
        t.globals().get_str("__r")
    }

    fn log(t: &mut Thread) -> Vec<String> {
        let l = t.globals().get_str("log");
        let l = l.as_table().unwrap();
        let v = (1..=l.len())
            .map(|i| l.get_int(i).as_str().unwrap().to_string())
            .collect();
        run(t, "log = {}");
        v
    }

    fn el(t: &mut Thread, var: &str) -> Rc<UIElement> {
        element_of(&eval(t, &format!("{var}.javaObject"))).unwrap()
    }

    /// Typed characters and keys as the window queues them.
    fn type_keys(t: &mut Thread, keys: &[i32], text: &str) {
        pz_input::with_input(|i| {
            for &k in keys {
                i.key_events.add_key_event(k, 1, true, 0);
            }
            for c in text.chars() {
                i.key_events.add_char_event(c, 0);
            }
        });
        update_keyboard(t);
    }

    #[test]
    fn construction_identity_and_frame() {
        let mut t = setup();
        run(&mut t, "b = newBox(10, 20, 200, 10, 'hello')");
        let e = el(&mut t, "b");
        assert_eq!(
            eval(&mut t, "b.javaObject:getCursorPos()").as_num(),
            Some(0.0)
        );
        UIManager::update(&mut t);
        assert_eq!(e.class_name(), UITEXTBOX2);
        assert!(e.super_classes().contains(&UIELEMENT));
        // The frame is the one child; the height grew to one line.
        assert_eq!(e.controls_len(), 1);
        assert_eq!(e.control(0).unwrap().class_name(), UININEGRID);
        assert_eq!(e.get_height(), 26.0);
        assert_eq!(
            eval(&mut t, "b.javaObject:getText()").as_str(),
            Some("hello")
        );
        run(&mut t, "b.javaObject:SetText('hello!')");
        assert_eq!(
            eval(&mut t, "b.javaObject:getCursorPos()").as_num(),
            Some(6.0)
        );
        assert_eq!(eval(&mut t, "b.javaObject:getInset()").as_num(), Some(5.0));
        // The constructor makes it Core.currentTextEntryBox (not focused).
        assert!(current_text_entry_box().is_some_and(|c| Rc::ptr_eq(&c, &e)));
        assert!(!is_doing_text_entry());
        run(&mut t, "b.javaObject:setHasFrame(false)");
        assert_eq!(e.controls_len(), 0);
        assert_eq!(eval(&mut t, "b.javaObject:getInset()").as_num(), Some(2.0));
    }

    #[test]
    fn typing_cursor_selection_clipboard() {
        let mut t = setup();
        run(&mut t, "b = newBox(0, 0, 300, 30, '')");
        let e = el(&mut t, "b");
        // Typing does nothing until the box is focused.
        type_keys(&mut t, &[], "ab");
        assert_eq!(tb(&e).internal_text(), "");
        // (The queue is left as it is, as LWJGL's is; drop those events.)
        while pz_input::with_input(|i| i.key_events.next()) {}
        run(&mut t, "b.javaObject:focus()");
        assert!(is_doing_text_entry());
        assert!(
            shared_core()
                .borrow()
                .current_text_entry_box
                .as_ref()
                .unwrap()
                .doing_text_entry
        );
        type_keys(&mut t, &[], "abc");
        assert_eq!(log(&mut t), ["change a", "change ab", "change abc"]);
        // Left twice, insert, Home, insert, End, Backspace, Delete at end.
        type_keys(&mut t, &[203, 203], "X");
        assert_eq!(tb(&e).internal_text(), "aXbc");
        type_keys(&mut t, &[199], "");
        assert_eq!(tb(&e).cursor_pos(), 0);
        type_keys(&mut t, &[211], "");
        assert_eq!(tb(&e).internal_text(), "Xbc");
        type_keys(&mut t, &[207, 14, 211], "");
        assert_eq!(tb(&e).internal_text(), "Xb");
        log(&mut t);
        // Select all, copy, type over the selection, paste.
        run(
            &mut t,
            "b.javaObject:selectAll(); b.javaObject:copyToClipboard()",
        );
        assert_eq!(CLIPBOARD.with(|c| c.borrow().clone()), "Xb");
        type_keys(&mut t, &[], "q");
        assert_eq!(tb(&e).internal_text(), "q");
        run(&mut t, "b.javaObject:pasteFromClipboard()");
        assert_eq!(tb(&e).internal_text(), "qXb");
        assert_eq!(tb(&e).cursor_pos(), 3);
        run(
            &mut t,
            "b.javaObject:selectAll(); b.javaObject:cutToClipboard()",
        );
        assert_eq!(tb(&e).internal_text(), "");
        assert_eq!(CLIPBOARD.with(|c| c.borrow().clone()), "qXb");
        // Enter on a single line: onCommandEntered; Escape: onOtherKey 1.
        type_keys(&mut t, &[], "go");
        log(&mut t);
        type_keys(&mut t, &[28, 1], "");
        assert_eq!(log(&mut t), ["command go", "other 1"]);
    }

    #[test]
    fn filters_mask_max_length_upper_case() {
        let mut t = setup();
        run(&mut t, "b = newBox(0, 0, 300, 30, ''); b.javaObject:focus(); b.javaObject:setOnlyNumbers(true)");
        let e = el(&mut t, "b");
        type_keys(&mut t, &[], "1a-2.x");
        assert_eq!(tb(&e).internal_text(), "1-2.");
        run(&mut t, "b.javaObject:SetText(''); b.javaObject:setOnlyNumbers(false); b.javaObject:setOnlyText(true)");
        type_keys(&mut t, &[], "a1B ");
        assert_eq!(tb(&e).internal_text(), "aB");
        run(&mut t, "b.javaObject:setMasked(true)");
        assert_eq!(eval(&mut t, "b.javaObject:getText()").as_str(), Some("**"));
        assert_eq!(
            eval(&mut t, "b.javaObject:getInternalText()").as_str(),
            Some("aB")
        );
        run(&mut t, "b.javaObject:setMasked(false); b.javaObject:setMaxTextLength(1); b.javaObject:setForceUpperCase(true)");
        UIManager::update(&mut t);
        assert_eq!(tb(&e).internal_text(), "A");
    }

    #[test]
    fn multiline_enter_and_wrapping() {
        let mut t = setup();
        run(&mut t, "b = newBox(0, 0, 100, 80, 'one'); b.javaObject:setMultipleLine(true); b.javaObject:setMaxLines(3); b.javaObject:focus()");
        let e = el(&mut t, "b");
        // The constructor leaves the cursor at 0; End moves after "one".
        type_keys(&mut t, &[207, 28], "two");
        assert_eq!(tb(&e).internal_text(), "one\ntwo");
        UIManager::update(&mut t);
        assert_eq!(tb(&e).lines(), ["one", "two"]);
        // Max lines reached: Enter no longer adds a line.
        type_keys(&mut t, &[28, 28], "");
        assert_eq!(tb(&e).internal_text(), "one\ntwo\n");
        // Words wrap at width - 2 * inset - 17 (100 - 10 - 17 = 73 px).
        run(&mut t, "b.javaObject:SetText('aaa bbb ccc ddd')");
        assert_eq!(tb(&e).lines(), ["aaa bbb", "ccc ddd"]);
        // Up goes to the same column on the line above.
        run(&mut t, "b.javaObject:setCursorLine(1)");
        tb(&e).st.borrow_mut().cursor = 10;
        type_keys(&mut t, &[200], "");
        assert_eq!(tb(&e).cursor_pos(), 2);
    }

    #[test]
    fn mouse_focus_clear_button_and_unfocus() {
        let mut t = setup();
        run(
            &mut t,
            "b = newBox(10, 10, 200, 30, 'abcdef'); b.javaObject:setClearButton(true)",
        );
        UIManager::update(&mut t);
        let e = el(&mut t, "b");
        // Button_Close.png stands in (no game textures in tests).
        tb(&e).st.borrow_mut().clear_button_texture =
            Some(crate::bind_render::textures().borrow_mut().white());
        // A click in the text focuses and places the cursor by x.
        // (x is local: the first glyph wider than x ends at character 4.)
        assert!(e.on_mouse_down(&mut t, 5.0 + 8.0 * 2.0 + 3.0, 10.0));
        assert!(is_doing_text_entry());
        assert_eq!(tb(&e).cursor_pos(), 3);
        // The clear button at the right edge clears the input.
        assert!(e.on_mouse_down(&mut t, 195.0, 10.0));
        assert_eq!(tb(&e).internal_text(), "");
        assert_eq!(log(&mut t), ["change "]);
        // A left press anywhere unfocuses the box (onLostFocus).
        with_ui(|m| {
            m.mouse.x = 600;
            m.mouse.y = 600;
            m.mouse.down[0] = true;
        });
        UIManager::update(&mut t);
        assert!(!is_doing_text_entry());
        assert!(current_text_entry_box().is_none());
        assert_eq!(log(&mut t), ["lost"]);
    }

    #[test]
    fn render_draws_text_cursor_and_placeholder() {
        let mut t = setup();
        run(
            &mut t,
            "b = newBox(10, 20, 200, 30, 'hi'); b.javaObject:setMasked(true)",
        );
        UIManager::update(&mut t);
        UIManager::render(&mut t);
        let texts: Vec<(String, f32, f32)> = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Text { text, x, y, .. } => Some((text.to_string(), *x, *y)),
                    _ => None,
                })
                .collect()
        });
        assert_eq!(texts, [("**".to_string(), 15.0, 25.0)]);
        // Focused: the 1 px cursor after the text (inset + width - 1).
        run(&mut t, "b.javaObject:focus(); b.javaObject:setCursorPos(2)");
        UIManager::render(&mut t);
        let cursor = with_ui(|m| {
            m.sprite.cmds.iter().any(|c| matches!(c, UiDrawCmd::Quad { x, w, .. } if *x == 10.0 + 5.0 + 15.0 && *w == 1.0))
        });
        assert!(cursor);
        // Empty and unfocused: the placeholder.
        run(&mut t, "b.javaObject:unfocus(); b.javaObject:SetText(''); b.javaObject:setPlaceholderText('Name')");
        UIManager::render(&mut t);
        let ph = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .any(|c| matches!(c, UiDrawCmd::Text { text, .. } if &**text == "Name"))
        });
        assert!(ph);
    }
}
