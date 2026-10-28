//! `zombie.iso.weather.ClimateColorInfo`: an interior and an exterior RGBA
//! colour. Lua holds these by reference (`ClimateMain.lua` configures the
//! climate by mutating the objects the getters return), so the climate keeps
//! them as [`ColorRef`]s.

use std::cell::RefCell;
use std::rc::Rc;

/// A shared, mutable `ClimateColorInfo` (Java object identity).
pub type ColorRef = Rc<RefCell<ClimateColorInfo>>;

/// `ClimateColorInfo`: interior then exterior RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateColorInfo {
    pub interior: [f32; 4],
    pub exterior: [f32; 4],
}

/// `new Color(0, 0, 0, 1)`'s alpha: the `int` constructor divides by 255.
const DEFAULT_ALPHA: f32 = 1.0 / 255.0;

impl Default for ClimateColorInfo {
    /// `new ClimateColorInfo()`: both `new Color(0, 0, 0, 1)` (the `int`
    /// constructor: black with alpha 1/255).
    fn default() -> Self {
        ClimateColorInfo { interior: [0.0, 0.0, 0.0, DEFAULT_ALPHA], exterior: [0.0, 0.0, 0.0, DEFAULT_ALPHA] }
    }
}

fn interp4(a: &[f32; 4], b: &[f32; 4], t: f32) -> [f32; 4] {
    // `Color.interp`: `dest = this + (to - this) * delta`.
    let mut o = [0.0; 4];
    for i in 0..4 {
        o[i] = a[i] + (b[i] - a[i]) * t;
    }
    o
}

impl ClimateColorInfo {
    /// `new ClimateColorInfo(r, g, b, a)`.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        ClimateColorInfo { interior: [r, g, b, a], exterior: [r, g, b, a] }
    }

    /// `new ClimateColorInfo(r, g, b, a, r2, g2, b2, a2)`: interior, then
    /// exterior.
    #[allow(clippy::too_many_arguments)]
    pub fn new8(r: f32, g: f32, b: f32, a: f32, r2: f32, g2: f32, b2: f32, a2: f32) -> Self {
        ClimateColorInfo { interior: [r, g, b, a], exterior: [r2, g2, b2, a2] }
    }

    pub fn shared(self) -> ColorRef {
        Rc::new(RefCell::new(self))
    }

    /// `interp(to, t, result)` as a value.
    pub fn interp(&self, to: &ClimateColorInfo, t: f32) -> ClimateColorInfo {
        ClimateColorInfo { interior: interp4(&self.interior, &to.interior, t), exterior: interp4(&self.exterior, &to.exterior, t) }
    }

    /// `scale(val)`: `Color.scale` multiplies all four channels.
    pub fn scale(&mut self, val: f32) {
        for c in self.interior.iter_mut().chain(self.exterior.iter_mut()) {
            *c *= val;
        }
    }
}

/// `a.interp(b, t, dest)` on shared colours (any of them may alias).
pub fn interp_into(a: &ColorRef, b: &ColorRef, t: f32, dest: &ColorRef) {
    let v = a.borrow().interp(&b.borrow(), t);
    *dest.borrow_mut() = v;
}

/// `dest.setTo(src)`.
pub fn set_to(dest: &ColorRef, src: &ColorRef) {
    let v = *src.borrow();
    *dest.borrow_mut() = v;
}
