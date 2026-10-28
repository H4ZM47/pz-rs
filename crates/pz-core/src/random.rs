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
#[derive(Clone, Debug)]
pub struct JavaRandom {
    seed: Cell<i64>,
}

impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        JavaRandom { seed: Cell::new((seed ^ MULT) & MASK) }
    }

    /// `setSeed(seed)`.
    pub fn set_seed(&self, seed: i64) {
        self.seed.set((seed ^ MULT) & MASK);
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

    /// `nextInt()`.
    pub fn next_int_unbounded(&self) -> i32 {
        self.next(32)
    }

    /// `nextDouble()`.
    pub fn next_double(&self) -> f64 {
        let hi = (self.next(26) as i64) << 27;
        let lo = self.next(27) as i64;
        (hi + lo) as f64 * (1.0f64 / (1u64 << 53) as f64)
    }

    /// `nextFloat()`.
    pub fn next_float(&self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }
}

/// `RandAbstract` over one generator.
#[derive(Clone, Debug)]
pub struct Rand {
    id: Cell<i32>,
    rand: JavaRandom,
}

impl Rand {
    pub fn new(seed: i64) -> Self {
        Rand { id: Cell::new(0), rand: JavaRandom::new(seed) }
    }

    /// Restarts the sequence from `seed` (`Rand.init`'s seeding; tests use
    /// it on [`with_standard`] for a reproducible `RandStandard`).
    pub fn set_seed(&self, seed: i64) {
        self.id.set(0);
        self.rand.set_seed(seed);
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

    /// The generator's own `nextInt()` (no range handling).
    pub fn next_raw_int(&self) -> i32 {
        self.rand.next_int_unbounded()
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

/// Runs `f` with `RandStandard.INSTANCE` itself, for code that takes a
/// generator (so tests can pass a seeded one) but must draw from the
/// shared sequence in the game.
pub fn with_standard<R>(f: impl FnOnce(&Rand) -> R) -> R {
    STANDARD.with(f)
}

/// Reseeds `RandStandard.INSTANCE` (see [`Rand::set_seed`]).
pub fn set_seed(seed: i64) {
    STANDARD.with(|r| r.set_seed(seed))
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

/// `zombie.util.LocationRNG`: xoroshiro128+ seeded through SplitMix64.
/// A new generator has all-zero state until [`LocationRng::set_seed`].
#[derive(Clone, Debug, Default)]
pub struct LocationRng {
    s0: Cell<i64>,
    s1: Cell<i64>,
    state: Cell<i64>,
}

impl LocationRng {
    /// `setSeed(seed)`.
    pub fn set_seed(&self, seed: i64) {
        self.state.set(seed);
        let s0 = self.next_split_mix64();
        self.s0.set(s0);
        let s1 = self.next_split_mix64();
        self.s1.set(s1);
    }

    /// `getSeed()`: the SplitMix64 state (the seed advanced twice).
    pub fn get_seed(&self) -> i64 {
        self.state.get()
    }

    fn next_split_mix64(&self) -> i64 {
        let z = self.state.get().wrapping_add(-7046029254386353131);
        self.state.set(z);
        let z = (z ^ ((z as u64) >> 30) as i64).wrapping_mul(-4658895280553007687);
        let z = (z ^ ((z as u64) >> 27) as i64).wrapping_mul(-7723592293110705685);
        z ^ ((z as u64) >> 31) as i64
    }

    /// The private `nextInt()`: the low 32 bits of the next output.
    fn next_raw(&self) -> i32 {
        let s0 = self.s0.get();
        let s1 = self.s1.get() ^ s0;
        let result = s0.wrapping_add(self.s1.get());
        self.s0.set(s0.rotate_left(55) ^ s1 ^ (s1 << 14));
        self.s1.set(s1.rotate_left(36));
        result as i32
    }

    /// `nextFloat()`: `[0, 1)` from the top 24 bits of `nextInt()`.
    pub fn next_float(&self) -> f32 {
        ((self.next_raw() as u32) >> 8) as i32 as f32 * f32::from_bits(0x3380_0000)
    }

    /// `nextInt(n)`.
    pub fn next_int(&self, n: i32) -> i32 {
        let r = ((self.next_raw() as u32) >> 1) as i64;
        (r.wrapping_mul(n as i64) >> 31) as i32
    }

    /// `nextInt(n, x, y, z)`: reseeds from the location first.
    pub fn next_int_at(&self, n: i32, x: i32, y: i32, z: i32) -> i32 {
        self.set_seed(((z as i64) << 16) | ((y as i64) << 32) | x as i64);
        self.next_int(n)
    }
}

/// `LocationRNG.instance`, the shared generator (`TileOverlays`,
/// `ContainerOverlays`, ...), one per thread.
pub mod location_rng {
    use super::LocationRng;

    thread_local! {
        static INSTANCE: LocationRng = LocationRng::default();
    }

    /// Run `f` on `LocationRNG.instance`.
    pub fn with_instance<R>(f: impl FnOnce(&LocationRng) -> R) -> R {
        INSTANCE.with(f)
    }
}

/// `zombie.core.skinnedmodel.population.OutfitRNG`: a per-thread
/// [`LocationRng`] for outfit, hair and decal choices.
pub mod outfit_rng {
    use super::LocationRng;

    thread_local! {
        static RNG: LocationRng = LocationRng::default();
    }

    /// `setSeed(seed)`.
    pub fn set_seed(seed: i64) {
        RNG.with(|r| r.set_seed(seed));
    }

    /// `getSeed()`.
    pub fn get_seed() -> i64 {
        RNG.with(|r| r.get_seed())
    }

    /// `Next(int max)`.
    pub fn next(max: i32) -> i32 {
        RNG.with(|r| r.next_int(max))
    }

    /// `Next(int min, int max)`: `[min, max)`, bounds swapped if reversed.
    pub fn next_range(min: i32, max: i32) -> i32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        RNG.with(|r| r.next_int(max.wrapping_sub(min))).wrapping_add(min)
    }

    /// `Next(float min, float max)`.
    pub fn next_float(min: f32, max: f32) -> f32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        min + RNG.with(|r| r.next_float()) * (max - min)
    }

    /// `NextBool(invProbability)`.
    pub fn next_bool(inv_probability: i32) -> bool {
        next(inv_probability) == 0
    }

    /// `pickRandom(list)`: `None` for an empty list; a single element is
    /// returned without a draw.
    pub fn pick_random<T>(list: &[T]) -> Option<&T> {
        match list.len() {
            0 => None,
            1 => list.first(),
            n => list.get(next(n as i32) as usize),
        }
    }

    /// `randomImmutableColor(noBlack)`; `no_black` is
    /// `SandboxOptions.noBlackClothes || noBlack`. RGB through
    /// `Color.HSBtoRGB`.
    pub fn random_immutable_color(no_black: bool) -> [f32; 3] {
        let hue = next_float(0.0, 1.0);
        let saturation = next_float(0.0, 0.6);
        let min_brightness = if no_black { 0.2 } else { 0.1 };
        let brightness = next_float(min_brightness, 0.9);
        hsb_to_rgb(hue, saturation, brightness)
    }

    /// `zombie.core.Color.HSBtoRGB(hue, saturation, brightness)`.
    pub fn hsb_to_rgb(hue: f32, saturation: f32, brightness: f32) -> [f32; 3] {
        let c = |v: f32| (v * 255.0 + 0.5) as i32;
        let (r, g, b) = if saturation == 0.0 {
            let v = c(brightness);
            (v, v, v)
        } else {
            let h = (hue - (hue as f64).floor() as f32) * 6.0;
            let f = h - (h as f64).floor() as f32;
            let p = brightness * (1.0 - saturation);
            let q = brightness * (1.0 - saturation * f);
            let t = brightness * (1.0 - saturation * (1.0 - f));
            match h as i32 {
                0 => (c(brightness), c(t), c(p)),
                1 => (c(q), c(brightness), c(p)),
                2 => (c(p), c(brightness), c(t)),
                3 => (c(p), c(q), c(brightness)),
                4 => (c(t), c(p), c(brightness)),
                5 => (c(brightness), c(p), c(q)),
                _ => (0, 0, 0),
            }
        };
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_rng_matches_the_original() {
        // Values printed by the original `OutfitRNG` for seed 12345.
        outfit_rng::set_seed(12345);
        let ints: Vec<i32> = (0..5).map(|_| outfit_rng::next(1000)).collect();
        assert_eq!(ints, [628, 190, 260, 525, 934]);
        let r = LocationRng::default();
        r.set_seed(12345);
        assert_eq!(r.get_seed(), 4354685564936857699);
    }

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
