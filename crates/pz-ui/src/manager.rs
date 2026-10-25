//! `zombie.ui.UIManager`: the global `UI` list, `AddUI`/`RemoveElement`,
//! the per-frame `update` (z-order fixes, mouse/wheel/move dispatch,
//! element updates) and `render`, key dispatch and the statics Lua uses.
//!
//! Java statics are a thread-local [`UIManager`] reached through
//! [`with_ui`]. Never call Lua inside `with_ui`: entry points copy what
//! they need out, release the borrow, then dispatch.
//!
//! The engine drives it each frame:
//! 1. fill [`UIManager::mouse`] (and call [`MouseState::end_frame`] after);
//! 2. [`UIManager::update`]`(thread)`;
//! 3. [`UIManager::render`]`(thread)`, then consume [`UIManager::sprite`].
//!
//! Parts tied to the game world (players, picking, the tooltip, clock,
//! speed controls, moodles, zoom, tutorial, Lua debugger) are stubbed with
//! `TODO(world)`.

use crate::element::UIElement;
use crate::sprite::SpriteRenderer;
use crate::text::TextRenderer;
use pz_kahlua::{Thread, Value};
use pz_render::Blend;
use std::cell::RefCell;
use std::rc::Rc;

/// `LuaEventManager.triggerEvent(name, args...)`, supplied by pz-lua-api.
pub type EventSink = Rc<dyn Fn(&mut Thread, &str, &[Value])>;

/// `UIManager.updateTooltip(mx, my)`, supplied by pz-lua-api.
pub type TooltipHook = Rc<dyn Fn(&mut Thread, i32, i32)>;

/// `Core.UnfocusActiveTextEntryBox()`, supplied by pz-lua-api.
pub type UnfocusHook = Rc<dyn Fn(&mut Thread)>;

/// `zombie.input.Mouse` as the UI reads it. The engine writes `x`, `y`,
/// `down` and `wheel` each frame and calls [`MouseState::end_frame`] after
/// the UI update.
#[derive(Clone, Debug)]
pub struct MouseState {
    /// `Mouse.getXA/getYA`: absolute window position.
    pub x: i32,
    pub y: i32,
    /// `Mouse.getX/getY`: position scaled to the world viewport.
    pub world_x: i32,
    pub world_y: i32,
    /// Button states this frame and last frame (`Mouse.getButtonCount()`
    /// is `down.len()`).
    pub down: Vec<bool>,
    pub prev_down: Vec<bool>,
    /// `Mouse.getWheelState()` (delta this frame).
    pub wheel: i32,
    /// `Mouse.UIBlockButtonDown(btn)`: the UI consumed the press.
    pub ui_blocked: Vec<bool>,
    /// `Mouse.isRightDelay()`.
    pub right_delay: bool,
}

impl Default for MouseState {
    fn default() -> Self {
        MouseState {
            x: 0,
            y: 0,
            world_x: 0,
            world_y: 0,
            down: vec![false; 5],
            prev_down: vec![false; 5],
            wheel: 0,
            ui_blocked: vec![false; 5],
            right_delay: false,
        }
    }
}

impl MouseState {
    pub fn button_count(&self) -> usize {
        self.down.len()
    }
    pub fn is_button_down(&self, b: usize) -> bool {
        self.down.get(b).copied().unwrap_or(false)
    }
    pub fn is_button_pressed(&self, b: usize) -> bool {
        self.is_button_down(b) && !self.prev_down.get(b).copied().unwrap_or(false)
    }
    pub fn is_button_released(&self, b: usize) -> bool {
        !self.is_button_down(b) && self.prev_down.get(b).copied().unwrap_or(false)
    }
    /// Roll `down` into `prev_down` and clear per-frame state.
    pub fn end_frame(&mut self) {
        self.prev_down = self.down.clone();
        self.wheel = 0;
        self.ui_blocked.fill(false);
    }
}

/// `UIManager.FadeInfo` (per-player fade).
#[derive(Clone, Debug)]
pub struct FadeInfo {
    pub fade_before_ui: bool,
    pub fade_alpha: f32,
    pub fade_time: i32,
    pub fade_time_max: i32,
    pub fading_out: bool,
}

