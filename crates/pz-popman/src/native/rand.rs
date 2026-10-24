//! The native library's `Rand`: one libc++ `std::mt19937` (seeded from
//! `/dev/urandom` when the library loads) shared by both threads, drawn
//! through libc++'s `uniform_int_distribution<int>` for integers and directly
//! for floats.

const N: usize = 624;
const M: usize = 397;

/// libc++ `std::mersenne_twister_engine<uint32_t, 32, 624, 397, 31, ...>`.
#[derive(Clone)]
pub struct Mt19937 {
    state: Box<[u32; N]>,
    index: usize,
}

impl Mt19937 {
    /// `std::mt19937(seed)`.
    pub fn new(seed: u32) -> Mt19937 {
        let mut state = Box::new([0u32; N]);
        state[0] = seed;
        for i in 1..N {
            let prev = state[i - 1];
            state[i] = 1812433253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        Mt19937 { state, index: 0 }
    }

    /// The raw 624-word state and the position, as the library stores them.
    pub fn raw(&self) -> (&[u32; N], usize) {
        (&self.state, self.index)
    }

    /// Replaces the raw state (e.g. one copied out of the original library).
    pub fn set_raw(&mut self, state: &[u32; N], index: usize) {
        self.state.copy_from_slice(state);
        self.index = index % N;
    }

    /// One draw (libc++ generates lazily, one word at a time).
    pub fn next_u32(&mut self) -> u32 {
        let i = self.index;
        let j = (i + 1) % N;
        let y = (self.state[i] & 0x8000_0000) | (self.state[j] & 0x7fff_fffe);
        let mag = if self.state[j] & 1 != 0 { 0x9908_b0df } else { 0 };
        let v = mag ^ self.state[(i + M) % N] ^ (y >> 1);
        self.state[i] = v;
        self.index = j;
        let mut z = v ^ (v >> 11);
        z ^= (z << 7) & 0x9d2c_5680;
        z ^= (z << 15) & 0xefc6_0000;
        z ^ (z >> 18)
    }
}

/// `Rand` of the native library.
#[derive(Clone)]
pub struct NativeRand {
    pub mt: Mt19937,
}

impl NativeRand {
    pub fn new(seed: u32) -> NativeRand {
        NativeRand { mt: Mt19937::new(seed) }
    }

    /// libc++ `uniform_int_distribution<int>(a, b)(mt)`.
    fn uniform(&mut self, a: i32, b: i32) -> i32 {
        let range = (b as u32).wrapping_sub(a as u32);
        if range == 0 {
            return b;
        }
        let rp = range.wrapping_add(1);
        if rp == 0 {
            return self.mt.next_u32() as i32;
        }
        let lz = rp.leading_zeros();
        let w = if (rp << lz) & 0x7fff_ffff != 0 { 32 - lz } else { 31 - lz };
        let mask = if w == 0 { u32::MAX } else { u32::MAX >> (32 - w) };
        loop {
            let u = self.mt.next_u32() & mask;
            if u < rp {
                return a.wrapping_add(u as i32);
            }
        }
    }

    /// `Rand::Next(int max)`: `[0, max)`, or `max` itself when `max <= 0`.
    pub fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return max;
        }
        self.uniform(0, max - 1)
    }

    /// `Rand::Next(int a, int b)`: `[min, max)` of the two, `b` when equal.
    pub fn next_range(&mut self, a: i32, b: i32) -> i32 {
        if a == b {
            return b;
        }
        let lo = a.min(b);
        let hi = a.max(b);
        self.uniform(lo, hi - 1)
    }

    /// `Rand::Next(float a, float b)`.
    pub fn next_float(&mut self, a: f32, b: f32) -> f32 {
        if a == b {
            return a;
        }
        let (lo, hi) = if a > b { (b, a) } else { (a, b) };
        // `ucvtf s, w, #32`: one rounding of u / 2^32.
        let u = self.mt.next_u32() as f32 / 4_294_967_296.0_f32;
        (hi - lo).mul_add(u, lo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mt19937_reference_output() {
        // The 10000th output of a default-seeded mt19937 is 4123659995.
        let mut mt = Mt19937::new(5489);
        let mut v = 0;
        for _ in 0..10000 {
            v = mt.next_u32();
        }
        assert_eq!(v, 4_123_659_995);
    }

    #[test]
    fn next_bounds() {
        let mut r = NativeRand::new(1);
        for _ in 0..1000 {
            let v = r.next(7);
            assert!((0..7).contains(&v));
            let w = r.next_range(10, 3);
            assert!((3..10).contains(&w));
        }
        assert_eq!(r.next(0), 0);
        assert_eq!(r.next(-4), -4);
        assert_eq!(r.next_range(5, 5), 5);
        assert_eq!(r.next_float(2.0, 2.0), 2.0);
    }
}
