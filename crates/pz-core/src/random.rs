//! `zombie.core.random.Rand` (the `RandStandard` instance) with
//! `RandAbstract`'s range handling.
//!
//! TODO(Rand): the original's generator is uncommons-maths'
//! `CellularAutomatonRNG` seeded from `SecureRandom`, so its sequence is
//! unpredictable; this uses `java.util.Random`'s LCG seeded from the clock.
//! Callers only depend on the range semantics, which are ported exactly.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

const MULT: i64 = 0x5DEECE66D;
const MASK: i64 = (1 << 48) - 1;

/// `java.util.Random`.
pub struct JavaRandom {
    seed: Cell<i64>,
}

impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        JavaRandom { seed: Cell::new((seed ^ MULT) & MASK) }
    }

    fn next(&self, bits: u32) -> i32 {
        let s = (self.seed.get().wrapping_mul(MULT).wrapping_add(0xB)) & MASK;
        self.seed.set(s);
        (s >> (48 - bits)) as i32
    }

    /// `nextInt(bound)` for `bound > 0`.
    pub fn next_int(&self, bound: i32) -> i32 {
        debug_assert!(bound > 0);
        if bound.count_ones() == 1 {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    /// `nextFloat()`.
    pub fn next_float(&self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }
}

/// `RandAbstract` over one generator.
pub struct Rand {
    id: Cell<i32>,
    rand: JavaRandom,
}

impl Rand {
    pub fn new(seed: i64) -> Self {
        Rand { id: Cell::new(0), rand: JavaRandom::new(seed) }
    }

    fn bump(&self) {
        let id = self.id.get() + 1;
        self.id.set(if id >= 10000 { 0 } else { id });
    }

    /// `Next(int max)`: `[0, max)`, 0 when `max <= 0`.
    pub fn next(&self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        self.bump();
        self.rand.next_int(max)
    }

    /// `Next(int min, int max)`: `[min, max)` with the bounds swapped if
    /// reversed; `min` when equal.
    pub fn next_range(&self, min: i32, max: i32) -> i32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        self.bump();
        self.rand.next_int(max.wrapping_sub(min)) + min
    }

    /// `Next(float min, float max)`.
    pub fn next_float(&self, min: f32, max: f32) -> f32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        self.bump();
        min + self.rand.next_float() * (max - min)
    }
}

thread_local! {
    static STANDARD: Rand = Rand::new(
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or(0),
    );
}

/// `Rand.Next(int max)` on `RandStandard.INSTANCE`.
pub fn next(max: i32) -> i32 {
    STANDARD.with(|r| r.next(max))
}

/// `Rand.Next(int min, int max)`.
pub fn next_range(min: i32, max: i32) -> i32 {
    STANDARD.with(|r| r.next_range(min, max))
}

/// `Rand.Next(float min, float max)`.
pub fn next_float(min: f32, max: f32) -> f32 {
    STANDARD.with(|r| r.next_float(min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_random_sequence() {
        // Values `java.util.Random` produces for these seeds.
        let r = JavaRandom::new(42);
        assert_eq!([r.next_int(10), r.next_int(10), r.next_int(10)], [0, 3, 8]);
        assert_eq!(JavaRandom::new(0).next_int(16), 11);
        assert_eq!(JavaRandom::new(7).next_float(), 0.730_699);
    }

    #[test]
    fn ranges() {
        let r = Rand::new(1);
        assert_eq!(r.next(0), 0);
        assert_eq!(r.next(-3), 0);
        assert_eq!(r.next_range(5, 5), 5);
        for _ in 0..100 {
            let v = r.next_range(7, 3);
            assert!((3..7).contains(&v));
            assert!((0..3).contains(&r.next(3)));
        }
    }
}
