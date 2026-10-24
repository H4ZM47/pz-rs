//! Lua-facing wrappers for the engine objects `UIElement.Draw*` receives.
//!
//! pz-render has no `LuaObject` for its [`Texture`], and its `Texture` has
//! no trim offsets or original size, so this crate defines
//! [`UiTexture`] (class `zombie.core.textures.Texture`) carrying them.
//! [`ColorObject`] stands in for `zombie.core.Color`. Both should move to
//! (or be reconciled with) pz-render once it exposes these classes.

use pz_kahlua::LuaObject;
use pz_render::{Rgba, Texture};
use std::any::Any;
use std::cell::Cell;

/// `zombie.core.textures.Texture` as seen by the UI.
#[derive(Clone, Debug)]
pub struct UiTexture {
    pub tex: Texture,
    /// `Texture.offsetX/offsetY`: trim offset inside the original image.
    pub offset_x: f32,
    pub offset_y: f32,
    /// `getWidthOrig/getHeightOrig`: untrimmed size.
    pub width_orig: i32,
    pub height_orig: i32,
}

impl UiTexture {
    /// An untrimmed texture.
    pub fn new(tex: Texture) -> Self {
        let (w, h) = (tex.width as i32, tex.height as i32);
        UiTexture { tex, offset_x: 0.0, offset_y: 0.0, width_orig: w, height_orig: h }
    }
    /// `getWidth()` (trimmed width).
    pub fn width(&self) -> i32 {
        self.tex.width as i32
    }
    /// `getHeight()`.
    pub fn height(&self) -> i32 {
        self.tex.height as i32
    }
}

impl LuaObject for UiTexture {
    fn class_name(&self) -> &str {
        "zombie.core.textures.Texture"
    }
    fn to_lua_string(&self) -> String {
        format!("Texture{{ name:\"{}\", w:{}, h:{} }}", self.tex.name, self.tex.width, self.tex.height)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `zombie.core.Color` (mutable public fields `r, g, b, a`).
#[derive(Debug, Default)]
pub struct ColorObject {
    pub rgba: Cell<Rgba>,
}

impl ColorObject {
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        ColorObject { rgba: Cell::new(Rgba(r, g, b, a)) }
    }
}

impl LuaObject for ColorObject {
    fn class_name(&self) -> &str {
        "zombie.core.Color"
    }
    fn to_lua_string(&self) -> String {
        let Rgba(r, g, b, a) = self.rgba.get();
        format!("Color {r} {g} {b} {a}")
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
