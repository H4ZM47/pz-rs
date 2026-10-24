//! `zombie.core.random.Rand` as the animator calls it.
//!
//! [`AnimRandom`] follows pz-world's `GameRand` convention (`next`,
//! `next_float`, and the `RandAbstract` range helpers), so the engine can
//! hand the animator the same generator the rest of the game draws from.
//! [`JavaRandom`] is `java.util.Random`. The game's `RandStandard` wraps an
//! uncommons-maths `CellularAutomatonRNG` seeded from `SecureRandom`, whose
//! stream is unpredictable anyway; only the range semantics are fixed.

/// `RandAbstract` over a generator.
pub trait AnimRandom {
    /// The generator's `nextInt(bound)` for `bound > 0`.
    fn next_int_raw(&mut self, bound: i32) -> i32;

    /// The generator's `nextFloat()` in `[0, 1)`.
    fn next_float(&mut self) -> f32;

    /// `Rand.Next(int max)`: `[0, max)`, 0 without drawing when `max <= 0`.
    fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        self.next_int_raw(max)
    }

    /// `Rand.Next(int min, int max)`.
    fn next_range(&mut self, min: i32, max: i32) -> i32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        self.next_int_raw(max.wrapping_sub(min)).wrapping_add(min)
    }

    /// `Rand.Next(float min, float max)`.
    fn next_float_range(&mut self, min: f32, max: f32) -> f32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        min + self.next_float() * (max - min)
    }
}

const MULT: i64 = 0x5DEECE66D;
const MASK: i64 = (1 << 48) - 1;

/// `java.util.Random`.
#[derive(Clone, Debug)]
pub struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    /// `new Random(seed)`.
    pub fn new(seed: i64) -> Self {
        JavaRandom { seed: (seed ^ MULT) & MASK }
    }

    fn bits(&mut self, bits: u32) -> i32 {
        self.seed = self.seed.wrapping_mul(MULT).wrapping_add(0xB) & MASK;
        (self.seed >> (48 - bits)) as i32
    }
}

impl Default for JavaRandom {
    fn default() -> Self {
        JavaRandom::new(0)
    }
}

impl AnimRandom for JavaRandom {
    fn next_int_raw(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        if bound.count_ones() == 1 {
            return ((bound as i64 * self.bits(31) as i64) >> 31) as i32;
        }
        loop {
            let b = self.bits(31);
            let val = b % bound;
            if b.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}
