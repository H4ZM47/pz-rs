//! `zombie.ui.NewWindow`: the Java dialog frame (title bar, sides, bottom,
//! optional close button, dragging by the title bar) that
//! `NewHealthPanel` and `UIDebugConsole` extend. [`WindowCore`] holds the
//! shared part; subclasses embed it and call its methods for `super`.

use super::speed_controls::HUDButton;
use pz_kahlua::Thread;
use pz_render::Texture;
use pz_ui::{UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const NEW_WINDOW: &str = "zombie.ui.NewWindow";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];

fn tex(n: &str) -> Option<Texture> {
    pz_render::get_shared_texture(n)
}
pub(crate) fn tw(t: &Option<Texture>) -> f64 {
    t.as_ref().map_or(0.0, |t| t.width as f64)
}
pub(crate) fn th(t: &Option<Texture>) -> f64 {
    t.as_ref().map_or(0.0, |t| t.height as f64)
}

/// `NewWindow`'s fields.
pub struct WindowCore {
    pub click_x: Cell<i32>,
    pub click_y: Cell<i32>,
    pub client_w: Cell<i32>,
    pub client_h: Cell<i32>,
    pub movable: Cell<bool>,
    pub moving: Cell<bool>,
    /// `nestedItems` (`Rectangle(l, t, r, b)` as x, y, width, height).
    pub nested: RefCell<Vec<[i32; 4]>>,
    pub resize_to_fit_y: Cell<bool>,
    pub alpha: Cell<f32>,
    pub dialog_bottom_left: Option<Texture>,
    pub dialog_bottom_middle: Option<Texture>,
    pub dialog_bottom_right: Option<Texture>,
    pub dialog_left: Option<Texture>,
    pub dialog_middle: Option<Texture>,
    pub dialog_right: Option<Texture>,
    pub title_left: Option<Texture>,
    pub title_middle: Option<Texture>,
    pub title_right: Option<Texture>,
    pub close_button: RefCell<Option<Rc<UIElement>>>,
}

impl WindowCore {
    /// The fields of `new NewWindow(x, y, width, height, bHasClose)`; the
    /// element part is [`WindowCore::init`].
    pub fn new() -> Self {
        WindowCore {
            click_x: Cell::new(0),
            click_y: Cell::new(0),
            client_w: Cell::new(0),
            client_h: Cell::new(0),
            movable: Cell::new(true),
            moving: Cell::new(false),
            nested: RefCell::default(),
            resize_to_fit_y: Cell::new(true),
            alpha: Cell::new(1.0),
            title_left: tex("media/ui/Dialog_Titlebar_Left.png"),
            title_middle: tex("media/ui/Dialog_Titlebar_Middle.png"),
            title_right: tex("media/ui/Dialog_Titlebar_Right.png"),
            dialog_left: tex("media/ui/Dialog_Left.png"),
            dialog_middle: tex("media/ui/Dialog_Middle.png"),
            dialog_right: tex("media/ui/Dialog_Right.png"),
            dialog_bottom_left: tex("media/ui/Dialog_Bottom_Left.png"),
            dialog_bottom_middle: tex("media/ui/Dialog_Bottom_Middle.png"),
            dialog_bottom_right: tex("media/ui/Dialog_Bottom_Right.png"),
            close_button: RefCell::new(None),
        }
    }

    /// The constructor's element part: position, the minimum size
    /// (156 x 78), the close button.
    pub fn init(&self, el: &UIElement, x: i32, y: i32, width: i32, height: i32, has_close: bool) {
        let (w, h) = (width.max(156), height.max(78));
        {
            let mut s = el.st_mut();
            s.x = x as f64;
            s.y = y as f64;
            s.width = w as f32;
            s.height = h as f32;
        }
        if has_close {
            let b = HUDButton::create(
                "close",
                (w - 16) as f64,
                2.0,
                "media/ui/inventoryPanes/Button_Close.png",
                "media/ui/inventoryPanes/Button_Close.png",
                el,
            );
            b.set_width(14.0);
            b.set_height(14.0);
            el.add_child(&b);
            *self.close_button.borrow_mut() = Some(b);
        }
        self.client_w.set(w);
        self.client_h.set(h);
    }

    /// `Nest(el, t, r, b, l)`.
    #[allow(clippy::too_many_arguments)]
    pub fn nest(
        &self,
        me: &UIElement,
        child: &UIElement,
        t: &mut Thread,
        top: i32,
        r: i32,
        b: i32,
        l: i32,
    ) {
        me.add_child(child);
        self.nested.borrow_mut().push([l, top, r, b]);
        child.set_x(l as f64);
        child.set_y(top as f64);
        child.update(t);
    }

    /// `ButtonClicked(name)`.
    pub fn button_clicked(&self, el: &UIElement, name: Option<Rc<str>>) {
        el.button_clicked_base(name.clone());
        if name.as_deref() == Some("close") {
            el.set_visible(false);
        }
    }

    /// `onMouseDown`: a press in the title bar starts a drag.
    pub fn on_mouse_down(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        el.on_mouse_down_base(t, x, y);
        if y < 18.0 {
            self.click_x.set(x as i32);
            self.click_y.set(y as i32);
            if self.movable.get() {
                self.moving.set(true);
            }
            el.set_capture(true);
        }
        true
    }

    /// `onMouseMove` (not consumed).
    pub fn on_mouse_move(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        el.on_mouse_move_base(t, dx, dy);
        if self.moving.get() {
            el.set_x(el.get_x() + dx);
            el.set_y(el.get_y() + dy);
        }
        false
    }

