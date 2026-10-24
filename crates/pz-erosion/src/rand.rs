//! `zombie.core.random.RandLocation` (a `RandAbstract` over the
//! per-location `java.util.Random` of `WorldGenParams.getRandom`).

use crate::java::JavaRandom;

/// `WorldGenParams.getRandom(wx, wy, offset)` for a world seed
/// (`WorldGenParams.seed`, the `hashCode()` of the seed string).
pub fn world_gen_random(world_seed: i32, wx: i32, wy: i32, offset: i64) -> JavaRandom {
    let mut r = JavaRandom::new((world_seed as i64).wrapping_add(offset));
    let wx_rnd = r.next_long();
    let wy_rnd = r.next_long();
    r.set_seed(
        (wx as i64).wrapping_mul(wx_rnd) ^ (wy as i64).wrapping_mul(wy_rnd) ^ world_seed as i64,
    );
    r
}

/// `WorldGenParams.setSeedString(s)`: the seed is `s.hashCode()`.
pub fn world_seed_from_string(s: &str) -> i32 {
    pz_world::java::string_hash(s)
}

/// `RandLocation`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RandLocation {
    pub rand: JavaRandom,
}

impl RandLocation {
    /// `new RandLocation(x, y)`.
    pub fn new(world_seed: i32, x: i32, y: i32) -> Self {
        RandLocation {
            rand: world_gen_random(world_seed, x, y, 0),
        }
    }

    /// `Next(int max)` (`RandAbstract`): 0 without drawing when `max <= 0`.
    pub fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        self.rand.next_int_bound(max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_per_location() {
        let mut a = RandLocation::new(0, 10, 20);
        let mut b = RandLocation::new(0, 10, 20);
        let mut c = RandLocation::new(0, 11, 20);
        let va: Vec<i32> = (0..8).map(|_| a.next(100)).collect();
        let vb: Vec<i32> = (0..8).map(|_| b.next(100)).collect();
        let vc: Vec<i32> = (0..8).map(|_| c.next(100)).collect();
        assert_eq!(va, vb);
        assert_ne!(va, vc);
        assert_eq!(a.next(0), 0);
        assert_eq!(world_seed_from_string(""), 0);
    }
}
