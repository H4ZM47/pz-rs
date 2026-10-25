//! Java subclasses of `UIElement` (`UI3DModel`, `UITextBox2`, `UIWorldMap`,
//! ...): an extension slot on [`UIElement`] whose hooks stand in for the
//! subclass's overrides of the virtual methods.
//!
//! A subclass is still the Rust type [`UIElement`] (so `AddChild`, `AddUI`,
//! the child lists and every `UIElement` binding accept it), created with
//! [`UIElement::new_ext`]. Its [`UIElementExt::class_name`] and
//! [`UIElementExt::super_classes`] are what Lua sees (metatable lookup,
//! `instanceof`). The element's public virtual methods (`render`, `update`,
//! the mouse handlers, `on_resize`) dispatch to the hooks; each hook's
//! default calls the base behaviour (`UIElement::*_base`, the Java
//! `super.*()`), which an override may call too.
//!
//! The extension is not inside the element's `RefCell`, so no borrow of
//! the element is held while a hook runs. Hooks must likewise drop their
//! own borrows before calling the base or Lua.

use crate::element::UIElement;
use pz_kahlua::Thread;
use std::any::Any;

/// The overridable part of a `UIElement` subclass.
pub trait UIElementExt: Any {
    /// The Java class, e.g. `zombie.ui.UI3DModel`.
    fn class_name(&self) -> &'static str;
    /// Superclasses and interfaces, nearest first; includes
    /// `zombie.ui.UIElement`.
    fn super_classes(&self) -> &'static [&'static str];
    fn as_any(&self) -> &dyn Any;

    /// `render()`.
    fn render(&self, el: &UIElement, t: &mut Thread) {
        el.render_base(t)
    }
    /// `update()`.
    fn update(&self, el: &UIElement, t: &mut Thread) {
        el.update_base(t)
    }
    /// `onResize()`.
    fn on_resize(&self, el: &UIElement, t: &mut Thread) {
        el.on_resize_base(t)
    }
    /// `onMouseDown(x, y)`.
    fn on_mouse_down(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        el.on_mouse_down_base(t, x, y)
    }
    /// `onMouseUp(x, y)`.
    fn on_mouse_up(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        el.on_mouse_up_base(t, x, y)
    }
    /// `onRightMouseDown(x, y)`.
    fn on_right_mouse_down(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        el.on_right_mouse_down_base(t, x, y)
    }
    /// `onRightMouseUp(x, y)`.
    fn on_right_mouse_up(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        el.on_right_mouse_up_base(t, x, y)
    }
    /// `onMouseMove(dx, dy)`.
    fn on_mouse_move(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) -> bool {
        el.on_mouse_move_base(t, dx, dy)
    }
    /// `onMouseMoveOutside(dx, dy)`.
    fn on_mouse_move_outside(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) {
        el.on_mouse_move_outside_base(t, dx, dy)
    }
    /// `onMouseWheel(del)`.
    fn on_mouse_wheel(&self, el: &UIElement, t: &mut Thread, del: f64) -> bool {
        el.on_mouse_wheel_base(t, del)
    }
}
