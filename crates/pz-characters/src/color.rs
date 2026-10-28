//! `zombie.core.ImmutableColor` and `zombie.core.Color` values, as the
//! descriptor statics and moodles use them. (`HumanVisual` is
//! `pz_inventory::visual::HumanVisual`.)

/// `zombie.core.ImmutableColor` (values only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImmutableColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl ImmutableColor {
    /// `ImmutableColor.white`.
    pub const WHITE: ImmutableColor = ImmutableColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    /// `new ImmutableColor(r, g, b, a)`: components capped at 1.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        ImmutableColor { r: r.min(1.0), g: g.min(1.0), b: b.min(1.0), a: a.min(1.0) }
    }

    /// The colour without alpha, as `HumanVisual` stores it.
    pub fn rgb(self) -> [f32; 3] {
        [self.r, self.g, self.b]
    }
}

/// `zombie.core.Color` (values only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// `new Color(r, g, b, a)`: components clamped to `[0, 1]`.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        let c = |v: f32| v.clamp(0.0, 1.0);
        Color { r: c(r), g: c(g), b: c(b), a: c(a) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors() {
        assert_eq!(ImmutableColor::new(2.0, 0.5, -1.0, 1.0), ImmutableColor { r: 1.0, g: 0.5, b: -1.0, a: 1.0 });
        assert_eq!(ImmutableColor::new(2.0, 0.5, -1.0, 1.0).rgb(), [1.0, 0.5, -1.0]);
        assert_eq!(Color::new(2.0, 0.5, -1.0, 1.0), Color { r: 1.0, g: 0.5, b: 0.0, a: 1.0 });
    }
}
