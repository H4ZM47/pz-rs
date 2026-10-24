//! Lua-facing wrappers for the engine objects `UIElement.Draw*` receives.
//!
//! Textures arrive as [`pz_render::LuaTexture`]. [`ColorObject`] stands in
//! for `zombie.core.Color`.

use pz_kahlua::LuaObject;
use pz_render::Rgba;
use std::any::Any;
use std::cell::Cell;

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