impl Default for FadeInfo {
    fn default() -> Self {
        FadeInfo { fade_before_ui: false, fade_alpha: 0.0, fade_time: 2, fade_time_max: 2, fading_out: false }
    }
}

impl FadeInfo {
    fn set_fade_time(&mut self, t: i32) {
        self.fade_time = t;
    }
    pub fn fade_in(&mut self, seconds: i32, lock_fps: i32) {
        self.fade_time_max = ((seconds * 30) as f32 * (lock_fps as f32 / 30.0)) as i32;
        self.fade_time = self.fade_time_max;
        self.fading_out = false;
    }
    pub fn fade_out(&mut self, seconds: i32, lock_fps: i32) {
        self.fade_in(seconds, lock_fps);
        self.fading_out = true;
    }
}

/// `UIManager.BlinkInfo`.
#[derive(Clone, Debug)]
pub struct BlinkInfo {
    pub alpha: f32,
    pub direction: f32,
    pub synced_icon_index: i32,
    pub synced_icon_index_timer: f32,
}

impl Default for BlinkInfo {
    fn default() -> Self {
        BlinkInfo { alpha: 1.0, direction: -1.0, synced_icon_index: 0, synced_icon_index_timer: 0.0 }
    }
}

impl BlinkInfo {
    fn update(&mut self, lock_fps: i32, realworld_seconds: f32) {
        if self.alpha >= 1.0 {
            self.alpha = 1.0;
            self.direction = -1.0;
        } else if self.alpha <= 0.0 {
            self.alpha = 0.0;
            self.direction = 1.0;
        }
        self.alpha += 0.015 * (lock_fps as f32 / 30.0) * self.direction;
        self.synced_icon_index_timer += realworld_seconds;
        if self.synced_icon_index_timer > 1.5 {
            self.synced_icon_index_timer = 0.0;
            self.synced_icon_index += 1;
            if self.synced_icon_index > i16::MAX as i32 {
                self.synced_icon_index = 0;
            }
        }
    }
}

pub struct UIManager {
    /// `UIManager.UI`, in draw order (last is on top).
    pub ui: Vec<Rc<UIElement>>,
    pub to_add: Vec<Rc<UIElement>>,
    pub to_remove: Vec<Rc<UIElement>>,
    pub to_top: Vec<Rc<UIElement>>,
    pub debug_ui: Vec<Rc<UIElement>>,
    pub last_mouse_x: i32,
    pub last_mouse_y: i32,
    pub fade_before_ui: bool,
    pub fade_alpha: f32,
    pub fade_in_time_max: i32,
    pub fade_in_time: i32,
    pub fading_out: bool,
    pub last_off_x: f32,
    pub last_off_y: f32,
    pub do_tick: bool,
    pub visible_all_ui: bool,
    pub use_ui_fbo: bool,
    pub suspend: bool,
    pub last_alpha: f32,
    pub ui_update_time_ms: i64,
    pub ui_update_interval_ms: i64,
    pub ui_render_time_ms: i64,
    pub ui_render_interval_ms: i64,
    pub show_lua_debugger_on_error: bool,
    pub show_paused_message: bool,
    pub rendering: bool,
    pub updating: bool,
    pub player_inventory_ui: Option<Rc<UIElement>>,
    pub player_loot_ui: Option<Rc<UIElement>>,
    pub player_inventory_tooltip: Option<Rc<UIElement>>,
    pub player_loot_tooltip: Option<Rc<UIElement>>,
    pub player_fade_info: [FadeInfo; 4],
    pub player_blink_info: [BlinkInfo; 4],
    /// Textures Lua stores and reads back (`setBlack`, `setMouseArrow`...).
    pub black: Value,
    pub mouse_arrow: Value,
    pub mouse_examine: Value,
    pub mouse_attack: Value,
    pub mouse_grab: Value,
    pub last_mouse_texture: Value,
    /// `UIManager.modal` (`ModalDialog`). TODO: port `ModalDialog`.
    pub modal: Option<Rc<UIElement>>,
    /// `TutorialManager.instance.stealControl`. TODO(world).
    pub steal_control: bool,
    /// `IsoPlayer.numPlayers`. TODO(world).
    pub num_players: usize,
    /// `PerformanceSettings.getLockFPS()`.
    pub lock_fps: i32,
    pub mouse: MouseState,
    /// `SpriteRenderer.instance` for the UI.
    pub sprite: SpriteRenderer,
    /// `TextManager.instance`.
    pub text: Option<Rc<dyn TextRenderer>>,
    /// `LuaEventManager.triggerEvent`.
    pub events: Option<EventSink>,
    /// `Core.UnfocusActiveTextEntryBox()`, supplied by the text entry
    /// binding (`Core.currentTextEntryBox` lives there).
    pub unfocus_text_entry: Option<UnfocusHook>,
    /// `UIManager.speedControls` (`zombie.ui.SpeedControls`, built by the
    /// in-game `init` in pz-lua-api's `hud`).
    pub speed_controls: Option<Rc<UIElement>>,
    /// `UIManager.clock`.
    pub clock: Option<Rc<UIElement>>,
    /// `UIManager.toolTip`.
    pub tool_tip: Option<Rc<UIElement>>,
    /// `UIManager.debugConsole`.
    pub debug_console: Option<Rc<UIElement>>,
    /// `UIManager.MoodleUI[4]`.
    pub moodle_ui: [Option<Rc<UIElement>>; 4],
    /// `UIManager.ProgressBar[4]`.
    pub progress_bar: [Option<Rc<UIElement>>; 4],
    /// `UIManager.resize()`'s HUD layout, supplied with the HUD widgets.
    pub resize_hook: Option<Rc<dyn Fn()>>,
    /// `UIManager.updateTooltip(mx, my)`, supplied with the shared tooltip
    /// (it needs the world's picked object).
    pub update_tooltip_hook: Option<TooltipHook>,
    /// Fixed clock for tests (`System.currentTimeMillis`).
    pub time_override_ms: Option<i64>,
}

