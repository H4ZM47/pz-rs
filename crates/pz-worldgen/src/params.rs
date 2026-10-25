//! `zombie.iso.worldgen.WorldGenParams`: the world seed and the per-position
//! random generators derived from it.

use crate::java::{string_hash, JavaRandom};

/// `WorldGenParams.GENERATION_SIZE`: chunks are generated in blocks of
/// `2 × 2`.
pub const GENERATION_SIZE: i32 = 2;
/// `WorldGenParams.GENERATION_SQUARES`: squares per side of a block.
pub const GENERATION_SQUARES: i32 = 16;

/// `WorldGenParams` (seed and procedural-cell bounds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldGenParams {
    pub seed_string: String,
    /// `seedString.hashCode()`.
    pub seed: i32,
    pub min_x_cell: i32,
    pub min_y_cell: i32,
    pub max_x_cell: i32,
    pub max_y_cell: i32,
}

impl WorldGenParams {
    /// `setSeedString(seed)` with the default ±250 cell bounds.
    pub fn new(seed_string: &str) -> Self {
        WorldGenParams {
            seed_string: seed_string.to_string(),
            seed: string_hash(seed_string),
            min_x_cell: -250,
            min_y_cell: -250,
            max_x_cell: 250,
            max_y_cell: 250,
        }
    }

    /// `getSeed()`.
    pub fn seed(&self) -> i64 {
        self.seed as i64
    }

    /// `getRandom(wx, wy, offset)`.
    pub fn random(&self, wx: i32, wy: i32, offset: i64) -> JavaRandom {
        let seed = self.seed as i64;
        let mut r = JavaRandom::new(seed.wrapping_add(offset));
        let wx_rnd = r.next_long();
        let wy_rnd = r.next_long();
        r.set_seed((wx as i64).wrapping_mul(wx_rnd) ^ (wy as i64).wrapping_mul(wy_rnd) ^ seed);
        r
    }
}

/// `WorldGenUtils.getCornerOfGeneration(b)`: the first chunk coordinate of
/// the generation block containing `b`.
pub fn corner_of_generation(b: i32) -> i32 {
    b.div_euclid(GENERATION_SIZE) * GENERATION_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners() {
        assert_eq!(corner_of_generation(5), 4);
        assert_eq!(corner_of_generation(-1), -2);
        assert_eq!(corner_of_generation(0), 0);
    }

    #[test]
    fn random_depends_on_position() {
        let p = WorldGenParams::new("abc");
        assert_eq!(p.seed, 96354);
        let a = p.random(1, 2, 0).next_long();
        let b = p.random(1, 2, 0).next_long();
        let c = p.random(2, 1, 0).next_long();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
