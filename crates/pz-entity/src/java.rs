//! Java library behaviour the save layout depends on.

/// `String.hashCode()` (over UTF-16 code units).
pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// Iteration order of a `new HashSet<String>()` after adding `items` in
/// order: duplicates dropped, then buckets in index order, insertion order
/// within a bucket (resizes split buckets without reordering them).
pub fn hash_set_order(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut set: Vec<String> = Vec::new();
    for s in items {
        if !set.contains(&s) {
            set.push(s);
        }
    }
    // Default capacity 16, load factor 0.75: the table doubles once the size
    // exceeds the threshold.
    let mut cap = 16usize;
    while set.len() > cap * 3 / 4 {
        cap *= 2;
    }
    let bucket = |s: &String| {
        let h = string_hash(s);
        ((h ^ ((h as u32) >> 16) as i32) as u32 as usize) & (cap - 1)
    };
    let mut keyed: Vec<(usize, String)> = set.into_iter().map(|s| (bucket(&s), s)).collect();
    keyed.sort_by_key(|(b, _)| *b);
    keyed.into_iter().map(|(_, s)| s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_hash_matches_java() {
        assert_eq!(string_hash(""), 0);
        assert_eq!(string_hash("a"), 97);
        assert_eq!(string_hash("ab"), 97 * 31 + 98);
        // Overflow wraps like Java int arithmetic.
        let long = "abcdefghijklmnopqrstuvwxyz";
        let mut h = 0i32;
        for c in long.chars() {
            h = h.wrapping_mul(31).wrapping_add(c as i32);
        }
        assert_eq!(string_hash(long), h);
    }

    #[test]
    fn hash_set_orders_by_bucket_then_insertion() {
        // "a" (97) -> bucket 1, "b" (98) -> 2, "q" (113) -> 1.
        let out = hash_set_order(["b", "q", "a", "q"].map(String::from));
        assert_eq!(out, ["q", "a", "b"]);
    }
}