impl Default for UIManager {
    fn default() -> Self {
        UIManager {
            ui: Vec::new(),
            to_add: Vec::new(),
            to_remove: Vec::new(),
            to_top: Vec::new(),
            debug_ui: Vec::new(),
            last_mouse_x: 0,
            last_mouse_y: 0,
            fade_before_ui: false,
            fade_alpha: 1.0,
            fade_in_time_max: 180,
            fade_in_time: 180,
            fading_out: false,
            last_off_x: 0.0,
            last_off_y: 0.0,
            do_tick: false,
            visible_all_ui: true,
            use_ui_fbo: false,
            suspend: false,
            last_alpha: 10000.0,
            ui_update_time_ms: 0,
            ui_update_interval_ms: 0,
            ui_render_time_ms: 0,
            ui_render_interval_ms: 0,
            show_lua_debugger_on_error: false,
            show_paused_message: true,
            rendering: false,
            updating: false,
            player_inventory_ui: None,
            player_loot_ui: None,
            player_inventory_tooltip: None,
            player_loot_tooltip: None,
            player_fade_info: Default::default(),
            player_blink_info: Default::default(),
            black: Value::Nil,
            mouse_arrow: Value::Nil,
            mouse_examine: Value::Nil,
            mouse_attack: Value::Nil,
            mouse_grab: Value::Nil,
            last_mouse_texture: Value::Nil,
            modal: None,
            steal_control: false,
            num_players: 1,
            lock_fps: 60,
            mouse: MouseState::default(),
            sprite: SpriteRenderer::default(),
            text: None,
            events: None,
            unfocus_text_entry: None,
            speed_controls: None,
            clock: None,
            tool_tip: None,
            debug_console: None,
            moodle_ui: Default::default(),
            progress_bar: Default::default(),
            resize_hook: None,
            update_tooltip_hook: None,
            time_override_ms: None,
        }
    }
}

thread_local! {
    static MANAGER: RefCell<UIManager> = RefCell::new(UIManager::default());
}

/// Run `f` on the thread's `UIManager` statics. Must not call into Lua.
pub fn with_ui<R>(f: impl FnOnce(&mut UIManager) -> R) -> R {
    MANAGER.with(|m| f(&mut m.borrow_mut()))
}

