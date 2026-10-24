//! Java collection iteration orders that saving depends on.

/// `String.hashCode` (over UTF-16 code units).
pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// The table size of a `new HashMap()` after `n` distinct insertions.
fn hash_map_capacity(n: usize) -> usize {
    let mut cap = 16;
    while n as f64 > cap as f64 * 0.75 {
        cap *= 2;
    }
    cap
}

/// Puts `entries` (distinct keys, in insertion order) into the order a
/// `java.util.HashMap` iterates them: by bucket of the spread hash, ties in
/// insertion order. Tree bins (eight or more colliding keys) are not
/// modelled.
pub fn sort_hash_map<T>(entries: &mut [T], hash: impl Fn(&T) -> i32) {
    let cap = hash_map_capacity(entries.len()) as u32;
    entries.sort_by_key(|e| {
        let h = hash(e) as u32;
        (h ^ (h >> 16)) & (cap - 1)
    });
}

/// [`sort_hash_map`] for `Integer` keys.
pub fn sort_int_hash_map<T>(entries: &mut [T], key: impl Fn(&T) -> i32) {
    sort_hash_map(entries, key)
}

/// [`sort_hash_map`] for `String` keys.
pub fn sort_string_hash_map<T>(entries: &mut [T], key: impl Fn(&T) -> &str) {
    sort_hash_map(entries, |e| string_hash(key(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_map_orders() {
        assert_eq!(string_hash("Ab"), 65 * 31 + 98);
        let mut v = vec![20, 3, 17, 1];
        sort_int_hash_map(&mut v, |&k| k);
        // Buckets of 16: 20->4, 3->3, 17->1, 1->1 (insertion order kept).
        assert_eq!(v, vec![17, 1, 3, 20]);
        let mut many: Vec<i32> = (0..13).rev().collect();
        sort_int_hash_map(&mut many, |&k| k);
        assert_eq!(many, (0..13).collect::<Vec<_>>());
    }
}
