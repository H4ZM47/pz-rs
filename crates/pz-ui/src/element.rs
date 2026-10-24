//! `zombie.ui.UIElement`: the engine object behind every Lua UI widget.
//!
//! State lives in a `RefCell` (Lua holds the element as
//! `Value::Object(Rc<dyn LuaObject>)`). Methods that call into Lua take the
//! `&mut Thread` explicitly (the Java's `UIManager.getDefaultThread()`), and
//! never keep the state borrowed across a Lua call. Child lists are walked by
//! index and re-read each step, like the Java `ArrayList` loops, so callbacks
//! may add or remove children while a dispatch is running.

use crate::lua::{pcall_bool, pcall_void, tableget};
use crate::manager::{now_ms, with_ui, UIManager};
use pz_kahlua::{LuaObject, TableRef, Thread, Value};
use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::rc::{Rc, Weak};

/// Fields of the Java object (names follow the original).
#[derive(Clone)]
pub struct UIElementState {
    pub capture: bool,
    pub ignore_loss_control: bool,
    pub clicked_value: Option<Rc<str>>,
    pub controls: Vec<Rc<UIElement>>,
    pub default_draw: bool,
    pub follow_game_world: bool,
    pub render_this_player_only: i32,
    pub height: f32,
    pub parent: Weak<UIElement>,
    pub visible: bool,
    pub width: f32,
    pub x: f64,
    pub y: f64,
    pub table: Option<TableRef>,
    pub always_back: bool,
    pub scroll_children: bool,
    pub scroll_with_parent: bool,
    pub render_clipped_children: bool,
    pub anchor_top: bool,
    pub anchor_left: bool,
    pub anchor_right: bool,
    pub anchor_bottom: bool,
    pub player_context: i32,
    pub always_on_top: bool,
    pub max_draw_height: i32,
    pub y_scroll: f64,
    pub x_scroll: f64,
    pub scroll_height: i32,
    pub lastheight: f64,
    pub lastwidth: f64,
    pub resize_dirty: bool,
    pub enabled: bool,
    pub to_top: Vec<Rc<UIElement>>,
    pub consume_mouse_events: bool,
    pub left_down_time: i64,
    pub clicked: bool,
    pub click_x: f64,
    pub click_y: f64,
    pub uiname: String,
    pub want_key_events: bool,
    pub want_extra_mouse_events: bool,
    pub force_cursor_visible: bool,
    pub shader_started: bool,
}

impl Default for UIElementState {
    fn default() -> Self {
        UIElementState {
            capture: false,
            ignore_loss_control: false,
            clicked_value: None,
            controls: Vec::new(),
            default_draw: true,
            follow_game_world: false,
            render_this_player_only: -1,
            height: 256.0,
            parent: Weak::new(),
            visible: true,
            width: 256.0,
            x: 0.0,
            y: 0.0,
            table: None,
            always_back: false,
            scroll_children: false,
            scroll_with_parent: true,
            render_clipped_children: true,
            anchor_top: true,
            anchor_left: true,
            anchor_right: false,
            anchor_bottom: false,
            player_context: -1,
            always_on_top: false,
            max_draw_height: -1,
            y_scroll: 0.0,
            x_scroll: 0.0,
            scroll_height: 0,
            lastheight: -1.0,
            lastwidth: -1.0,
            resize_dirty: false,
            enabled: true,
            to_top: Vec::new(),
            consume_mouse_events: true,
            left_down_time: 0,
            clicked: false,
            click_x: 0.0,
            click_y: 0.0,
            uiname: String::new(),
            want_key_events: false,
            want_extra_mouse_events: false,
            force_cursor_visible: false,
            shader_started: false,
        }
    }
}

pub struct UIElement {
    me: Weak<UIElement>,
    state: RefCell<UIElementState>,
}

