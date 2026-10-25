//! Java runtime behaviour the world generator depends on bit for bit:
//! `java.util.Random`, the `"^" + pattern` wildcard regexes the original
//! builds with `String.replace`, and `java.util.HashMap` iteration order.

use std::cell::RefCell;
use std::collections::HashMap;

use regex::Regex;

const MULTIPLIER: u64 = 0x5DEECE66D;
const MASK: u64 = (1 << 48) - 1;

/// `java.util.Random` (the LCG, including the JDK 17+ `nextInt(origin,
/// bound)` default method of `RandomGenerator`).
#[derive(Clone, Debug)]
pub struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    /// `new Random(seed)`.
    pub fn new(seed: i64) -> Self {
        JavaRandom {
            seed: (seed as u64 ^ MULTIPLIER) & MASK,
        }
    }

    /// `setSeed(seed)`.
    pub fn set_seed(&mut self, seed: i64) {
        self.seed = (seed as u64 ^ MULTIPLIER) & MASK;
    }

    /// `next(bits)`.
    pub fn next_bits(&mut self, bits: u32) -> i32 {
        self.seed = self.seed.wrapping_mul(MULTIPLIER).wrapping_add(0xB) & MASK;
        (self.seed >> (48 - bits)) as i64 as i32
    }

    /// `nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        self.next_bits(32)
    }

    /// `nextInt(bound)`; `bound` must be positive (the original throws).
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let mut r = self.next_bits(31);
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
            u = self.next_bits(31);
        }
    }

    /// `nextInt(origin, bound)` (`RandomSupport.boundedNextInt`), which
    /// draws differently from `nextInt(bound)`.
    pub fn next_int_range(&mut self, origin: i32, bound: i32) -> i32 {
        assert!(origin < bound, "bound must be greater than origin");
        let mut r = self.next_int();
        let n = bound.wrapping_sub(origin);
        let m = n.wrapping_sub(1);
        if n & m == 0 {
            r = (r & m).wrapping_add(origin);
        } else if n > 0 {
            let mut u = ((r as u32) >> 1) as i32;
            loop {
                r = u % n;
                if u.wrapping_add(m).wrapping_sub(r) >= 0 {
                    break;
                }
                u = ((self.next_int() as u32) >> 1) as i32;
            }
            r = r.wrapping_add(origin);
        } else {
            while r < origin || r >= bound {
                r = self.next_int();
            }
        }
        r
    }

    /// `nextLong()`.
    pub fn next_long(&mut self) -> i64 {
        let hi = self.next_bits(32) as i64;
        let lo = self.next_bits(32) as i64;
        (hi << 32).wrapping_add(lo)
    }

    /// `nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        self.next_bits(24) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    /// `nextDouble()`.
    pub fn next_double(&mut self) -> f64 {
        let hi = (self.next_bits(26) as i64) << 27;
        let lo = self.next_bits(27) as i64;
        (hi + lo) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

/// `String.hashCode()`.
pub fn string_hash(s: &str) -> i32 {
    pz_world::java::string_hash(s)
}

/// Iteration order of a `java.util.HashMap<String, _>` filled with `keys` in
/// order (default capacity, no removals). Returns indices into `keys`.
pub fn hashmap_order(keys: &[String]) -> Vec<usize> {
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let ordered = pz_world::java::hashmap_order(&refs);
    let mut used = vec![false; keys.len()];
    ordered
        .iter()
        .map(|k| {
            let i = keys
                .iter()
                .enumerate()
                .position(|(i, x)| !used[i] && x == k)
                .expect("key");
            used[i] = true;
            i
        })
        .collect()
}

/// The regex the original builds from a wildcard pattern:
/// `"^" + p` with `.` escaped, `*` → `.*` and `?` → `.?`, used with
/// `String.matches` (whole-string match).
pub fn wildcard_regex(pattern: &str) -> String {
    let s = format!("^{pattern}");
    let s = s.replace('.', "\\.");
    let s = s.replace('*', ".*");
    s.replace('?', ".?")
}

/// Compiled wildcard patterns, matched like `String.matches`.
#[derive(Default)]
pub struct WildcardCache {
    cache: RefCell<HashMap<String, Option<Regex>>>,
}

impl WildcardCache {
    /// `text.matches(wildcard_regex(pattern))`. A pattern that is not a
    /// valid regex never matches (the original would throw).
    pub fn matches(&self, pattern: &str, text: &str) -> bool {
        let mut c = self.cache.borrow_mut();
        let re = c
            .entry(pattern.to_string())
            .or_insert_with(|| Regex::new(&format!("^(?:{})$", wildcard_regex(pattern))).ok());
        re.as_ref().is_some_and(|r| r.is_match(text))
    }

    /// `text.matches(regex)` for a regex built by the caller.
    pub fn matches_regex(&self, regex: &str, text: &str) -> bool {
        let key = format!("\u{0}{regex}");
        let mut c = self.cache.borrow_mut();
        let re = c
            .entry(key)
            .or_insert_with(|| Regex::new(&format!("^(?:{regex})$")).ok());
        re.as_ref().is_some_and(|r| r.is_match(text))
    }
}

/// An exception the original would throw (and the chunk load would fail
/// with), such as a `NullPointerException` on incomplete definitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaException(pub String);

impl std::fmt::Display for JavaException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JavaException {}

pub type JResult<T> = Result<T, JavaException>;

pub fn npe(what: &str) -> JavaException {
    JavaException(format!("NullPointerException: {what}"))
}

/// `zombie.core.random.Rand` over a `java.util.Random`.
impl pz_world::GameRand for JavaRandom {
    fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            0
        } else {
            self.next_int_bound(max)
        }
    }
    fn next_float(&mut self) -> f32 {
        JavaRandom::next_float(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_matches_the_jdk() {
        // Values produced by the JDK for `new Random(12345)` calling, in
        // turn, nextInt(0,3), nextInt(0,4), nextInt(5), nextFloat(),
        // nextDouble(), nextLong().
        let mut r = JavaRandom::new(12345);
        let mut rows = Vec::new();
        for _ in 0..5 {
            rows.push((
                r.next_int_range(0, 3),
                r.next_int_range(0, 4),
                r.next_int_bound(5),
                r.next_float(),
                r.next_double(),
                r.next_long(),
            ));
        }
        assert_eq!(
            rows[0],
            (1, 1, 1, 0.9171147, 0.8330913489710237, 6022414958441676900)
        );
        assert_eq!(
            rows[1],
            (
                1,
                2,
                2,
                0.98805076,
                0.4480776326931518,
                -6674900032466492448
            )
        );
        assert_eq!(rows[4].0, 0);
        assert_eq!(rows[4].5, -8201417328859773479);
        assert_eq!(string_hash("pzrs-pathfind-oracle"), -1116860725);
    }

    #[test]
    fn wildcards() {
        let c = WildcardCache::default();
        assert!(c.matches("blends_natural_01_*", "blends_natural_01_12"));
        assert!(!c.matches("blends_natural_01_*", "xblends_natural_01_12"));
        assert!(c.matches("blends_natural_01_0", "blends_natural_01_0"));
        assert!(!c.matches("blends_natural_01_0", "blends_natural_01_00"));
        assert!(c.matches("Road_*", "Road_01"));
        assert!(c.matches("a?c", "ac"));
        assert!(!c.matches("a.c", "abc"));
    }

    #[test]
    fn hashmap_order_indices() {
        let keys = vec!["b".to_string(), "a".to_string()];
        assert_eq!(hashmap_order(&keys), vec![1, 0]);
    }
}
