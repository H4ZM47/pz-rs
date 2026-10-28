//! `zombie.ui.ScrollBar`: the vertical scroll bar of the debug console's
//! output log.

use crate::bind_uitextbox2::UITextBox2;
use pz_kahlua::Thread;
use pz_render::{Rgba, Texture};
use pz_ui::{with_ui, UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub const SCROLL_BAR: &str = "zombie.ui.ScrollBar";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];

fn tex(n: &str) -> Option<Texture> {
    pz_render::get_shared_texture(n)
}

/// `new Color(r, g, b, a)` from bytes.
fn c255(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    )
}

/// `zombie.ui.ScrollBar`.
pub struct ScrollBar {
    pub background_colour: Rgba,
    pub button_colour: Rgba,
    pub button_highlight_colour: Rgba,
    pub is_verticle: bool,
    full_length: Cell<i32>,
    inside_length: Cell<i32>,
    button_inside_length: Cell<f32>,
    button_offset: Cell<f32>,
    mouse_drag_start_pos: Cell<i32>,
    button_drag_start_pos: Cell<f32>,
    back_vertical: Option<Texture>,
    top_vertical: Option<Texture>,
    bottom_vertical: Option<Texture>,
    button_back_vertical: Option<Texture>,
    button_top_vertical: Option<Texture>,
    button_bottom_vertical: Option<Texture>,
    mouse_over: Cell<bool>,
    being_dragged: Cell<bool>,
    parent_text_box: RefCell<Weak<UIElement>>,
    pub name: String,
}

const END_LENGTH: i32 = 7;
const BUTTON_END_LENGTH: f32 = 6.0;
const THICKNESS: f64 = 10.0;
const BUTTON_THICKNESS: f64 = 9.0;

fn mouse_ya() -> i32 {
    with_ui(|m| m.mouse.y)
}

impl ScrollBar {
    /// `new ScrollBar(name, messages, x, y, length, isVertical)` (always
    /// vertical, as in the original; no message parent is used).
    pub fn create(name: &str, x: i32, y: i32, length: i32) -> Rc<UIElement> {
        // The horizontal textures are loaded but never drawn.
        for n in [
            "ScrollbarH_Bkg_Middle",
            "ScrollbarH_Bkg_Bottom",
            "ScrollbarH_Bkg_Top",
            "ScrollbarH_Middle",
            "ScrollbarH_Bottom",
            "ScrollbarH_Top",
        ] {
            let _ = tex(&format!("media/ui/{n}.png"));
        }
        let height = length as f32;
        let sb = ScrollBar {
            background_colour: c255(255, 255, 255, 255),
            button_colour: c255(255, 255, 255, 127),
            button_highlight_colour: c255(255, 255, 255, 255),
            is_verticle: true,
            full_length: Cell::new(length),
            inside_length: Cell::new(length - 14),
            button_inside_length: Cell::new(height - 12.0),
            button_offset: Cell::new(0.0),
            mouse_drag_start_pos: Cell::new(0),
            button_drag_start_pos: Cell::new(0.0),
            back_vertical: tex("media/ui/ScrollbarV_Bkg_Middle.png"),
            top_vertical: tex("media/ui/ScrollbarV_Bkg_Top.png"),
            bottom_vertical: tex("media/ui/ScrollbarV_Bkg_Bottom.png"),
            button_back_vertical: tex("media/ui/ScrollbarV_Middle.png"),
            button_top_vertical: tex("media/ui/ScrollbarV_Top.png"),
            button_bottom_vertical: tex("media/ui/ScrollbarV_Bottom.png"),
            mouse_over: Cell::new(false),
            being_dragged: Cell::new(false),
            parent_text_box: RefCell::new(Weak::new()),
            name: name.to_string(),
        };
        let e = UIElement::new_ext(None, Box::new(sb));
        {
            let mut s = e.st_mut();
            s.x = x as f64;
            s.y = y as f64;
            s.width = THICKNESS as f32;
            s.height = height;
        }
        e
    }

    /// `SetParentTextBox(parent)`.
    pub fn set_parent_text_box(&self, tb: &Rc<UIElement>) {
        *self.parent_text_box.borrow_mut() = Rc::downgrade(tb);
    }

    /// `setHeight(height)` (the override: the lengths follow).
    pub fn set_height(&self, el: &UIElement, h: f64) {
        el.set_height(h);
        self.full_length.set(h as i32);
        self.inside_length.set(h as i32 - 14);
    }

    pub fn is_being_dragged(&self) -> bool {
        self.being_dragged.get()
    }

    pub fn is_mouse_over(&self) -> bool {
        self.mouse_over.get()
    }

    pub fn button_offset(&self) -> f32 {
        self.button_offset.get()
    }

    pub fn button_inside_length(&self) -> f32 {
        self.button_inside_length.get()
    }

    /// `scrollToBottom()`.
    pub fn scroll_to_bottom(&self, el: &UIElement) {
        self.button_offset
            .set(el.get_height() as i32 as f32 - (self.button_inside_length.get() + 12.0) - 0.0);
    }

    fn clamp_offset(&self, el: &UIElement, extra: f32) {
        if self.button_offset.get() < 0.0 {
            self.button_offset.set(0.0);
        }
        let max = el.get_height() as i32 as f32 - (self.button_inside_length.get() + 12.0) - extra;
        if self.button_offset.get() > max {
            self.button_offset.set(max);
        }
    }
}