impl LuaObject for UIElement {
    fn class_name(&self) -> &str {
        "zombie.ui.UIElement"
    }
    /// Without a thread the table's `__tostring` cannot run; the Lua
    /// binding's `toString` uses `KahluaUtil.tostring` properly.
    fn to_lua_string(&self) -> String {
        match &self.st().table {
            Some(t) => match t.rawget(&Value::str("Type")) {
                Value::Str(s) => format!("{} 0x{}", s, pz_kahlua::exec::identity_hash(&Value::Table(t.clone()))),
                _ => format!("table 0x{}", pz_kahlua::exec::identity_hash(&Value::Table(t.clone()))),
            },
            None => format!("zombie.ui.UIElement@{:x}", self.hash_code()),
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn n(v: f64) -> Value {
    Value::Num(v)
}

fn same(a: &Rc<UIElement>, b: &UIElement) -> bool {
    std::ptr::eq(Rc::as_ptr(a), b)
}

impl UIElement {
    /// `new UIElement()` / `new UIElement(KahluaTable)`.
    pub fn new(table: Option<TableRef>) -> Rc<UIElement> {
        Rc::new_cyclic(|me| UIElement {
            me: me.clone(),
            state: RefCell::new(UIElementState { table, ..Default::default() }),
        })
    }

    /// The owning `Rc` of `self`.
    pub fn rc(&self) -> Rc<UIElement> {
        self.me.upgrade().expect("UIElement used after drop")
    }

    /// This element as a Lua value.
    pub fn to_value(&self) -> Value {
        Value::Object(self.rc())
    }

    pub fn st(&self) -> Ref<'_, UIElementState> {
        self.state.borrow()
    }
    pub fn st_mut(&self) -> RefMut<'_, UIElementState> {
        self.state.borrow_mut()
    }

    /// `Object.hashCode()` (identity).
    pub fn hash_code(&self) -> u32 {
        let p = self as *const Self as usize;
        (p ^ (p >> 32)) as u32
    }

    // ---- Lua callbacks ----

    fn lua_fn(&self, t: &mut Thread, name: &str) -> Option<(Value, TableRef)> {
        let table = self.st().table.clone()?;
        let f = tableget(t, &table, name)?;
        Some((f, table))
    }

    /// `protectedCallVoid(thread, tableget(table, name), table, args...)`
    /// when the function exists.
    fn call_void(&self, t: &mut Thread, name: &str, args: &[Value]) -> bool {
        match self.lua_fn(t, name) {
            Some((f, table)) => {
                let mut a = Vec::with_capacity(args.len() + 1);
                a.push(Value::Table(table));
                a.extend_from_slice(args);
                pcall_void(t, &f, &a);
                true
            }
            None => false,
        }
    }

    /// `protectedCallBoolean`; `None` when the function does not exist.
    fn call_bool(&self, t: &mut Thread, name: &str, args: &[Value]) -> Option<Option<bool>> {
        let (f, table) = self.lua_fn(t, name)?;
        let mut a = Vec::with_capacity(args.len() + 1);
        a.push(Value::Table(table));
        a.extend_from_slice(args);
        Some(pcall_bool(t, &f, &a))
    }

    fn has_fn(&self, t: &mut Thread, name: &str) -> bool {
        self.lua_fn(t, name).is_some()
    }

    // ---- children ----

    pub fn controls_len(&self) -> usize {
        self.st().controls.len()
    }
    pub fn control(&self, i: usize) -> Option<Rc<UIElement>> {
        self.st().controls.get(i).cloned()
    }
    /// `getControls()` snapshot.
    pub fn get_controls(&self) -> Vec<Rc<UIElement>> {
        self.st().controls.clone()
    }
    fn remove_control_ref(&self, el: &UIElement) {
        let mut s = self.st_mut();
        if let Some(i) = s.controls.iter().position(|c| same(c, el)) {
            s.controls.remove(i);
        }
    }

    /// `AddChild(el)`.
    pub fn add_child(&self, el: &UIElement) {
        self.st_mut().controls.push(el.rc());
        el.set_parent(Some(self));
    }
    /// `RemoveChild(el)`.
    pub fn remove_child(&self, el: &UIElement) {
        self.remove_control_ref(el);
        el.set_parent(None);
    }
    /// `RemoveControl(el)`.
    pub fn remove_control(&self, el: &UIElement) {
        self.remove_child(el);
    }
    /// `ClearChildren()`: the children keep their parent pointer, as in Java.
    pub fn clear_children(&self) {
        self.st_mut().controls.clear();
    }
    /// `BringToTop(el)`: move a child to the end of the draw order now.
    pub fn bring_child_to_top(&self, el: &UIElement) {
        self.remove_control_ref(el);
        self.st_mut().controls.push(el.rc());
    }
    /// `bringToTop()`: queued, applied by the next `update`.
    pub fn bring_to_top(&self) {
        UIManager::push_to_top(self.rc());
        if let Some(p) = self.get_parent() {
            p.st_mut().to_top.push(self.rc());
        }
    }

    pub fn get_parent(&self) -> Option<Rc<UIElement>> {
        self.st().parent.upgrade()
    }
    pub fn set_parent(&self, parent: Option<&UIElement>) {
        self.st_mut().parent = parent.map(|p| p.me.clone()).unwrap_or_default();
    }

    pub fn get_table(&self) -> Option<TableRef> {
        self.st().table.clone()
    }
    pub fn set_table(&self, t: Option<TableRef>) {
        self.st_mut().table = t;
    }

    // ---- simple properties ----

    /// `getMaxDrawHeight()`.
    pub fn get_max_draw_height(&self) -> f64 {
        self.st().max_draw_height as f64
    }
    pub fn set_max_draw_height(&self, h: f64) {
        self.st_mut().max_draw_height = h as i32;
    }
    pub fn clear_max_draw_height(&self) {
        self.st_mut().max_draw_height = -1;
    }
    pub fn get_x_scroll(&self) -> f64 {
        self.st().x_scroll
    }
    pub fn get_y_scroll(&self) -> f64 {
        self.st().y_scroll
    }
    pub fn set_x_scroll(&self, t: &mut Thread, x: f64) {
        if self.st().x_scroll != x {
            self.st_mut().x_scroll = x;
            self.invoke_on_scroll_pos_changed(t);
        }
    }
    pub fn set_y_scroll(&self, t: &mut Thread, y: f64) {
        if self.st().y_scroll != y {
            self.st_mut().y_scroll = y;
            self.invoke_on_scroll_pos_changed(t);
        }
    }
    fn invoke_on_scroll_pos_changed(&self, t: &mut Thread) {
        let (xs, ys) = (self.get_x_scroll(), self.get_y_scroll());
        self.call_void(t, "onScrollPosChanged", &[n(xs), n(ys)]);
    }
    pub fn set_always_on_top(&self, b: bool) {
        self.st_mut().always_on_top = b;
    }
    pub fn is_always_on_top(&self) -> bool {
        self.st().always_on_top
    }
    pub fn back_most(&self) {
        self.st_mut().always_back = true;
    }
    pub fn is_back_most(&self) -> bool {
        self.st().always_back
    }
    pub fn get_scroll_height(&self) -> f64 {
        self.st().scroll_height as f64
    }
    pub fn set_scroll_height(&self, h: f64) {
        self.st_mut().scroll_height = h as i32;
    }
    pub fn is_consume_mouse_events(&self) -> bool {
        self.st().consume_mouse_events
    }
    pub fn set_consume_mouse_events(&self, b: bool) {
        self.st_mut().consume_mouse_events = b;
    }
    /// `ButtonClicked(name)`.
    pub fn button_clicked(&self, name: Option<Rc<str>>) {
        self.set_clicked_value(name);
    }
    pub fn get_clicked_value(&self) -> Option<Rc<str>> {
        self.st().clicked_value.clone()
    }
    pub fn set_clicked_value(&self, v: Option<Rc<str>>) {
        self.st_mut().clicked_value = v;
    }
    pub fn get_scroll_children(&self) -> bool {
        self.st().scroll_children
    }
    pub fn set_scroll_children(&self, b: bool) {
        self.st_mut().scroll_children = b;
    }
    pub fn get_scroll_with_parent(&self) -> bool {
        self.st().scroll_with_parent
    }
    pub fn set_scroll_with_parent(&self, b: bool) {
        self.st_mut().scroll_with_parent = b;
    }
    pub fn set_render_clipped_children(&self, b: bool) {
        self.st_mut().render_clipped_children = b;
    }
    pub fn is_capture(&self) -> bool {
        self.st().capture
    }
    pub fn set_capture(&self, b: bool) {
        self.st_mut().capture = b;
    }
    pub fn is_ignore_loss_control(&self) -> bool {
        self.st().ignore_loss_control
    }
    pub fn set_ignore_loss_control(&self, b: bool) {
        self.st_mut().ignore_loss_control = b;
    }
    pub fn is_default_draw(&self) -> bool {
        self.st().default_draw
    }
    pub fn set_default_draw(&self, b: bool) {
        self.st_mut().default_draw = b;
    }
    pub fn is_follow_game_world(&self) -> bool {
        self.st().follow_game_world
    }
    pub fn set_follow_game_world(&self, b: bool) {
        self.st_mut().follow_game_world = b;
    }
    pub fn get_render_this_player_only(&self) -> i32 {
        self.st().render_this_player_only
    }
    pub fn set_render_this_player_only(&self, i: i32) {
        self.st_mut().render_this_player_only = i;
    }
    pub fn get_height(&self) -> f64 {
        self.st().height as f64
    }
    pub fn get_width(&self) -> f64 {
        self.st().width as f64
    }
    pub fn set_height(&self, h: f64) {
        let mut s = self.st_mut();
        if s.height as f64 != h {
            s.resize_dirty = true;
        }
        s.lastheight = s.height as f64;
        s.height = h as f32;
    }
    pub fn set_width(&self, w: f64) {
        let mut s = self.st_mut();
        if s.width as f64 != w {
            s.resize_dirty = true;
        }
        s.lastwidth = s.width as f64;
        s.width = w as f32;
    }
    pub fn set_height_silent(&self, h: f64) {
        let mut s = self.st_mut();
        s.lastheight = s.height as f64;
        s.height = h as f32;
    }
    pub fn set_width_silent(&self, w: f64) {
        let mut s = self.st_mut();
        s.lastwidth = s.width as f64;
        s.width = w as f32;
    }
    pub fn set_height_only(&self, h: f64) {
        self.st_mut().height = h as f32;
    }
    pub fn set_width_only(&self, w: f64) {
        self.st_mut().width = w as f32;
    }
    pub fn is_visible(&self) -> bool {
        self.st().visible
    }
    pub fn set_visible(&self, b: bool) {
        self.st_mut().visible = b;
    }
    pub fn get_x(&self) -> f64 {
        self.st().x
    }
    pub fn get_y(&self) -> f64 {
        self.st().y
    }
    /// `setX(double)` stores `(float)x` into the double field.
    pub fn set_x(&self, x: f64) {
        self.st_mut().x = x as f32 as f64;
    }
    pub fn set_y(&self, y: f64) {
        self.st_mut().y = y as f32 as f64;
    }
    pub fn is_enabled(&self) -> bool {
        self.st().enabled
    }
    pub fn set_enabled(&self, b: bool) {
        self.st_mut().enabled = b;
    }
    pub fn is_anchor_top(&self) -> bool {
        self.st().anchor_top
    }
    pub fn is_anchor_left(&self) -> bool {
        self.st().anchor_left
    }
    pub fn is_anchor_right(&self) -> bool {
        self.st().anchor_right
    }
    pub fn is_anchor_bottom(&self) -> bool {
        self.st().anchor_bottom
    }
    fn set_anchor(&self, f: impl FnOnce(&mut UIElementState)) {
        let mut s = self.st_mut();
        f(&mut s);
        s.lastwidth = s.width as f64;
        s.lastheight = s.height as f64;
    }
    pub fn set_anchor_top(&self, b: bool) {
        self.set_anchor(|s| s.anchor_top = b);
    }
    pub fn set_anchor_left(&self, b: bool) {
        self.set_anchor(|s| s.anchor_left = b);
    }
    pub fn set_anchor_right(&self, b: bool) {
        self.set_anchor(|s| s.anchor_right = b);
    }
    pub fn set_anchor_bottom(&self, b: bool) {
        self.set_anchor(|s| s.anchor_bottom = b);
    }
    pub fn ignore_width_change(&self) {
        let mut s = self.st_mut();
        s.lastwidth = s.width as f64;
    }
    pub fn ignore_height_change(&self) {
        let mut s = self.st_mut();
        s.lastheight = s.height as f64;
    }
    pub fn get_player_context(&self) -> i32 {
        self.st().player_context
    }
    pub fn set_player_context(&self, i: i32) {
        self.st_mut().player_context = i;
    }
    pub fn set_want_key_events(&self, b: bool) {
        self.st_mut().want_key_events = b;
    }
    pub fn is_want_key_events(&self) -> bool {
        self.st().want_key_events
    }
    pub fn set_want_extra_mouse_events(&self, b: bool) {
        self.st_mut().want_extra_mouse_events = b;
    }
    pub fn is_want_extra_mouse_events(&self) -> bool {
        self.st().want_extra_mouse_events
    }
    pub fn is_force_cursor_visible(&self) -> bool {
        self.st().force_cursor_visible
    }
    pub fn set_force_cursor_visible(&self, b: bool) {
        self.st_mut().force_cursor_visible = b;
    }
    pub fn set_ui_name(&self, name: Option<&str>) {
        self.st_mut().uiname = name.unwrap_or("").to_string();
    }

    /// `getUIName()`.
    pub fn get_ui_name(&self) -> String {
        let s = self.st();
        if !s.uiname.is_empty() {
            return s.uiname.clone();
        }
        // TODO: `TextBox` subclass ("Text: " + text) once ported.
        if let Some(t) = &s.table {
            let name = t.rawget(&Value::str("name"));
            let ty = t.rawget(&Value::str("Type"));
            if !ty.is_nil() {
                if !name.is_nil() {
                    return format!("{}: {}", pz_kahlua::exec::java_value_of(&ty), pz_kahlua::exec::java_value_of(&name));
                }
                return pz_kahlua::exec::java_value_of(&ty);
            }
        }
        format!("UI: {}", self.hash_code() as i32)
    }

    // ---- geometry ----

    /// `getAbsoluteX()`: integer-truncated positions up the parent chain.
    pub fn get_absolute_x(&self) -> f64 {
        let (x, parent, swp) = {
            let s = self.st();
            (s.x, s.parent.upgrade(), s.scroll_with_parent)
        };
        match parent {
            Some(p) => {
                if p.get_scroll_children() && swp {
                    p.get_absolute_x() + (x as i32) as f64 + (p.get_x_scroll() as i32) as f64
                } else {
                    p.get_absolute_x() + (x as i32) as f64
                }
            }
            None => (x as i32) as f64,
        }
    }

    pub fn get_absolute_y(&self) -> f64 {
        let (y, parent, swp) = {
            let s = self.st();
            (s.y, s.parent.upgrade(), s.scroll_with_parent)
        };
        match parent {
            Some(p) => {
                if p.get_scroll_children() && swp {
                    p.get_absolute_y() + (y as i32) as f64 + (p.get_y_scroll() as i32) as f64
                } else {
                    p.get_absolute_y() + (y as i32) as f64
                }
            }
            None => (y as i32) as f64,
        }
    }

    /// `getXScrolled(parent)`.
    pub fn get_x_scrolled(&self, parent: Option<&UIElement>) -> f64 {
        let s = self.st();
        match parent {
            Some(p) if p.st().scroll_children && s.scroll_with_parent => s.x + p.get_x_scroll(),
            _ => s.x,
        }
    }

    pub fn get_y_scrolled(&self, parent: Option<&UIElement>) -> f64 {
        let s = self.st();
        match parent {
            Some(p) if p.st().scroll_children && s.scroll_with_parent => s.y + p.get_y_scroll(),
            _ => s.y,
        }
    }

    /// `isOverElement(mx, my)` in parent coordinates.
    pub fn is_over_element(&self, mx: f64, my: f64) -> bool {
        let s = self.st();
        mx >= s.x && my >= s.y && mx < s.x + s.width as f64 && my < s.y + s.height as f64
    }

    pub fn clamp_to_parent_x(&self, x: f64) -> f64 {
        let Some(p) = self.get_parent() else { return x };
        let x1 = p.clamp_to_parent_x(p.get_absolute_x());
        let x2 = p.clamp_to_parent_x(x1 + (p.get_width() as i32) as f64);
        let mut x = x;
        if x < x1 {
            x = x1;
        }
        if x > x2 {
            x = x2;
        }
        x
    }

    pub fn clamp_to_parent_y(&self, y: f64) -> f64 {
        let Some(p) = self.get_parent() else { return y };
        let y1 = p.clamp_to_parent_y(p.get_absolute_y());
        let y2 = p.clamp_to_parent_y(y1 + (p.get_height() as i32) as f64);
        let mut y = y;
        if y < y1 {
            y = y1;
        }
        if y > y2 {
            y = y2;
        }
        y
    }

    /// `isReallyVisible()`.
    pub fn is_really_visible(&self) -> bool {
        if !self.is_visible() {
            return false;
        }
        match self.get_parent() {
            Some(p) => {
                if !p.st().controls.iter().any(|c| same(c, self)) {
                    return false;
                }
                p.is_really_visible()
            }
            None => with_ui(|m| m.ui.iter().any(|c| same(c, self))),
        }
    }

    /// `isModalVisible()`.
    pub fn is_modal_visible(&self) -> bool {
        if !self.is_really_visible() {
            return false;
        }
        if self.is_capture() {
            return true;
        }
        let mut i = 0;
        while let Some(c) = self.control(i) {
            if c.is_modal_visible() {
                return true;
            }
            i += 1;
        }
        false
    }

    fn local_hit(&self, sx: f64, sy: f64) -> bool {
        let (mut h, mdh, w) = {
            let s = self.st();
            (s.height as i32, s.max_draw_height, s.width as f64)
        };
        if mdh != -1 {
            h = h.min(mdh);
        }
        let lx = sx - self.get_absolute_x();
        let ly = sy - self.get_absolute_y();
        lx >= 0.0 && lx < w && ly >= 0.0 && ly < h as f64
    }

    /// `isPointOver(screenX, screenY)`: inside this element and not covered
    /// by a later sibling (or later top-level element).
    pub fn is_point_over(&self, sx: f64, sy: f64) -> bool {
        if !self.is_visible() {
            return false;
        }
        if !self.local_hit(sx, sy) {
            return false;
        }
        match self.get_parent() {
            None => {
                let uis = with_ui(|m| m.ui.clone());
                for other in uis.iter().rev() {
                    if same(other, self) {
                        break;
                    }
                    if other.is_point_over(sx, sy) {
                        return false;
                    }
                }
                true
            }
            Some(p) => {
                let sibs = p.get_controls();
                for other in sibs.iter().rev() {
                    if same(other, self) {
                        break;
                    }
                    if !other.is_visible() || !other.is_consume_mouse_events() {
                        continue;
                    }
                    if other.local_hit(sx, sy) {
                        return false;
                    }
                }
                p.is_point_over(sx, sy)
            }
        }
    }

    pub fn is_mouse_over(&self) -> bool {
        let (mx, my) = with_ui(|m| (m.mouse.x, m.mouse.y));
        self.is_point_over(mx as f64, my as f64)
    }

    fn parent_clips(&self, y: f64) -> bool {
        if let Some(p) = self.get_parent() {
            let mdh = p.st().max_draw_height;
            if mdh != -1 && mdh as f64 <= y {
                return true;
            }
        }
        false
    }

    fn self_clips(&self, y: f64) -> bool {
        let mdh = self.st().max_draw_height;
        mdh != -1 && mdh as f64 <= y
    }

    fn scroll(&self) -> (f64, f64) {
        let s = self.st();
        (s.x_scroll, s.y_scroll)
    }

    fn child_hit(&self, ui: &UIElement, x: f64, y: f64, strict: bool) -> bool {
        let xs = ui.get_x_scrolled(Some(self));
        let ys = ui.get_y_scrolled(Some(self));
        let (w, h) = (ui.get_width(), ui.get_height());
        let inside_lo = if strict { x > xs && y > ys } else { x >= xs && y >= ys };
        inside_lo && x < xs + w && y < ys + h
    }

    fn child_local(&self, ui: &UIElement, x: f64, y: f64) -> (f64, f64) {
        (x - (ui.get_x_scrolled(Some(self)) as i32) as f64, y - (ui.get_y_scrolled(Some(self)) as i32) as f64)
    }

    fn abs_hit(ui: &UIElement, mx: f64, my: f64) -> bool {
        let ax = ui.get_absolute_x();
        let ay = ui.get_absolute_y();
        mx >= ax && my >= ay && mx < ax + ui.get_width() && my < ay + ui.get_height()
    }

    // ---- mouse ----

    fn outside_recursive(&self, t: &mut Thread, name: &str, x: f64, y: f64, use_scrolled: bool, f: fn(&UIElement, &mut Thread, f64, f64)) {
        let (xs, ys) = self.scroll();
        self.call_void(t, name, &[n(x - xs), n(y - ys)]);
        let mut i = self.controls_len() as isize - 1;
        while i >= 0 {
            if let Some(ui) = self.control(i as usize) {
                let (lx, ly) = if use_scrolled {
                    self.child_local(&ui, x, y)
                } else {
                    (x - (ui.get_x() as i32) as f64, y - (ui.get_y() as i32) as f64)
                };
                f(&ui, t, lx, ly);
            }
            i -= 1;
        }
    }

    pub fn on_right_mouse_up_outside(&self, t: &mut Thread, x: f64, y: f64) {
        self.outside_recursive(t, "onRightMouseUpOutside", x, y, true, UIElement::on_right_mouse_up_outside);
    }
    pub fn on_right_mouse_down_outside(&self, t: &mut Thread, x: f64, y: f64) {
        self.outside_recursive(t, "onRightMouseDownOutside", x, y, true, UIElement::on_right_mouse_down_outside);
    }
    pub fn on_mouse_up_outside(&self, t: &mut Thread, x: f64, y: f64) {
        self.outside_recursive(t, "onMouseUpOutside", x, y, true, UIElement::on_mouse_up_outside);
    }
    /// Children get `x - (int)ui.getX()` here (not the scrolled position),
    /// as in the original.
    pub fn on_mouse_down_outside(&self, t: &mut Thread, x: f64, y: f64) {
        self.outside_recursive(t, "onMouseDownOutside", x, y, false, UIElement::on_mouse_down_outside);
    }

    /// Shared body of `onMouseDown` / `onRightMouseDown` (and the up
    /// variants without the outside-callback difference).
    #[allow(clippy::too_many_arguments)]
    fn dispatch_button(
        &self,
        t: &mut Thread,
        x: f64,
        y: f64,
        strict: bool,
        child: fn(&UIElement, &mut Thread, f64, f64) -> bool,
        child_outside: &dyn Fn(&UIElement, &UIElement, &mut Thread, f64, f64),
        outside_name: &str,
        name: &str,
        nil_consumes: bool,
    ) -> bool {
        let mut force_outside = false;
        let mut i = self.controls_len() as isize - 1;
        while i >= 0 {
            let Some(ui) = self.control(i as usize) else {
                i = (self.controls_len() as isize).min(i) - 1;
                continue;
            };
            if !force_outside && (self.child_hit(&ui, x, y, strict) || ui.is_capture()) {
                let (lx, ly) = self.child_local(&ui, x, y);
                if child(&ui, t, lx, ly) {
                    force_outside = true;
                }
            } else {
                child_outside(self, &ui, t, x, y);
            }
            i = (i).min(self.controls_len() as isize) - 1;
        }
        let (xs, ys) = self.scroll();
        if self.get_table().is_some() {
            if force_outside {
                if let Some(o) = self.call_bool(t, outside_name, &[n(x - xs), n(y - ys)]) {
                    if o.unwrap_or(true) {
                        return true;
                    }
                }
            } else if let Some(o) = self.call_bool(t, name, &[n(x - xs), n(y - ys)]) {
                match o {
                    None => return nil_consumes || self.is_consume_mouse_events(),
                    Some(true) => return true,
                    Some(false) => {}
                }
            }
        }
        force_outside
    }

    /// `onMouseDown(x, y)` in local coordinates; returns whether consumed.
    pub fn on_mouse_down(&self, t: &mut Thread, x: f64, y: f64) -> bool {
        let (clicked, cx, cy, ldt, table) = {
            let s = self.st();
            (s.clicked, s.click_x, s.click_y, s.left_down_time, s.table.clone())
        };
        if clicked
            && UIManager::is_double_click((cx as i32) as f64, (cy as i32) as f64, (x as i32) as f64, (y as i32) as f64, ldt as f64)
            && table.as_ref().is_some_and(|t| !t.rawget(&Value::str("onMouseDoubleClick")).is_nil())
        {
            self.st_mut().clicked = false;
            return self.on_mouse_double_click(t, x, y);
        }
        {
            let mut s = self.st_mut();
            s.clicked = true;
            s.click_x = x;
            s.click_y = y;
            s.left_down_time = now_ms();
        }
        if self.parent_clips(y) || self.self_clips(y) || !self.is_visible() {
            return false;
        }
        let (xs, ys) = self.scroll();
        self.call_void(t, "onFocus", &[n(x - xs), n(y - ys)]);
        // Children that miss get `onMouseDownOutside` with the parent's
        // coordinates (not their own), as in the original; non-recursive.
        let outside = |me: &UIElement, ui: &UIElement, t: &mut Thread, x: f64, y: f64| {
            let (xs, ys) = me.scroll();
            ui.call_void(t, "onMouseDownOutside", &[n(x - xs), n(y - ys)]);
        };
        // Down uses strict `>` on the low edges; the Java's `onMouseDown`
        // falls back to `consumeMouseEvents` for a nil result.
        self.dispatch_button(t, x, y, true, UIElement::on_mouse_down, &outside, "onMouseDownOutside", "onMouseDown", false)
    }

    fn on_mouse_double_click(&self, t: &mut Thread, x: f64, y: f64) -> bool {
        let sy = self.get_y();
        if self.parent_clips(sy) || self.self_clips(sy) || !self.is_visible() {
            return false;
        }
        let (xs, ys) = self.scroll();
        self.call_bool(t, "onMouseDoubleClick", &[n(x - xs), n(y - ys)]);
        true
    }

    /// `onMouseUp(x, y)`.
    pub fn on_mouse_up(&self, t: &mut Thread, x: f64, y: f64) -> bool {
        if self.parent_clips(y) || self.self_clips(y) || !self.is_visible() {
            return false;
        }
        let outside = |me: &UIElement, ui: &UIElement, t: &mut Thread, x: f64, y: f64| {
            let (lx, ly) = me.child_local(ui, x, y);
            ui.on_mouse_up_outside(t, lx, ly);
        };
        self.dispatch_button(t, x, y, false, UIElement::on_mouse_up, &outside, "onMouseUpOutside", "onMouseUp", false)
    }

    /// `onRightMouseDown(x, y)`: a nil Lua result consumes.
    pub fn on_right_mouse_down(&self, t: &mut Thread, x: f64, y: f64) -> bool {
        if !self.is_visible() || self.parent_clips(y) || self.self_clips(y) {
            return false;
        }
        let outside = |me: &UIElement, ui: &UIElement, t: &mut Thread, x: f64, y: f64| {
            let (xs, ys) = me.scroll();
            ui.call_void(t, "onRightMouseDownOutside", &[n(x - xs), n(y - ys)]);
        };
        self.dispatch_button(t, x, y, false, UIElement::on_right_mouse_down, &outside, "onRightMouseDownOutside", "onRightMouseDown", true)
    }

    /// `onRightMouseUp(x, y)`: a nil Lua result consumes.
    pub fn on_right_mouse_up(&self, t: &mut Thread, x: f64, y: f64) -> bool {
        if !self.is_visible() || self.parent_clips(y) || self.self_clips(y) {
            return false;
        }
        let outside = |me: &UIElement, ui: &UIElement, t: &mut Thread, x: f64, y: f64| {
            let (lx, ly) = me.child_local(ui, x, y);
            ui.on_right_mouse_up_outside(t, lx, ly);
        };
        self.dispatch_button(t, x, y, false, UIElement::on_right_mouse_up, &outside, "onRightMouseUpOutside", "onRightMouseUp", true)
    }

    /// `onMouseButtonDown(btn, x, y)` for buttons other than left/right.
    pub fn on_mouse_button_down(&self, t: &mut Thread, btn: i32, _x: f64, _y: f64) {
        if !self.is_want_extra_mouse_events() || !self.has_fn(t, "onMouseButtonDown") {
            return;
        }
        let mut i = self.controls_len() as isize - 1;
        while i >= 0 {
            if let Some(c) = self.control(i as usize) {
                if c.is_mouse_over() {
                    return;
                }
            }
            i -= 1;
        }
        self.call_void(t, "onMouseButtonDown", &[n(btn as f64)]);
    }

    pub fn on_consume_mouse_button_down(&self, t: &mut Thread, btn: i32, x: f64, y: f64) -> bool {
        match btn {
            0 => self.on_mouse_down(t, x, y),
            1 => self.on_right_mouse_down(t, x, y),
            _ => {
                self.on_mouse_button_down(t, btn, x, y);
                false
            }
        }
    }

    pub fn on_mouse_button_down_outside(&self, t: &mut Thread, btn: i32, x: f64, y: f64) {
        match btn {
            0 => self.on_mouse_down_outside(t, x, y),
            1 => self.on_right_mouse_down_outside(t, x, y),
            _ => {}
        }
    }

    pub fn on_consume_mouse_button_up(&self, t: &mut Thread, btn: i32, x: f64, y: f64) -> bool {
        match btn {
            0 => self.on_mouse_up(t, x, y),
            1 => self.on_right_mouse_up(t, x, y),
            _ => false,
        }
    }

    pub fn on_mouse_button_up_outside(&self, t: &mut Thread, btn: i32, x: f64, y: f64) {
        match btn {
            0 => self.on_mouse_up_outside(t, x, y),
            1 => self.on_right_mouse_up_outside(t, x, y),
            _ => {}
        }
    }

    /// `onConsumeMouseWheel(del, x, y)`.
    pub fn on_consume_mouse_wheel(&self, t: &mut Thread, del: f64, _x: f64, _y: f64) -> bool {
        if !self.is_ignore_loss_control() && with_ui(|m| m.steal_control) {
            return false;
        }
        if !self.is_visible() {
            return false;
        }
        self.on_mouse_wheel(t, del)
    }

    /// `onMouseWheel(del)`: children under the mouse first.
    pub fn on_mouse_wheel(&self, t: &mut Thread, del: f64) -> bool {
        let (mx, my) = with_ui(|m| (m.mouse.x as f64, m.mouse.y as f64));
        let mut i = self.controls_len() as isize - 1;
        while i >= 0 {
            if let Some(ui) = self.control(i as usize) {
                if ui.is_visible() && (UIElement::abs_hit(&ui, mx, my) || ui.is_capture()) && ui.on_mouse_wheel(t, del) {
                    return self.is_consume_mouse_events();
                }
            }
            i -= 1;
        }
        matches!(self.call_bool(t, "onMouseWheel", &[n(del)]), Some(Some(true)))
    }

    pub fn on_consume_mouse_move(&self, t: &mut Thread, dx: f64, dy: f64, _x: f64, _y: f64) -> bool {
        self.on_mouse_move(t, dx, dy)
    }

    /// `onMouseMove(dx, dy)`: the element's own callback runs before its
    /// children's.
    pub fn on_mouse_move(&self, t: &mut Thread, dx: f64, dy: f64) -> bool {
        let (mx, my) = with_ui(|m| (m.mouse.x as f64, m.mouse.y as f64));
        let sy = self.get_y();
        if self.parent_clips(sy) {
            return false;
        }
        let mdh = self.st().max_draw_height;
        if mdh != -1 && mdh as f64 <= my - self.get_absolute_y() {
            return false;
        }
        if !self.is_visible() {
            return false;
        }
        self.call_void(t, "onMouseMove", &[n(dx), n(dy)]);
        let mut consumed = false;
        let mut i = self.controls_len() as isize - 1;
        while i >= 0 {
            if let Some(ui) = self.control(i as usize) {
                if ui.is_visible() {
                    if UIElement::abs_hit(&ui, mx, my) || ui.is_capture() {
                        if (!consumed || ui.is_capture()) && ui.on_mouse_move(t, dx, dy) {
                            consumed = true;
                        }
                    } else {
                        ui.on_mouse_move_outside(t, dx, dy);
                    }
                }
            }
            i -= 1;
        }
        consumed || self.is_consume_mouse_events()
    }

    pub fn on_extend_mouse_move_outside(&self, t: &mut Thread, dx: f64, dy: f64, _x: f64, _y: f64) {
        self.on_mouse_move_outside(t, dx, dy);
    }

    pub fn on_mouse_move_outside(&self, t: &mut Thread, dx: f64, dy: f64) {
        self.call_void(t, "onMouseMoveOutside", &[n(dx), n(dy)]);
        let mut i = self.controls_len() as isize - 1;
        while i >= 0 {
            if let Some(ui) = self.control(i as usize) {
                if ui.is_visible() {
                    ui.on_mouse_move_outside(t, dx, dy);
                }
            }
            i -= 1;
        }
    }

    // ---- resize / render / update ----

    /// `onresize()` (empty in the original).
    pub fn onresize(&self) {}

    /// `onResize()`: apply anchors against the parent's size change, call
    /// Lua `onResize(self, w, h)`, recurse.
    pub fn on_resize(&self, t: &mut Thread) {
        if let Some(p) = self.get_parent() {
            let (dirty, difx, dify) = {
                let ps = p.st();
                (ps.resize_dirty, ps.width as f64 - ps.lastwidth, ps.height as f64 - ps.lastheight)
            };
            if dirty {
                let (at, ab, al, ar) = {
                    let s = self.st();
                    (s.anchor_top, s.anchor_bottom, s.anchor_left, s.anchor_right)
                };
                if !at && ab {
                    self.set_y(self.get_y() + dify);
                }
                if at && ab {
                    self.set_height(self.get_height() + dify);
                }
                if !al && ar {
                    self.set_x(self.get_x() + difx);
                }
                if al && ar {
                    self.set_width(self.get_width() + difx);
                }
            }
        }
        let (w, h) = (self.get_width(), self.get_height());
        self.call_void(t, "onResize", &[n(w), n(h)]);
        let mut i = self.controls_len() as isize - 1;
        while i >= 0 {
            if let Some(ui) = self.control(i as usize) {
                ui.on_resize(t);
            }
            i -= 1;
        }
        let mut s = self.st_mut();
        s.resize_dirty = false;
        s.lastwidth = s.width as f64;
        s.lastheight = s.height as f64;
    }

    /// `render()`: Lua `prerender`, children in order, Lua `render`.
    pub fn render(&self, t: &mut Thread) {
        let (enabled, visible, y) = {
            let s = self.st();
            (s.enabled, s.visible, s.y)
        };
        if !enabled || !visible || self.parent_clips(y) {
            return;
        }
        if let Some(p) = self.get_parent() {
            if !p.st().render_clipped_children {
                let pay = p.get_absolute_y();
                let ay = self.get_absolute_y();
                if ay + self.get_height() <= pay || ay >= pay + p.get_height() {
                    return;
                }
            }
        }
        self.call_void(t, "prerender", &[]);
        let mut i = 0;
        while let Some(c) = self.control(i) {
            c.render(t);
            i += 1;
        }
        self.call_void(t, "render", &[]);
        // TODO(debug): Core.debug && DebugOptions.uiRenderOutline outline.
    }

    /// `update()`: apply queued `bringToTop`s, Lua `update` (only on UI
    /// ticks, every 100 ms), pending resize, then children.
    pub fn update(&self, t: &mut Thread) {
        if !self.is_enabled() {
            return;
        }
        {
            let mut s = self.st_mut();
            let to_top = std::mem::take(&mut s.to_top);
            if !to_top.is_empty() {
                let mut moved = Vec::new();
                let mut i = 0;
                while i < s.controls.len() {
                    if to_top.iter().any(|e| Rc::ptr_eq(e, &s.controls[i])) {
                        moved.push(s.controls.remove(i));
                    } else {
                        i += 1;
                    }
                }
                s.controls.extend(moved);
            }
        }
        if with_ui(|m| m.do_tick) {
            self.call_void(t, "update", &[]);
        }
        if self.st().resize_dirty {
            self.on_resize(t);
            let mut s = self.st_mut();
            s.lastwidth = s.width as f64;
            s.lastheight = s.height as f64;
            s.resize_dirty = false;
        }
        let mut i = 0;
        while let Some(c) = self.control(i) {
            c.update(t);
            i += 1;
        }
    }

    // ---- keys ----

    pub fn is_key_consumed(&self, t: &mut Thread, key: i32) -> bool {
        matches!(self.call_bool(t, "isKeyConsumed", &[n(key as f64)]), Some(Some(true)))
    }
    pub fn on_consume_key_press(&self, t: &mut Thread, key: i32) -> bool {
        self.on_key_press(t, key);
        self.is_key_consumed(t, key)
    }
    pub fn on_key_press(&self, t: &mut Thread, key: i32) {
        self.call_void(t, "onKeyPress", &[n(key as f64)]);
    }
    pub fn on_consume_key_repeat(&self, t: &mut Thread, key: i32) -> bool {
        self.on_key_repeat(t, key);
        self.is_key_consumed(t, key)
    }
    pub fn on_key_repeat(&self, t: &mut Thread, key: i32) {
        self.call_void(t, "onKeyRepeat", &[n(key as f64)]);
    }
    pub fn on_consume_key_release(&self, t: &mut Thread, key: i32) -> bool {
        self.on_key_release(t, key);
        self.is_key_consumed(t, key)
    }
    pub fn on_key_release(&self, t: &mut Thread, key: i32) {
        self.call_void(t, "onKeyRelease", &[n(key as f64)]);
    }

    /// `StartOutline(tex, thickness, r, g, b, a)`.
    /// TODO(world): `IsoObject.OutlineShader`; no shader support yet.
    pub fn start_outline(&self) {}
    /// `EndOutline()`.
    pub fn end_outline(&self) {
        let started = self.st().shader_started;
        if started {
            // TODO(render): IndieGL.EndShader().
        }
    }
}