/// `System.currentTimeMillis()` (or the test override).
pub fn now_ms() -> i64 {
    if let Some(t) = with_ui(|m| m.time_override_ms) {
        return t;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn remove_ptr(v: &mut Vec<Rc<UIElement>>, el: &Rc<UIElement>) {
    if let Some(i) = v.iter().position(|e| Rc::ptr_eq(e, el)) {
        v.remove(i);
    }
}

fn contains(v: &[Rc<UIElement>], el: &Rc<UIElement>) -> bool {
    v.iter().any(|e| Rc::ptr_eq(e, el))
}

fn n(v: f64) -> Value {
    Value::Num(v)
}

impl UIManager {
    /// Reset all statics (fresh `UIManager` class state).
    pub fn reset() {
        with_ui(|m| *m = UIManager::default());
    }

    pub fn set_text_renderer(r: Option<Rc<dyn TextRenderer>>) {
        with_ui(|m| m.text = r);
    }

    pub fn set_event_sink(s: Option<EventSink>) {
        with_ui(|m| m.events = s);
    }

    /// `LuaEventManager.triggerEvent(name, args...)` through the sink.
    pub fn trigger(t: &mut Thread, name: &str, args: &[Value]) {
        if let Some(sink) = with_ui(|m| m.events.clone()) {
            sink(t, name, args);
        }
    }

    // ---- list management ----

    /// `AddUI(el)`: queued until the next `update`.
    pub fn add_ui(el: Rc<UIElement>) {
        with_ui(|m| {
            remove_ptr(&mut m.to_remove, &el);
            m.to_remove.push(el.clone());
            remove_ptr(&mut m.to_add, &el);
            m.to_add.push(el);
        });
    }

    /// `RemoveElement(el)`.
    pub fn remove_element(el: Rc<UIElement>) {
        with_ui(|m| {
            remove_ptr(&mut m.to_add, &el);
            remove_ptr(&mut m.to_remove, &el);
            m.to_remove.push(el);
        });
    }

    pub fn clear_arrays() {
        with_ui(|m| {
            m.to_add.clear();
            m.to_remove.clear();
            m.ui.clear();
        });
    }

    /// `updateBeforeFadeOut()`: apply the pending add/remove queues.
    pub fn update_before_fade_out() {
        with_ui(|m| m.apply_queues());
    }

    fn apply_queues(&mut self) {
        if !self.to_remove.is_empty() {
            let rem = std::mem::take(&mut self.to_remove);
            self.ui.retain(|e| !contains(&rem, e));
        }
        let add = std::mem::take(&mut self.to_add);
        self.ui.extend(add);
    }

    pub(crate) fn push_to_top(el: Rc<UIElement>) {
        with_ui(|m| m.to_top.push(el));
    }

    /// `getUI()` snapshot.
    pub fn get_ui() -> Vec<Rc<UIElement>> {
        with_ui(|m| m.ui.clone())
    }

    /// `init()`: clears the UI. TODO(world): speed controls, tooltip,
    /// clock, moodles, debug console, progress bars and `OnCreateUI` are
    /// only created with a player (`IsoPlayer.getInstance() != null`).
    pub fn init() {
        with_ui(|m| {
            m.show_paused_message = true;
            m.ui.clear();
            m.debug_ui.clear();
            m.clock = None;
            m.moodle_ui = Default::default();
            m.fade_before_ui = false;
            m.visible_all_ui = true;
            for f in &mut m.player_fade_info {
                f.fade_before_ui = false;
                f.fade_time = 0;
                f.fading_out = false;
            }
        });
    }

    /// `resize()`: the HUD layout (clock, moodles, speed controls) through
    /// the hook its widgets install; nothing without a clock.
    pub fn resize() {
        if with_ui(|m| m.clock.is_none()) {
            return;
        }
        if let Some(h) = with_ui(|m| m.resize_hook.clone()) {
            h();
        }
    }

    // ---- per-frame update ----

    /// `UIManager.update()`.
    pub fn update(t: &mut Thread) {
        let now = now_ms();
        let Some((mx, my)) = with_ui(|m| {
            if m.suspend {
                return None;
            }
            m.apply_queues();
            m.fade_in_time = (m.fade_in_time - 1).max(0);
            let (lock_fps, np) = (m.lock_fps, m.num_players.min(4));
            for i in 0..np {
                let ft = m.player_fade_info[i].fade_time - 1;
                m.player_fade_info[i].set_fade_time(ft);
                // TODO(world): GameTime.getRealworldSecondsSinceLastUpdate.
                let secs = m.ui_update_interval_ms as f32 / 1000.0;
                m.player_blink_info[i].update(lock_fps, secs);
            }
            if now - m.ui_update_time_ms >= 100 {
                m.do_tick = true;
                m.ui_update_interval_ms = (now - m.ui_update_time_ms).min(1000);
                m.ui_update_time_ms = now;
            } else {
                m.do_tick = false;
            }
            m.reorder();
            m.updating = true;
            Some((m.mouse.x, m.mouse.y))
        }) else {
            return;
        };

        let consumed = Self::update_mouse_buttons(t, mx, my);
        let _consumed_click = consumed & 1 == 1;
        let _consumed_rclick = consumed & 2 == 2;
        // TODO(world): "Attack/Click" keybind on picked objects
        // (OnObjectLeftMouseButtonDown/Up, IsoObject.onMouseLeftClick).

        let wheel = with_ui(|m| m.mouse.wheel);
        let lastwheel = 0;
        if wheel != lastwheel {
            let del = if wheel - lastwheel < 0 { 1.0 } else { -1.0 };
            let uis = Self::get_ui();
            let mut i = uis.len() as isize - 1;
            while i >= 0 {
                let ui = uis[i as usize].clone();
                if (ui.is_point_over(mx as f64, my as f64) || ui.is_capture())
                    && ui.on_consume_mouse_wheel(t, del, mx as f64 - ui.get_x(), my as f64 - ui.get_y())
                {
                    break;
                }
                i -= 1;
            }
            Self::trigger(t, "OnMouseWheel", &[n(wheel as f64)]);
            // TODO(world): Core.doZoomScroll(0, del) when not consumed.
        }

        let _consumed_move = Self::update_mouse_move(t, mx, my, false);
        // TODO(world): with a player, an unconsumed move picks objects and
        // triggers OnMouseMove(mx, my, mxw, myw).

        with_ui(|m| {
            m.last_mouse_x = mx;
            m.last_mouse_y = my;
        });
        Self::update_ui_elements(t);
        // updateTooltip (the shared tooltip over picked objects).
        if let Some(h) = with_ui(|m| m.update_tooltip_hook.clone()) {
            h(t, mx, my);
        }
        // TODO(world): handleZoomKeys, IsoCamera last offsets.
        with_ui(|m| m.updating = false);
    }

    /// The z-order pass of `update()`: always-on-top and `bringToTop`ped
    /// elements to the end, back-most and world-following ones to the front.
    fn reorder(&mut self) {
        let mut tutorial = Vec::new();
        let mut i = self.ui.len() as isize - 1;
        while i >= 0 {
            let ui = self.ui[i as usize].clone();
            if ui.get_parent().is_some() {
                // The original throws IllegalStateException here.
                self.ui.remove(i as usize);
                i -= 1;
                continue;
            }
            if ui.is_follow_game_world() {
                tutorial.push(ui.clone());
            }
            if ui.ext::<crate::tooltip::ObjectTooltip>().is_some() {
                let rem = self.ui.remove(i as usize);
                self.ui.push(rem);
            }
            i -= 1;
        }
        let mut moved = Vec::new();
        let mut i = 0;
        while i < self.ui.len() {
            let ui = &self.ui[i];
            if ui.is_always_on_top() || contains(&self.to_top, ui) {
                moved.push(self.ui.remove(i));
            } else {
                i += 1;
            }
        }
        self.ui.extend(moved);
        self.to_top.clear();
        for i in 0..self.ui.len() {
            if self.ui[i].is_back_most() {
                let e = self.ui.remove(i);
                self.ui.insert(0, e);
            }
        }
        for e in tutorial {
            remove_ptr(&mut self.ui, &e);
            self.ui.insert(0, e);
        }
    }

    /// `isOverElement(ui, mx, my)`: -1 ignore, 0 outside, 1 inside.
    fn is_over_element(ui: &UIElement, mx: i32, my: i32) -> i32 {
        let (steal, modal) = with_ui(|m| (m.steal_control, m.modal.clone()));
        if !ui.is_ignore_loss_control() && steal {
            return -1;
        }
        if !ui.is_visible() {
            return -1;
        }
        if let Some(md) = modal {
            if !std::ptr::eq(Rc::as_ptr(&md), ui) && md.is_visible() {
                return -1;
            }
        }
        if ui.is_capture() {
            return 1;
        }
        let (mx, my) = (mx as f64, my as f64);
        let mdh = ui.get_max_draw_height();
        let hit = if mdh != -1.0 {
            mx >= ui.get_x() && my >= ui.get_y() && mx < ui.get_x() + ui.get_width() && my < ui.get_y() + ui.get_height().min(mdh)
        } else {
            ui.is_over_element(mx, my)
        };
        hit as i32
    }

    /// `updateMouseButtons(mx, my)`: bit 0 left consumed, bit 1 right.
    fn update_mouse_buttons(t: &mut Thread, mx: i32, my: i32) -> i32 {
        let mouse = with_ui(|m| m.mouse.clone());
        let mut consumed_click = false;
        let mut consumed_rclick = false;
        for btn in 0..mouse.button_count() {
            // TODO(world): btn 0 Gizmos.hitTest.
            let mut consumed = false;
            let pressed = mouse.is_button_pressed(btn);
            let released = !pressed && mouse.is_button_released(btn);
            if pressed || released {
                if pressed && btn == 0 {
                    if let Some(f) = with_ui(|m| m.unfocus_text_entry.clone()) {
                        f(t);
                    }
                }
                let uis = Self::get_ui();
                let mut i = uis.len() as isize - 1;
                while i >= 0 && !consumed {
                    let ui = uis[i as usize].clone();
                    let (lx, ly) = (mx as f64 - ui.get_x(), my as f64 - ui.get_y());
                    match Self::is_over_element(&ui, mx, my) {
                        0 => {
                            if pressed {
                                ui.on_mouse_button_down_outside(t, btn as i32, lx, ly);
                            } else {
                                ui.on_mouse_button_up_outside(t, btn as i32, lx, ly);
                            }
                        }
                        1 => {
                            let c = if pressed {
                                ui.on_consume_mouse_button_down(t, btn as i32, lx, ly)
                            } else {
                                ui.on_consume_mouse_button_up(t, btn as i32, lx, ly)
                            };
                            if c {
                                consumed = true;
                            }
                        }
                        _ => {}
                    }
                    i -= 1;
                }
            }
            if btn == 0 {
                consumed_click = consumed;
            } else if btn == 1 {
                consumed_rclick = consumed;
            }
        }
        for btn in 2..mouse.button_count() {
            let key = n((10000 + btn) as f64);
            if mouse.is_button_pressed(btn) {
                Self::trigger(t, "OnKeyStartPressed", std::slice::from_ref(&key));
                Self::trigger(t, "OnKeyPressed", &[key]);
            } else if !mouse.is_button_released(btn) && mouse.is_button_down(btn) {
                Self::trigger(t, "OnKeyKeepPressed", &[key]);
            }
        }
        let (fx, fy) = (n(mx as f64), n(my as f64));
        if mouse.is_button_pressed(0) {
            if !consumed_click {
                Self::trigger(t, "OnMouseDown", &[fx.clone(), fy.clone()]);
                Self::trigger(t, "OnKeyStartPressed", &[n(10000.0)]);
                Self::trigger(t, "OnKeyPressed", &[n(10000.0)]);
                // CloseContainers() is empty.
            } else {
                with_ui(|m| m.mouse.ui_blocked[0] = true);
            }
        } else if mouse.is_button_released(0) {
            if !consumed_click {
                Self::trigger(t, "OnMouseUp", &[fx.clone(), fy.clone()]);
            }
        } else if mouse.is_button_down(0) && !consumed_click {
            Self::trigger(t, "OnKeyKeepPressed", &[n(10000.0)]);
        }
        if mouse.is_button_pressed(1) {
            if !consumed_rclick {
                Self::trigger(t, "OnRightMouseDown", &[fx.clone(), fy.clone()]);
                // TODO(world): OnObjectRightMouseButtonDown on picked object.
            } else {
                with_ui(|m| {
                    if m.mouse.ui_blocked.len() > 1 {
                        m.mouse.ui_blocked[1] = true
                    }
                });
            }
            // TODO(world): speed controls / IsoObject.onMouseRightClick.
        } else if mouse.is_button_released(1) {
            if !consumed_rclick {
                Self::trigger(t, "OnRightMouseUp", &[fx, fy]);
                // TODO(world): OnObjectRightMouseButtonUp, drag object.
            }
        } else if mouse.is_button_down(1) {
            let uis = Self::get_ui();
            for ui in uis.iter().rev() {
                if Self::is_over_element(ui, mx, my) == 1 {
                    consumed_rclick = true;
                    break;
                }
            }
            if !consumed_rclick {
                Self::trigger(t, "OnKeyKeepPressed", &[n(10001.0)]);
            }
        }
        if !consumed_rclick && mouse.right_delay {
            Self::trigger(t, "OnKeyStartPressed", &[n(10001.0)]);
            Self::trigger(t, "OnKeyPressed", &[n(10001.0)]);
        }
        consumed_click as i32 + if consumed_rclick { 2 } else { 0 }
    }

    /// `updateMouseMove(mx, my, consumedMove)`.
    fn update_mouse_move(t: &mut Thread, mx: i32, my: i32, mut consumed: bool) -> bool {
        let (lx, ly, steal) = with_ui(|m| (m.last_mouse_x, m.last_mouse_y, m.steal_control));
        if lx == mx && ly == my {
            return consumed;
        }
        let (dx, dy) = ((mx - lx) as f64, (my - ly) as f64);
        let uis = Self::get_ui();
        for ui in uis.iter().rev() {
            if (!ui.is_ignore_loss_control() && steal) || !ui.is_visible() {
                continue;
            }
            let (ux, uy) = (mx as f64 - ui.get_x(), my as f64 - ui.get_y());
            if ui.is_over_element(mx as f64, my as f64) || ui.is_capture() {
                if !consumed && ui.on_consume_mouse_move(t, dx, dy, ux, uy) {
                    consumed = true;
                }
            } else {
                ui.on_extend_mouse_move_outside(t, dx, dy, ux, uy);
            }
        }
        consumed
    }

    /// `updateUIElements()`: by index, re-reading the live list.
    fn update_ui_elements(t: &mut Thread) {
        let mut i = 0;
        while let Some(ui) = with_ui(|m| m.ui.get(i).cloned()) {
            ui.update(t);
            i += 1;
        }
    }

    // ---- render ----

    /// `UIManager.render()`: records into [`UIManager::sprite`] (cleared
    /// first).
    pub fn render(t: &mut Thread) {
        let now = now_ms();
        let go = with_ui(|m| {
            if m.suspend {
                return false;
            }
            m.ui_render_interval_ms = (now - m.ui_render_time_ms).min(1000);
            m.ui_render_time_ms = now;
            m.sprite.clear();
            m.sprite.reset_stencil();
            m.sprite.set_blend(Blend::Alpha);
            true
        });
        if !go {
            return;
        }
        // TODO(render): UITransition.UpdateAll(); black.png via the loader.
        Self::trigger(t, "OnPreUIDraw", &[]);
        with_ui(|m| {
            if m.fade_before_ui {
                m.render_fade_overlay();
            }
            m.last_alpha = m.fade_alpha;
            // TODO(world): per-player fades need IsoPlayer.players.
            m.rendering = true;
        });
        let mut i = 0;
        while let Some(el) = with_ui(|m| m.ui.get(i).cloned()) {
            i += 1;
            let steal = with_ui(|m| m.steal_control);
            if (!el.is_ignore_loss_control() && steal) || el.is_follow_game_world() {
                continue;
            }
            if !el.is_default_draw() {
                continue;
            }
            el.render(t);
        }
        with_ui(|m| m.rendering = false);
        if let Some(tip) = with_ui(|m| m.tool_tip.clone()) {
            tip.render(t);
        }
        with_ui(|m| {
            // TODO(world): the "IGUI_GamePaused" box (GameTime.isGamePaused).
            if !m.fade_before_ui {
                m.render_fade_overlay();
            }
        });
        Self::trigger(t, "OnPostUIDraw", &[]);
    }

    /// `renderFadeOverlay()`: updates `fadeAlpha`. TODO(world): the black
    /// overlay is drawn only with a camera character.
    fn render_fade_overlay(&mut self) {
        let a = self.fade_in_time as f32 / self.fade_in_time_max as f32;
        self.set_fade_alpha(a as f64);
        if self.fading_out {
            let a = 1.0 - self.fade_alpha as f64;
            self.set_fade_alpha(a);
        }
    }

    pub fn set_fade_alpha(&mut self, a: f64) {
        let a = a as f32;
        self.fade_alpha = if a.is_nan() { a } else { a.clamp(0.0, 1.0) };
    }

    pub fn set_fade_in_time(&mut self, t: f64) {
        self.fade_in_time = (t as i32).max(0);
    }

    /// `FadeIn(seconds)` / `FadeOut(seconds)`.
    pub fn fade(&mut self, seconds: f64, out: bool) {
        self.fade_in_time_max = (seconds * 30.0 * (self.lock_fps as f32 / 30.0) as f64) as i32;
        let max = self.fade_in_time_max as f64;
        self.set_fade_in_time(max);
        self.fading_out = out;
    }

    // ---- keys ----

    fn key_dispatch(t: &mut Thread, f: impl Fn(&UIElement, &mut Thread) -> bool) -> bool {
        let uis = Self::get_ui();
        for ui in uis.iter().rev() {
            if ui.is_visible() && ui.is_want_key_events() && f(ui, t) {
                return true;
            }
        }
        false
    }

    pub fn on_key_press(t: &mut Thread, key: i32) -> bool {
        Self::key_dispatch(t, |ui, t| ui.on_consume_key_press(t, key))
    }
    pub fn on_key_repeat(t: &mut Thread, key: i32) -> bool {
        Self::key_dispatch(t, |ui, t| ui.on_consume_key_repeat(t, key))
    }
    pub fn on_key_release(t: &mut Thread, key: i32) -> bool {
        Self::key_dispatch(t, |ui, t| ui.on_consume_key_release(t, key))
    }

    // ---- queries ----

    pub fn is_force_cursor_visible() -> bool {
        Self::get_ui().iter().rev().any(|ui| ui.is_visible() && (ui.is_force_cursor_visible() || ui.is_mouse_over()))
    }

    pub fn is_modal_visible() -> bool {
        Self::get_ui().iter().any(|ui| ui.is_modal_visible())
    }

    pub fn get_double_click_interval() -> f64 {
        500.0
    }
    pub fn get_double_click_dist() -> f64 {
        5.0
    }

    /// `isDoubleClick(x1, y1, x2, y2, clickTime)`.
    pub fn is_double_click(x1: f64, y1: f64, x2: f64, y2: f64, click_time: f64) -> bool {
        if (x2 - x1).abs() > Self::get_double_click_dist() || (y2 - y1).abs() > Self::get_double_click_dist() {
            return false;
        }
        // Negated `>` keeps Java's NaN behaviour.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let r = !((now_ms() as f64) - click_time > Self::get_double_click_interval());
        r
    }

    pub fn is_mouse_over_inventory() -> bool {
        let (inv, loot, invt, lt) = with_ui(|m| {
            (m.player_inventory_ui.clone(), m.player_loot_ui.clone(), m.player_inventory_tooltip.clone(), m.player_loot_tooltip.clone())
        });
        if invt.is_some_and(|u| u.is_mouse_over()) || lt.is_some_and(|u| u.is_mouse_over()) {
            return true;
        }
        let (Some(inv), Some(loot)) = (inv, loot) else { return false };
        if inv.get_max_draw_height() == -1.0 && inv.is_mouse_over() {
            return true;
        }
        loot.get_max_draw_height() == -1.0 && loot.is_mouse_over()
    }

    pub fn get_blink_alpha(player: i32) -> f32 {
        with_ui(|m| if (0..4).contains(&player) { m.player_blink_info[player as usize].alpha } else { 1.0 })
    }

    /// `getSyncedIconIndex(playerIndex, maxIndex)` (always player 0's
    /// counter, as in the original).
    pub fn get_synced_icon_index(max_index: i32) -> i32 {
        with_ui(|m| if max_index == 0 { 0 } else { m.player_blink_info[0].synced_icon_index % max_index })
    }
}