impl UIElementExt for ScrollBar {
    fn class_name(&self) -> &'static str {
        SCROLL_BAR
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn render(&self, el: &UIElement, _t: &mut Thread) {
        if !self.is_verticle {
            return;
        }
        let bg = self.background_colour;
        let inside = self.inside_length.get() as f64;
        el.draw_texture_scaled_col_c(
            self.top_vertical.as_ref(),
            0.0,
            0.0,
            THICKNESS,
            END_LENGTH as f64,
            bg,
        );
        el.draw_texture_scaled_col_c(
            self.back_vertical.as_ref(),
            0.0,
            END_LENGTH as f64,
            THICKNESS,
            inside,
            bg,
        );
        el.draw_texture_scaled_col_c(
            self.bottom_vertical.as_ref(),
            0.0,
            END_LENGTH as f64 + inside,
            THICKNESS,
            END_LENGTH as f64,
            bg,
        );
        let col = if self.mouse_over.get() {
            self.button_highlight_colour
        } else {
            self.button_colour
        };
        let off = self.button_offset.get() as i32;
        let bil = self.button_inside_length.get();
        el.draw_texture_scaled_col_c(
            self.button_top_vertical.as_ref(),
            1.0,
            (off + 1) as f64,
            BUTTON_THICKNESS,
            BUTTON_END_LENGTH as f64,
            col,
        );
        el.draw_texture_scaled_col_c(
            self.button_back_vertical.as_ref(),
            1.0,
            (off + 1 + 6) as f64,
            BUTTON_THICKNESS,
            bil as f64,
            col,
        );
        el.draw_texture_scaled_col_c(
            self.button_bottom_vertical.as_ref(),
            1.0,
            ((off + 1 + 6) as f32 + bil) as f64,
            BUTTON_THICKNESS,
            BUTTON_END_LENGTH as f64,
            col,
        );
    }

    fn on_mouse_move(&self, _el: &UIElement, _t: &mut Thread, _dx: f64, _dy: f64) -> bool {
        self.mouse_over.set(true);
        true
    }

    fn on_mouse_move_outside(&self, _el: &UIElement, _t: &mut Thread, _dx: f64, _dy: f64) {
        self.mouse_over.set(false);
    }

    fn on_mouse_up(&self, _el: &UIElement, _t: &mut Thread, _x: f64, _y: f64) -> bool {
        self.being_dragged.set(false);
        false
    }

    /// A press on the button starts a drag; elsewhere the button centres
    /// on the press.
    fn on_mouse_down(&self, el: &UIElement, _t: &mut Thread, _x: f64, y: f64) -> bool {
        let off = self.button_offset.get();
        let len = self.button_inside_length.get() + 12.0;
        if y >= off as f64 && y <= (off + len) as f64 {
            self.being_dragged.set(true);
            self.mouse_drag_start_pos.set(mouse_ya());
            self.button_drag_start_pos.set(off);
        } else {
            self.button_offset.set((y - (len / 2.0) as f64) as f32);
        }
        self.clamp_offset(el, 1.0);
        false
    }

    /// `update()`: the drag, then the button's size and the text box's
    /// top line from the parent text box.
    fn update(&self, el: &UIElement, t: &mut Thread) {
        el.update_base(t);
        if self.being_dragged.get() {
            let dist = self.mouse_drag_start_pos.get() - mouse_ya();
            self.button_offset
                .set(self.button_drag_start_pos.get() - dist as f32);
            self.clamp_offset(el, 0.0);
            if !with_ui(|m| m.mouse.is_button_down(0)) {
                self.being_dragged.set(false);
            }
        }
        let Some(tb_el) = self.parent_text_box.borrow().upgrade() else {
            return;
        };
        let Some(tb) = tb_el.ext::<UITextBox2>() else {
            return;
        };
        let mut s = tb.state();
        let text_height = pz_ui::draw::font_height(s.font);
        let h = el.get_height() as i32;
        let lines = s.lines.len() as i32;
        if lines > s.num_visible_lines && !s.lines.is_empty() {
            let mut num_visible = s.num_visible_lines;
            if num_visible * text_height > tb_el.get_height() as i32 - s.inset() * 2 {
                num_visible -= 1;
            }
            let percent_shown = num_visible as f32 / lines as f32;
            self.button_inside_length
                .set(((h as f32 * percent_shown) as i32 - 12) as f32);
            self.button_inside_length
                .set(self.button_inside_length.get().max(0.0));
            let button_height = self.button_inside_length.get() + 12.0;
            if self.button_offset.get() < 0.0 {
                self.button_offset.set(0.0);
            }
            if self.button_offset.get() > h as f32 - button_height - 0.0 {
                self.button_offset.set(h as f32 - button_height - 0.0);
            }
            let percent_down = self.button_offset.get() / h as f32;
            s.top_line_index = (lines as f32 * percent_down) as i32;
            let free = h - button_height as i32;
            let unseen = text_height * (lines - num_visible);
            let ratio = free as f32 / unseen as f32;
            let pos = self.button_offset.get() / ratio;
            s.top_line_index = ((pos / text_height as f32) as i32).min(lines - num_visible - 1);
        } else {
            self.button_offset.set(0.0);
            self.button_inside_length.set((h - 12) as f32);
            s.top_line_index = 0;
        }
    }
}