    /// `onMouseMoveOutside`.
    pub fn on_mouse_move_outside(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) {
        if !el.is_visible() {
            return;
        }
        el.on_mouse_move_outside_base(t, dx, dy);
        if self.moving.get() {
            el.set_x(el.get_x() + dx);
            el.set_y(el.get_y() + dy);
        }
    }

    /// `onMouseUp`: the drag ends.
    pub fn on_mouse_up(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        el.on_mouse_up_base(t, x, y);
        self.moving.set(false);
        el.set_capture(false);
        true
    }

    /// `render()`: the frame (title bar, sides, bottom) at 0.8 alpha, then
    /// the children.
    pub fn render(&self, el: &UIElement, t: &mut Thread) {
        // TODO(render): dialogRight's GL_NEAREST min filter.
        let alpha = (0.8 * self.alpha.get()) as f64;
        let mut y = 0.0;
        let title_h = th(&self.title_middle)
            .max(pz_ui::draw::font_height(Some(pz_text::UIFont::Small)) as f64);
        let w = el.get_width();
        let h = el.get_height();
        let d = |tx: &Option<Texture>, x: f64, y: f64| el.draw_texture(tx.as_ref(), x, y, alpha);
        let ds = |tx: &Option<Texture>, x: f64, y: f64, ww: f64, hh: f64| {
            el.draw_texture_scaled(tx.as_ref(), x, y, ww, hh, alpha)
        };
        d(&self.title_left, 0.0, y);
        d(&self.title_right, w - tw(&self.title_right), y);
        ds(
            &self.title_middle,
            tw(&self.title_left),
            y,
            w - tw(&self.title_left) * 2.0,
            title_h,
        );
        y += th(&self.title_right);
        let side_h = h - th(&self.title_left) - th(&self.dialog_bottom_left);
        ds(&self.dialog_left, 0.0, y, tw(&self.dialog_left), side_h);
        ds(
            &self.dialog_middle,
            tw(&self.dialog_left),
            y,
            w - tw(&self.dialog_right) * 2.0,
            side_h,
        );
        ds(
            &self.dialog_right,
            w - tw(&self.dialog_right),
            y,
            tw(&self.dialog_left),
            side_h,
        );
        let y = ((y as i32) as f64 + side_h) as i32 as f64;
        ds(
            &self.dialog_bottom_middle,
            tw(&self.dialog_bottom_left),
            y,
            w - tw(&self.dialog_bottom_left) * 2.0,
            th(&self.dialog_bottom_middle),
        );
        d(&self.dialog_bottom_left, 0.0, y);
        d(
            &self.dialog_bottom_right,
            w - tw(&self.dialog_bottom_right),
            y,
        );
        el.render_base(t);
    }

    /// `update()`: the close button's place, then the nested items' sizes
    /// (or, with `resizeToFitY`, the height from the tallest).
    pub fn update(&self, el: &UIElement, t: &mut Thread) {
        el.update_base(t);
        let close = self.close_button.borrow().clone();
        if let Some(b) = &close {
            b.set_x(3.0);
            b.set_y(3.0);
            b.set_width(15.0);
            b.set_height(15.0);
        }
        let nested = self.nested.borrow().clone();
        let mut n = 0;
        if !self.resize_to_fit_y.get() {
            for r in &nested {
                let Some(con) = el.control(n) else { break };
                if close.as_ref().is_some_and(|b| Rc::ptr_eq(b, &con)) {
                    // `continue` without `++n`: the original stays on the
                    // close button for the rest of the list.
                    continue;
                }
                let [x, y, w, h] = *r;
                con.set_x(x as f64);
                con.set_y(y as f64);
                con.set_width((self.client_w.get() - (x + w)) as f64);
                con.set_height((self.client_h.get() - (y + h)) as f64);
                con.onresize();
                n += 1;
            }
        } else {
            let mut height = 0.0f32;
            for _ in &nested {
                let Some(con) = el.control(n) else { break };
                if close.as_ref().is_some_and(|b| Rc::ptr_eq(b, &con)) {
                    continue;
                }
                if (height as f64) < con.get_height() {
                    height = (con.get_height() as i32) as f32;
                }
                n += 1;
            }
            el.st_mut().height = height + 50.0;
        }
    }
}

impl Default for WindowCore {
    fn default() -> Self {
        Self::new()
    }
}

/// `zombie.ui.NewWindow` itself.
pub struct NewWindow {
    pub core: WindowCore,
}

impl NewWindow {
    /// `new NewWindow(x, y, width, height, bHasClose)`.
    pub fn create(x: i32, y: i32, width: i32, height: i32, has_close: bool) -> Rc<UIElement> {
        let e = UIElement::new_ext(
            None,
            Box::new(NewWindow {
                core: WindowCore::new(),
            }),
        );
        e.ext::<NewWindow>()
            .unwrap()
            .core
            .init(&e, x, y, width, height, has_close);
        e
    }
}

impl UIElementExt for NewWindow {
    fn class_name(&self) -> &'static str {
        NEW_WINDOW
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
    fn on_mouse_down(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        self.core.on_mouse_down(el, t, x, y)
    }
    fn on_mouse_move(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) -> bool {
        self.core.on_mouse_move(el, t, dx, dy)
    }
    fn on_mouse_move_outside(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) {
        self.core.on_mouse_move_outside(el, t, dx, dy)
    }
    fn on_mouse_up(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        self.core.on_mouse_up(el, t, x, y)
    }
    fn render(&self, el: &UIElement, t: &mut Thread) {
        self.core.render(el, t)
    }
    fn update(&self, el: &UIElement, t: &mut Thread) {
        self.core.update(el, t)
    }
}
