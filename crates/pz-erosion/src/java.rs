//! Java runtime semantics the erosion code depends on: `java.util.Random`,
//! `Float.toString`, `Integer.parseInt` / `Float.parseFloat`.

const MULTIPLIER: i64 = 0x5_DEEC_E66D;
const ADDEND: i64 = 0xB;
const MASK: i64 = (1 << 48) - 1;

/// `java.util.Random` (the legacy LCG; its `nextInt(bound)` override).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    /// `new Random(seed)`.
    pub fn new(seed: i64) -> Self {
        let mut r = JavaRandom { seed: 0 };
        r.set_seed(seed);
        r
    }

    /// A generator with the given internal (already scrambled) 48-bit state.
    pub fn from_state(state: i64) -> Self {
        JavaRandom { seed: state & MASK }
    }

    /// The internal 48-bit state.
    pub fn state(&self) -> i64 {
        self.seed
    }

    /// `setSeed(seed)`.
    pub fn set_seed(&mut self, seed: i64) {
        self.seed = (seed ^ MULTIPLIER) & MASK;
    }

    /// `next(bits)`.
    pub fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND)) & MASK;
        ((self.seed as u64) >> (48 - bits)) as i32
    }

    /// `nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    /// `nextInt(bound)`; `bound` must be positive (the original throws).
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(
            bound > 0,
            "IllegalArgumentException: bound must be positive"
        );
        let mut r = self.next(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.next(31);
        }
    }

    /// `nextLong()`.
    pub fn next_long(&mut self) -> i64 {
        ((self.next(32) as i64) << 32).wrapping_add(self.next(32) as i64)
    }

    /// `nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }
}

/// `String.valueOf(float)` / `Float.toString` (JDK 19+: the shortest decimal
/// that rounds to the value, which is also what Rust prints).
pub fn float_to_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    // Shortest digits and exponent: d.ddddE<exp>.
    let sci = format!("{:e}", f.abs());
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let mut digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    // Java breaks a tie between two equally close shortest decimals toward
    // the even digit; Rust's formatter may pick the other one.
    if digits.len() > 1 && digits.as_bytes()[digits.len() - 1] % 2 == 1 {
        let n = digits.len() as i32;
        let val = |d: &str| -> Option<f64> { format!("{d}e{}", exp - n + 1).parse().ok() };
        let last = digits.as_bytes()[digits.len() - 1];
        let exact = f.abs() as f64;
        for alt_last in [last - 1, last + 1] {
            if !alt_last.is_ascii_digit() {
                continue;
            }
            let mut alt = digits.clone().into_bytes();
            *alt.last_mut().unwrap() = alt_last;
            let alt = String::from_utf8(alt).unwrap();
            if let (Some(a), Some(c)) = (val(&alt), val(&digits)) {
                let (da, dc) = ((a - exact).abs(), (c - exact).abs());
                if a as f32 == f.abs() && (da - dc).abs() <= dc * 1e-9 {
                    digits = alt;
                    break;
                }
            }
        }
    }
    let sign = if f < 0.0 { "-" } else { "" };
    let a = f.abs();
    if (1e-3..1e7).contains(&a) {
        let n = digits.len() as i32;
        let point = exp + 1; // digits before the decimal point
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point >= n {
            format!("{}{}.0", digits, "0".repeat((point - n) as usize))
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        };
        format!("{sign}{s}")
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{}", &digits[..1], frac, exp)
    }
}

/// `Integer.parseInt(s)`; `None` is the `NumberFormatException`.
pub fn parse_int(s: &str) -> Option<i32> {
    pz_world::java::parse_int(s)
}

/// `Float.parseFloat(s)`; `None` is the `NumberFormatException`.
pub fn parse_float(s: &str) -> Option<f32> {
    pz_world::java::parse_float(s)
}

/// `String.trim()`.
pub fn trim(s: &str) -> &str {
    pz_world::java::java_trim(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_known_values() {
        // new Random(0).nextInt() is a widely documented constant.
        let mut r = JavaRandom::new(0);
        assert_eq!(r.next_int(), -1_155_484_576);
        let mut r = JavaRandom::new(42);
        let a = r.next_int_bound(10);
        assert!((0..10).contains(&a));
        let mut r2 = JavaRandom::from_state(JavaRandom::new(42).state());
        assert_eq!(r2.next_int_bound(10), a);
        let f = JavaRandom::new(1).next_float();
        assert!((0.0..1.0).contains(&f));
    }

    #[test]
    fn float_strings() {
        assert_eq!(float_to_string(12.5), "12.5");
        assert_eq!(float_to_string(0.39), "0.39");
        assert_eq!(float_to_string(1.0), "1.0");
        assert_eq!(float_to_string(100.0), "100.0");
        assert_eq!(float_to_string(0.001), "0.001");
        assert_eq!(float_to_string(0.0001), "1.0E-4");
        assert_eq!(float_to_string(1.0e7), "1.0E7");
        assert_eq!(float_to_string(-2.5e-5), "-2.5E-5");
        assert_eq!(float_to_string(0.0), "0.0");
        // 512697.125 lies halfway between the two shortest candidates.
        assert_eq!(float_to_string(f32::from_bits(1_224_365_860)), "512697.12");
    }
}
