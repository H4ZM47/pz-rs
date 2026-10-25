//! `zombie.ui.ActionProgressBar`: the timed-action bar above a player.

use super::host::with_host;
use pz_render::{DrawCmd, Rgba, Texture};
use pz_ui::{with_ui, UIElement, UIElementExt, UiDrawCmd};
use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

pub const ACTION_PROGRESS_BAR: &str = "zombie.ui.ActionProgressBar";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];
const WHITE: Rgba = Rgba(1.0, 1.0, 1.0, 1.0);

/// `zombie.ui.ActionProgressBar`.
pub struct ActionProgressBar {
    background: Option<Texture>,
    foreground: Option<Texture>,
    delta_value: Cell<f32>,
    animation_progress: Cell<f32>,
    pub delay_hide: Cell<i32>,
    offset_x: i32,
    offset_y: i32,
}

fn w(t: &Option<Texture>) -> f32 {
    t.as_ref().map_or(0.0, |t| t.width as f32)
}
fn h(t: &Option<Texture>) -> f32 {
    t.as_ref().map_or(0.0, |t| t.height as f32)
}

fn bar_size() -> f32 {
    crate::bind_core::shared_core()
        .borrow()
        .get_option_action_progress_bar_size() as f32
}

impl ActionProgressBar {
    /// `new ActionProgressBar(x, y)`.
    pub fn create(x: i32, y: i32) -> Rc<UIElement> {
        let b = ActionProgressBar {
            background: pz_render::get_shared_texture("BuildBar_Bkg"),
            foreground: pz_render::get_shared_texture("BuildBar_Bar"),
            delta_value: Cell::new(1.0),
            animation_progress: Cell::new(0.0),
            delay_hide: Cell::new(0),
            offset_x: x,
            offset_y: y,
        };
        let (bw, bh) = (w(&b.background), h(&b.background));
        let e = UIElement::new_ext(None, Box::new(b));
        {
            let mut s = e.st_mut();
            s.width = bw;
            s.height = bh;
            s.follow_game_world = true;
        }
        e
    }

    pub fn set_value(&self, v: f32) {
        self.delta_value.set(v);
    }

    pub fn get_value(&self) -> f32 {
        self.delta_value.get()
    }

    /// `update(nPlayer)`.
    pub fn update_bar(&self, el: &UIElement, n: usize) {
        let visible_all = with_ui(|m| m.visible_all_ui);
        if self.delta_value.get() == f32::INFINITY {
            let secs = with_host(|h| h.game_time.realworld_seconds_since_last_update());
            let mut p = self.animation_progress.get() + 0.02 * (secs * 60.0);
            if p > 1.0 {
                p = 0.0;
            }
            self.animation_progress.set(p);
            el.set_visible(true);
            self.update_screen_pos(el, n);
            self.delay_hide.set(2);
            return;
        }
        let v = self.get_value();
        if v > 0.0 && v < 1.0 {
            el.set_visible(true);
            self.delay_hide.set(2);
        } else if el.is_visible() && self.delay_hide.get() > 0 {
            self.delay_hide.set(self.delay_hide.get() - 1);
            if self.delay_hide.get() == 0 {
                el.set_visible(false);
            }
        }
        if !visible_all {
            el.set_visible(false);
        }
        if el.is_visible() {
            self.update_screen_pos(el, n);
        }
    }

    /// `updateScreenPos(nPlayer)`.
    fn update_screen_pos(&self, el: &UIElement, n: usize) {
        let Some(p) = with_host(|h| h.players.player_screen(n)) else {
            return;
        };
        let scale = bar_size();
        let width = w(&self.background) * scale;
        let height = h(&self.background) * scale;
        {
            let mut s = el.st_mut();
            s.width = width;
            s.height = height;
        }
        let (off_x, off_y, zoom) =
            with_host(|h| (h.camera.off_x(), h.camera.off_y(), h.camera.zoom(n)));
        let tile_scale = crate::bind_core::shared_core().borrow().tile_scale;
        let mut sx = pz_render::iso::x_to_screen(p.x, p.y);
        let mut sy = pz_render::iso::y_to_screen(p.x, p.y, p.z, 0);
        sx = sx - off_x - p.offset_x;
        sy = sy - off_y - p.offset_y;
        sy -= (128 / (2 / tile_scale)) as f32;
        sx /= zoom;
        sy /= zoom;
        sx -= width / 2.0;
        sy -= height;
        if p.user_name_height > 0 {
            sy -= (p.user_name_height + 2) as f32;
        }
        el.set_x((sx + self.offset_x as f32) as f64);
        el.set_y((sy + self.offset_y as f32) as f64);
    }
}

impl UIElementExt for ActionProgressBar {
    fn class_name(&self) -> &'static str {
        ACTION_PROGRESS_BAR
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `render()` (no `super.render()`).
    fn render(&self, el: &UIElement, _t: &mut pz_kahlua::Thread) {
        if !el.is_visible() || !with_ui(|m| m.visible_all_ui) {
            return;
        }
        let scale = bar_size();
        // IndieGL.glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA).
        with_ui(|m| {
            m.sprite.push(UiDrawCmd::Render(DrawCmd::BlendFuncSeparate(
                770, 771, 1, 771,
            )))
        });
        let bg = self.background.as_ref();
        el.draw_uv_slice_texture(
            bg,
            0.0,
            0.0,
            (w(&self.background) * scale) as f64,
            (h(&self.background) * scale) as f64,
            WHITE,
            0.0,
            0.0,
            1.0,
            1.0,
        );
        let fg_off_y = self.foreground.as_ref().map_or(0.0, |t| t.offset_y);
        let fg_y = fg_off_y * scale - fg_off_y;
        let fg_h = h(&self.foreground) + 1.0;
        let fg_w = w(&self.foreground) + 1.0;
        let fg = self.foreground.as_ref();
        let (x, ww, hh) = (
            (3.0 * scale) as f64,
            (fg_w * scale) as f64,
            (fg_h * scale) as f64,
        );
        let d = self.delta_value.get();
        if d == f32::INFINITY {
            let a = self.animation_progress.get();
            if a < 0.5 {
                el.draw_uv_slice_texture(
                    fg,
                    x,
                    fg_y as f64,
                    ww,
                    hh,
                    WHITE,
                    0.0,
                    0.0,
                    (a * 2.0) as f64,
                    1.0,
                );
            } else {
                el.draw_uv_slice_texture(
                    fg,
                    x,
                    fg_y as f64,
                    ww,
                    hh,
                    WHITE,
                    ((a - 0.5) * 2.0) as f64,
                    0.0,
                    1.0,
                    1.0,
                );
            }
        } else {
            el.draw_uv_slice_texture(fg, x, fg_y as f64, ww, hh, WHITE, 0.0, 0.0, d as f64, 1.0);
        }
    }
}
