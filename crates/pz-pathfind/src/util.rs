//! Small numeric helpers shared by the whole pathfinder (the native
//! library's `Util.cpp`: `fastfloor`, `coorddivision`, `coordmodulo`,
//! `clamp`, `lerp`) plus the `IsoDirections` table.
//!
//! Distances are computed in `f32` the way the arm64 build does it: the
//! squared y difference is multiplied first and the x (and z) terms are
//! fused onto it with `fmadd`. [`dist2`] and [`dist3`] reproduce that exact
//! evaluation order so results are bit-identical.

/// `fastfloor(float)`: truncate, then step down for negative fractions.
#[inline]
pub fn fastfloor(x: f32) -> i32 {
    let i = x as i32;
    i - ((x < i as f32) as i32)
}

/// `coorddivision(int, int)`: floor division done through a float divide,
/// exactly like the native helper (so it loses precision the same way for
/// very large coordinates).
#[inline]
pub fn coorddivision(value: i32, divisor: i32) -> i32 {
    fastfloor(value as f32 / divisor as f32)
}

/// `coordmodulo(int, int)`: positive modulo built on [`coorddivision`].
#[inline]
pub fn coordmodulo(value: i32, divisor: i32) -> i32 {
    value - coorddivision(value, divisor) * divisor
}

/// `clamp(float, float, float)`.
#[inline]
pub fn clamp(v: f32, min: f32, max: f32) -> f32 {
    let mut v = v;
    if v < min {
        v = min;
    }
    if v > max {
        v = max;
    }
    v
}

/// The native `lerp(a, b, t)`. Note the operand order of the shipped
/// build: it evaluates `b + (a - b) * t` as one fused multiply-add, so
/// `t == 0` yields `b`.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (a - b).mul_add(t, b)
}

/// `sqrt(dx*dx + dy*dy)` evaluated as `fmadd(dx, dx, dy*dy)`.
#[inline]
pub fn dist2(dx: f32, dy: f32) -> f32 {
    dx.mul_add(dx, dy * dy).sqrt()
}

/// `sqrt(dx*dx + dy*dy + dz*dz)` evaluated as
/// `fmadd(dz, dz, fmadd(dx, dx, dy*dy))`.
#[inline]
pub fn dist3(dx: f32, dy: f32, dz: f32) -> f32 {
    dz.mul_add(dz, dx.mul_add(dx, dy * dy)).sqrt()
}

/// `IsoDirections`, in ordinal order. The native library stores the
/// direction of a sloped surface as one of these, or `Max` (ordinal 8)
/// when there is none; this crate uses `Option<Dir>` for that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Dir {
    N = 0,
    NW = 1,
    W = 2,
    SW = 3,
    S = 4,
    SE = 5,
    E = 6,
    NE = 7,
}

impl Dir {
    /// All directions in ordinal order (`IsoDirections::VALUES`).
    pub const ALL: [Dir; 8] = [
        Dir::N,
        Dir::NW,
        Dir::W,
        Dir::SW,
        Dir::S,
        Dir::SE,
        Dir::E,
        Dir::NE,
    ];

    /// Direction for an ordinal, `None` for 8 (`Max`) and anything else out
    /// of range.
    pub fn from_index(i: i32) -> Option<Dir> {
        if (0..8).contains(&i) {
            Some(Dir::ALL[i as usize])
        } else {
            None
        }
    }

    #[inline]
    pub fn index(self) -> i32 {
        self as i32
    }

    #[inline]
    pub fn dx(self) -> i32 {
        match self {
            Dir::N | Dir::S => 0,
            Dir::NW | Dir::W | Dir::SW => -1,
            Dir::SE | Dir::E | Dir::NE => 1,
        }
    }

    #[inline]
    pub fn dy(self) -> i32 {
        match self {
            Dir::W | Dir::E => 0,
            Dir::N | Dir::NW | Dir::NE => -1,
            Dir::SW | Dir::S | Dir::SE => 1,
        }
    }

    #[inline]
    pub fn rot180(self) -> Dir {
        Dir::ALL[((self as usize) + 4) & 7]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_modulo() {
        assert_eq!(fastfloor(1.5), 1);
        assert_eq!(fastfloor(-0.5), -1);
        assert_eq!(fastfloor(-1.0), -1);
        assert_eq!(coorddivision(-1, 8), -1);
        assert_eq!(coordmodulo(-1, 8), 7);
        assert_eq!(coordmodulo(17, 8), 1);
    }

    #[test]
    fn dirs() {
        assert_eq!(Dir::N.rot180(), Dir::S);
        assert_eq!(Dir::NE.rot180(), Dir::SW);
        assert_eq!((Dir::SE.dx(), Dir::SE.dy()), (1, 1));
        assert_eq!(Dir::from_index(8), None);
    }
}
