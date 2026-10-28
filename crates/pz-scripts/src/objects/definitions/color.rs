//! The data of `zombie.core.Color` that scripts store.

/// `zombie.core.Color`: `new Color()` is black with full alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for Color {
    fn default() -> Self {
        Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
    }
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    /// `set(r, g, b, a)`.
    pub fn set(&mut self, r: f32, g: f32, b: f32, a: f32) {
        *self = Color { r, g, b, a };
    }
}
