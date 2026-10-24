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

/// `String.equalsIgnoreCase` (per UTF-16 unit: equal, or equal after
/// upper-casing, or equal after upper- then lower-casing).
pub fn java_equals_ignore_case(a: &str, b: &str) -> bool {
    let (a, b): (Vec<u16>, Vec<u16>) = (a.encode_utf16().collect(), b.encode_utf16().collect());
    if a.len() != b.len() {
        return false;
    }
    fn map(u: u16, f: impl Fn(char) -> Option<char>) -> u16 {
        char::from_u32(u as u32)
            .and_then(f)
            .and_then(|c| u16::try_from(c as u32).ok())
            .unwrap_or(u)
    }
    let upper = |c: char| {
        let mut it = c.to_uppercase();
        let r = it.next();
        if it.next().is_some() {
            Some(c)
        } else {
            r
        }
    };
    let lower = |c: char| {
        let mut it = c.to_lowercase();
        let r = it.next();
        if it.next().is_some() {
            Some(c)
        } else {
            r
        }
    };
    a.iter().zip(&b).all(|(&x, &y)| {
        if x == y {
            return true;
        }
        let (ux, uy) = (map(x, upper), map(y, upper));
        ux == uy || map(ux, lower) == map(uy, lower)
    })
}

/// `Boolean.parseBoolean`.
pub fn parse_boolean(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

/// Random source matching `zombie.core.random.Rand.Next(int)`. The engine
/// supplies the real generator; map loading only consumes it.
pub trait GameRand {
    fn next(&mut self, max: i32) -> i32;
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
        assert!(java_equals_ignore_case("ParkingStall", "parkingSTALL"));
        assert!(!java_equals_ignore_case("Farm", "FarmLand"));
    }

    #[test]
    fn hashmap_order_by_bucket() {
        // "b" (98) lands in bucket 2, "a" (97) in bucket 1.
        assert_eq!(hashmap_order(&["b", "a"]), vec!["a", "b"]);
    }
}
