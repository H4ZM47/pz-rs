//! Small Java/PZ runtime helpers whose exact semantics the map code relies on.

/// `PZMath.fastfloor(float)`.
pub fn fastfloor(val: f32) -> i32 {
    let xi = val as i32;
    if val < xi as f32 {
        xi - 1
    } else {
        xi
    }
}

/// `PZMath.coordmodulo(int, int)` (computed through `float`, like the original).
pub fn coordmodulo(value: i32, divisor: i32) -> i32 {
    value - fastfloor(value as f32 / divisor as f32) * divisor
}

/// `String.trim()`: strips leading/trailing chars `<= ' '`.
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `String.hashCode()` over UTF-16 code units.
pub fn string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for u in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(u as i32);
    }
    h
}

/// Iteration order of a `java.util.HashMap<String, _>` (default capacity and
/// load factor) after inserting `keys` in order without removals.
///
/// Buckets are visited by index; within a bucket entries keep insertion order
/// (resizes split buckets order-preservingly). Treeified bins (more than 8
/// colliding keys in one bucket) are not modelled.
pub fn hashmap_order<'a>(keys: &[&'a str]) -> Vec<&'a str> {
    let mut cap: usize = 16;
    while keys.len() as f64 > cap as f64 * 0.75 {
        cap *= 2;
    }
    let mut indexed: Vec<(usize, usize, &str)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let h = string_hash(k);
            let spread = h ^ ((h as u32) >> 16) as i32;
            ((spread as u32 as usize) & (cap - 1), i, *k)
        })
        .collect();
    indexed.sort_by_key(|&(bucket, i, _)| (bucket, i));
    indexed.into_iter().map(|(_, _, k)| k).collect()
}

/// `Boolean.parseBoolean`.
pub fn parse_boolean(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

/// Random source matching `zombie.core.random.Rand` (`RandAbstract` over a
/// `java.util.Random`-like generator). The engine supplies the real
/// generator; map loading only consumes it.
pub trait GameRand {
    /// `Rand.Next(int max)`: `nextInt(max)`, or 0 without drawing when
    /// `max <= 0`.
    fn next(&mut self, max: i32) -> i32;

    /// The generator's `nextFloat()` in `[0, 1)`. Generators that are never
    /// asked for floats may keep the default, which draws nothing.
    fn next_float(&mut self) -> f32 {
        0.0
    }

    /// `Rand.Next(int min, int max)`.
    fn next_range(&mut self, min: i32, max: i32) -> i32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        self.next(max.wrapping_sub(min)).wrapping_add(min)
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

/// `Integer.parseInt(s)`: an optional sign and decimal digits.
pub fn parse_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `Float.parseFloat(s)` for decimal input: surrounding whitespace is
/// ignored and a trailing `f`/`d` type suffix is accepted.
pub fn parse_float(s: &str) -> Option<f32> {
    let t = java_trim(s);
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    if t.is_empty()
        || t.bytes()
            .any(|b| !(b.is_ascii_digit() || b"+-.eE".contains(&b)))
    {
        return None;
    }
    t.parse().ok()
}

/// The `Throwable` for an error the port reports by its Java simple class
/// name (`"NullPointerException"`, `"ArrayIndexOutOfBoundsException"`, ...),
/// as `toString()` prints it. Those are all `java.lang` exceptions; other
/// text is kept as is.
pub fn java_throwable(desc: &str) -> pz_core::debug::LogThrowable {
    let bare = !desc.is_empty()
        && desc.chars().all(|c| c.is_ascii_alphanumeric())
        && desc.ends_with("Exception");
    if bare {
        pz_core::debug::LogThrowable::new(format!("java.lang.{desc}"))
    } else {
        pz_core::debug::LogThrowable::new(desc)
    }
}

/// The `Throwable` an I/O error corresponds to: `FileNotFoundException` for
/// a missing file, `EOFException` for a short read, otherwise an
/// `IOException` with the error's message (or the Java exception the port
/// named, see [`java_throwable`]).
pub fn io_throwable(e: &std::io::Error) -> pz_core::debug::LogThrowable {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::NotFound => {
            pz_core::debug::LogThrowable::new(format!("java.io.FileNotFoundException: {e}"))
        }
        ErrorKind::UnexpectedEof => pz_core::debug::LogThrowable::new("java.io.EOFException"),
        _ => {
            let msg = e.to_string();
            if msg.chars().all(|c| c.is_ascii_alphanumeric()) && msg.ends_with("Exception") {
                java_throwable(&msg)
            } else {
                pz_core::debug::LogThrowable::new(format!("java.io.IOException: {msg}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_modulo() {
        assert_eq!(fastfloor(-0.5), -1);
        assert_eq!(fastfloor(-1.0), -1);
        assert_eq!(fastfloor(3.9), 3);
        assert_eq!(coordmodulo(-1, 8), 7);
        assert_eq!(coordmodulo(256, 300), 256);
        assert_eq!(coordmodulo(600, 300), 0);
    }

    #[test]
    fn hashes() {
        assert_eq!(string_hash(""), 0);
        assert_eq!(string_hash("abc"), 96354);
        assert_eq!(java_trim(" \t a b\r\n"), "a b");
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_int("-12"), Some(-12));
        assert_eq!(parse_int("+3"), Some(3));
        assert_eq!(parse_int(" 3"), None);
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_float(" 255 "), Some(255.0));
        assert_eq!(parse_float("1.5f"), Some(1.5));
        assert_eq!(parse_float("x"), None);
    }

    struct Seq(Vec<i32>);
    impl GameRand for Seq {
        fn next(&mut self, max: i32) -> i32 {
            assert!(max > 0);
            self.0.remove(0)
        }
        fn next_float(&mut self) -> f32 {
            0.5
        }
    }

    #[test]
    fn rand_ranges() {
        let mut r = Seq(vec![3]);
        assert_eq!(r.next_range(5, 5), 5);
        assert_eq!(r.next_range(10, 2), 5);
        assert!(r.0.is_empty());
        assert_eq!(r.next_float_range(1.0, 0.0), 0.5);
    }

    #[test]
    fn hashmap_order_by_bucket() {
        // "b" (98) lands in bucket 2, "a" (97) in bucket 1.
        assert_eq!(hashmap_order(&["b", "a"]), vec!["a", "b"]);
    }
}
