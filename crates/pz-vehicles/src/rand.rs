//! `zombie.core.random.Rand` (`RandStandard.INSTANCE`) and `RandLua`
//! (`ZombRand`, `ZombRandFloat`): two `RandAbstract` streams over
//! `java.util.Random`.
//!
//! The original seeds `CellularAutomatonRNG` from `SecureRandom`, so its
//! sequence cannot be reproduced; both streams here are `java.util.Random`
//! (what a parity harness installs in the original with
//! `RandStandard.INSTANCE.rand = new Random(seed)`), seeded with
//! [`seed_standard`] and [`seed_lua`].

use std::cell::Cell;

const MULT: i64 = 0x5DEECE66D;
const MASK: i64 = (1 << 48) - 1;

/// `java.util.Random`.
#[derive(Clone, Debug)]
pub struct JavaRandom {
    seed: Cell<i64>,
}

impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        JavaRandom {
            seed: Cell::new((seed ^ MULT) & MASK),
        }
    }

    fn next(&self, bits: u32) -> i32 {
        let s = (self.seed.get().wrapping_mul(MULT).wrapping_add(0xB)) & MASK;
        self.seed.set(s);
        (s >> (48 - bits)) as i32
    }

    /// `nextInt(bound)`; panics like Java (`IllegalArgumentException`) on
    /// `bound <= 0`.
    pub fn next_int(&self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        if (bound as u32).is_power_of_two() {
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

/// `RandAbstract` over one `java.util.Random`.
#[derive(Debug)]
pub struct RandStream {
    id: Cell<i32>,
    rand: std::cell::RefCell<JavaRandom>,
    draws: Cell<u64>,
}

impl RandStream {
    pub fn new(seed: i64) -> Self {
        RandStream {
            id: Cell::new(0),
            rand: std::cell::RefCell::new(JavaRandom::new(seed)),
            draws: Cell::new(0),
        }
    }

    pub fn reseed(&self, seed: i64) {
        *self.rand.borrow_mut() = JavaRandom::new(seed);
        self.draws.set(0);
    }

    fn bump(&self) {
        let id = self.id.get() + 1;
        self.id.set(if id >= 10000 { 0 } else { id });
        self.draws.set(self.draws.get() + 1);
    }

    /// Number of draws since seeding (for diagnostics).
    pub fn draws(&self) -> u64 {
        self.draws.get()
    }

    /// `Next(int max)`: 0 without drawing when `max <= 0`.
    pub fn next(&self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        self.bump();
        self.rand.borrow().next_int(max)
    }

    /// `Next(long max)` (the original narrows to int).
    pub fn next_long(&self, max: i64) -> i64 {
        self.next(max as i32) as i64
    }

    /// `Next(int min, int max)`.
    pub fn next_range(&self, min: i32, max: i32) -> i32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        self.bump();
        self.rand.borrow().next_int(max.wrapping_sub(min)) + min
    }

    /// `Next(float min, float max)`.
    pub fn next_float(&self, min: f32, max: f32) -> f32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        self.bump();
        min + self.rand.borrow().next_float() * (max - min)
    }

    /// `NextBool(int invProbability)`: `Next(invProbability) == 0`.
    pub fn next_bool(&self, inv_probability: i32) -> bool {
        self.next(inv_probability) == 0
    }
}

thread_local! {
    static STANDARD: RandStream = RandStream::new(0);
    static LUA: RandStream = RandStream::new(0);
    static LOCK_FPS: Cell<i32> = const { Cell::new(60) };
}

/// Seeds `Rand` (`RandStandard.INSTANCE`).
pub fn seed_standard(seed: i64) {
    STANDARD.with(|r| r.reseed(seed));
}

/// Seeds `RandLua.INSTANCE` (`ZombRand`).
pub fn seed_lua(seed: i64) {
    LUA.with(|r| r.reseed(seed));
}

/// `PerformanceSettings.getLockFPS()` as `AdjustForFramerate` reads it.
pub fn set_lock_fps(fps: i32) {
    LOCK_FPS.with(|f| f.set(fps));
}

/// Runs `f` on the `Rand` stream.
pub fn standard<R>(f: impl FnOnce(&RandStream) -> R) -> R {
    STANDARD.with(f)
}

/// Runs `f` on the `RandLua` stream.
pub fn lua<R>(f: impl FnOnce(&RandStream) -> R) -> R {
    LUA.with(f)
}

/// `Rand.Next(int)`.
pub fn next(max: i32) -> i32 {
    standard(|r| r.next(max))
}

/// `Rand.Next(int, int)`.
pub fn next_range(min: i32, max: i32) -> i32 {
    standard(|r| r.next_range(min, max))
}

/// `Rand.Next(float, float)`.
pub fn next_float(min: f32, max: f32) -> f32 {
    standard(|r| r.next_float(min, max))
}

/// `Rand.NextBool(int)`.
pub fn next_bool(inv_probability: i32) -> bool {
    standard(|r| r.next_bool(inv_probability))
}

/// `Rand.AdjustForFramerate(chance)` outside a server:
/// `(int)((float)chance * ((float)lockFPS / 30.0f))`.
pub fn adjust_for_framerate(chance: i32) -> i32 {
    let fps = LOCK_FPS.with(|f| f.get());
    (chance as f32 * (fps as f32 / 30.0)) as i32
}

/// `ZombRand(double max)`.
pub fn zomb_rand(max: f64) -> f64 {
    if max == 0.0 {
        return 0.0;
    }
    if max < 0.0 {
        return -(lua(|r| r.next_long(-(max as i64))) as f64);
    }
    lua(|r| r.next_long(max as i64)) as f64
}

/// `ZombRand(double min, double max)`.
pub fn zomb_rand_range(min: f64, max: f64) -> f64 {
    lua(|r| r.next_range(min as i32, max as i32)) as f64
}

/// `ZombRandFloat(float min, float max)`.
pub fn zomb_rand_float(min: f32, max: f32) -> f32 {
    lua(|r| r.next_float(min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_random_sequence() {
        let r = JavaRandom::new(42);
        assert_eq!([r.next_int(10), r.next_int(10), r.next_int(10)], [0, 3, 8]);
        assert_eq!(JavaRandom::new(0).next_int(16), 11);
    }
}
