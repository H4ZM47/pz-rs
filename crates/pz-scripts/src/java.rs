//! Java library behaviour the script loader depends on: `String.trim`,
//! `String.split`, `indexOf`/`lastIndexOf` with Java's `fromIndex` rules,
//! `String.hashCode`/`compareTo`, and `java.util.HashMap` iteration order.
//!
//! Script text is held as UTF-8. Every pattern the loader searches for is
//! ASCII, so byte offsets give the same splits as Java's UTF-16 offsets; where
//! an offset itself is observable (`utf16_len`) it is converted explicitly.

/// `String.trim()`: strips every char `<= U+0020` from both ends.
pub fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `String.isEmpty()` after `trim()`: `StringUtils.isNullOrWhitespace`.
pub fn is_null_or_whitespace(s: Option<&str>) -> bool {
    match s {
        None => true,
        Some(s) => s.chars().all(char::is_whitespace),
    }
}

/// `\s` in `java.util.regex`: `[ \t\n\x0B\f\r]`.
pub fn is_regex_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

/// `String.split(regex)` for a regex that is a character class (`[{}]`,
/// `,`) or a character class with `+` (`\\s+`). Java keeps a leading empty
/// string when the input starts with a non-empty match, drops trailing empty
/// strings, and returns the input itself when nothing matched.
pub fn split(s: &str, is_sep: impl Fn(char) -> bool, plus: bool) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut matched = false;
    let mut iter = s.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if !is_sep(c) {
            continue;
        }
        let mut end = i + c.len_utf8();
        if plus {
            while let Some(&(j, d)) = iter.peek() {
                if !is_sep(d) {
                    break;
                }
                end = j + d.len_utf8();
                iter.next();
            }
        }
        matched = true;
        out.push(&s[start..i]);
        start = end;
    }
    if !matched {
        return vec![s];
    }
    out.push(&s[start..]);
    while out.last().is_some_and(|p| p.is_empty()) {
        out.pop();
    }
    out
}

/// `s.split("\\s+")`.
pub fn split_ws(s: &str) -> Vec<&str> {
    split(s, is_regex_space, true)
}

/// `s.split(String.valueOf(c))` for a single literal char.
pub fn split_char(s: &str, sep: char) -> Vec<&str> {
    split(s, |c| c == sep, false)
}

/// `String.indexOf(str, fromIndex)` on byte offsets; -1 when absent.
pub fn index_of(s: &str, pat: &str, from: i64) -> i64 {
    let from = from.max(0) as usize;
    if from > s.len() {
        return -1;
    }
    match s.as_bytes()[from..].windows(pat.len()).position(|w| w == pat.as_bytes()) {
        Some(p) => (from + p) as i64,
        None if pat.is_empty() => from as i64,
        None => -1,
    }
}

/// `String.lastIndexOf(str, fromIndex)` on byte offsets: the last match
/// starting at or before `from`; -1 when absent or `from < 0`.
pub fn last_index_of(s: &str, pat: &str, from: i64) -> i64 {
    if from < 0 {
        return -1;
    }
    let max_start = s.len().checked_sub(pat.len());
    let Some(max_start) = max_start else { return -1 };
    let start = (from as usize).min(max_start);
    let b = s.as_bytes();
    let p = pat.as_bytes();
    (0..=start).rev().find(|&i| &b[i..i + p.len()] == p).map_or(-1, |i| i as i64)
}

/// `String.lastIndexOf(str)`.
pub fn last_index_of_all(s: &str, pat: &str) -> i64 {
    last_index_of(s, pat, s.len() as i64)
}

/// Length in UTF-16 code units, as Java's `String.length()`.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `String.hashCode()`.
pub fn string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for u in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(u as i32);
    }
    h
}

/// `String.compareTo`: lexicographic over UTF-16 code units.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `String.equalsIgnoreCase`.
pub fn eq_ignore_case(a: &str, b: &str) -> bool {
    let mut x = a.encode_utf16();
    let mut y = b.encode_utf16();
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(c1), Some(c2)) => {
                if c1 == c2 {
                    continue;
                }
                let u1 = upper16(c1);
                let u2 = upper16(c2);
                if u1 == u2 || lower16(u1) == lower16(u2) {
                    continue;
                }
                return false;
            }
            _ => return false,
        }
    }
}

fn upper16(c: u16) -> u16 {
    char::from_u32(c as u32)
        .map(|ch| {
            let mut up = ch.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) if (u as u32) <= 0xFFFF => u as u16,
                _ => c,
            }
        })
        .unwrap_or(c)
}

fn lower16(c: u16) -> u16 {
    char::from_u32(c as u32)
        .map(|ch| {
            let mut lo = ch.to_lowercase();
            match (lo.next(), lo.next()) {
                (Some(l), None) if (l as u32) <= 0xFFFF => l as u16,
                _ => c,
            }
        })
        .unwrap_or(c)
}

/// `String.toLowerCase(Locale.ENGLISH)`.
pub fn to_lower(s: &str) -> String {
    s.to_lowercase()
}

/// Shared validation for `Float.parseFloat`/`Double.parseDouble`: Java trims
/// `<= U+0020` chars and accepts an optional `f`/`F`/`d`/`D` suffix.
/// Hexadecimal floating-point literals are not supported.
fn java_float_literal(s: &str) -> Option<&str> {
    let t = trim(s);
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    match t {
        "NaN" | "+NaN" | "-NaN" | "Infinity" | "+Infinity" | "-Infinity" => return Some(t),
        _ => {}
    }
    let ok = !t.is_empty()
        && t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'));
    ok.then_some(t)
}

fn special(t: &str) -> Option<f64> {
    match t {
        "NaN" | "+NaN" | "-NaN" => Some(f64::NAN),
        "Infinity" | "+Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

/// `Double.parseDouble`.
pub fn parse_f64(s: &str) -> Option<f64> {
    let t = java_float_literal(s)?;
    special(t).or_else(|| t.parse::<f64>().ok())
}

/// `Float.parseFloat`: rounds the decimal string directly to `float`, which
/// Rust's correctly-rounded `f32` parser also does.
pub fn parse_f32(s: &str) -> Option<f32> {
    let t = java_float_literal(s)?;
    special(t).map(|v| v as f32).or_else(|| t.parse::<f32>().ok())
}

/// `Integer.parseInt`: optional sign, ASCII digits, no surrounding space.
pub fn parse_i32(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i32>().ok()
}

/// `Boolean.parseBoolean`.
pub fn parse_bool(s: &str) -> bool {
    eq_ignore_case(s, "true")
}

const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;

/// `java.util.HashMap<String, V>` with the same iteration order as the JDK
/// (17–25): power-of-two table, `hash ^ (hash >>> 16)` spreading, tail
/// insertion into bins, order-preserving lo/hi splits on resize, and the
/// resize-instead-of-treeify rule for small tables.
///
/// Bins that the JDK would turn into red-black trees (9+ colliding keys with a
/// table of 64+) keep list order here; with String keys at load factor 0.75
/// this needs a pathological key set and is not reached by the game scripts.
#[derive(Clone, Debug)]
pub struct JHashMap<V> {
    table: Vec<Vec<usize>>,
    entries: Vec<Option<(i32, String, V)>>,
    size: usize,
    threshold: usize,
    /// The key's Java `hashCode()`; `String.hashCode` unless the map stands
    /// in for one keyed by another type (see [`JHashMap::with_hasher`]).
    hasher: fn(&str) -> i32,
}

impl<V> Default for JHashMap<V> {
    fn default() -> Self {
        JHashMap { table: Vec::new(), entries: Vec::new(), size: 0, threshold: 0, hasher: string_hash }
    }
}

impl<V> JHashMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// A map whose keys are represented by strings but hash like another
    /// Java type, e.g. `ResourceLocation` keyed by its `ns:path` id.
    pub fn with_hasher(hasher: fn(&str) -> i32) -> Self {
        JHashMap { hasher, ..Self::default() }
    }

    fn spread(&self, key: &str) -> i32 {
        let h = (self.hasher)(key);
        h ^ ((h as u32) >> 16) as i32
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    fn find(&self, key: &str) -> Option<usize> {
        if self.table.is_empty() {
            return None;
        }
        let h = self.spread(key);
        let bin = &self.table[(h as u32 as usize) & (self.table.len() - 1)];
        bin.iter().copied().find(|&i| {
            let (eh, ek, _) = self.entries[i].as_ref().unwrap();
            *eh == h && ek == key
        })
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.find(key).map(|i| &self.entries[i].as_ref().unwrap().2)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.find(key).map(|i| &mut self.entries[i].as_mut().unwrap().2)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.find(key).is_some()
    }

    /// `put`: replaces the value in place (keeping its position) or appends.
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        if let Some(i) = self.find(&key) {
            let slot = &mut self.entries[i].as_mut().unwrap().2;
            return Some(std::mem::replace(slot, value));
        }
        if self.table.is_empty() {
            self.resize();
        }
        let h = self.spread(&key);
        let idx = (h as u32 as usize) & (self.table.len() - 1);
        self.entries.push(Some((h, key, value)));
        let e = self.entries.len() - 1;
        self.table[idx].push(e);
        if self.table[idx].len() > TREEIFY_THRESHOLD && self.table.len() < MIN_TREEIFY_CAPACITY {
            self.resize();
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
        None
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        let i = self.find(key)?;
        let h = self.entries[i].as_ref().unwrap().0;
        let idx = (h as u32 as usize) & (self.table.len() - 1);
        self.table[idx].retain(|&e| e != i);
        self.size -= 1;
        self.entries[i].take().map(|(_, _, v)| v)
    }

    pub fn clear(&mut self) {
        // HashMap.clear keeps the table capacity.
        for bin in &mut self.table {
            bin.clear();
        }
        self.entries.clear();
        self.size = 0;
    }

    fn resize(&mut self) {
        let old_cap = self.table.len();
        let new_cap = if old_cap == 0 { 16 } else { old_cap * 2 };
        self.threshold = new_cap * 3 / 4;
        let mut table = vec![Vec::new(); new_cap];
        for (j, bin) in std::mem::take(&mut self.table).into_iter().enumerate() {
            for e in bin {
                let h = self.entries[e].as_ref().unwrap().0 as u32 as usize;
                if old_cap == 0 || h & old_cap == 0 {
                    table[j].push(e);
                } else {
                    table[j + old_cap].push(e);
                }
            }
        }
        self.table = table;
    }

    /// Iteration in `HashMap` order (`keySet`/`values`/`entrySet`).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.table.iter().flatten().map(move |&i| {
            let (_, k, v) = self.entries[i].as_ref().unwrap();
            (k.as_str(), v)
        })
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.iter().map(|(k, _)| k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_matches_java() {
        assert_eq!(split_ws(""), vec![""]);
        assert_eq!(split_ws("a"), vec!["a"]);
        assert_eq!(split_ws("  a  b "), vec!["", "a", "b"]);
        assert!(split_ws("   ").is_empty());
        assert_eq!(split("{a}{b}", |c| c == '{' || c == '}', false), vec!["", "a", "", "b"]);
        assert_eq!(split_char("a,,b,,", ','), vec!["a", "", "b"]);
    }

    #[test]
    fn index_rules() {
        assert_eq!(index_of("a}b}", "}", 1), 1);
        assert_eq!(index_of("a}b}", "}", 2), 3);
        assert_eq!(index_of("ab", "}", 9), -1);
        assert_eq!(last_index_of("/*x*/", "/*", 4), 0);
        assert_eq!(last_index_of("/*x*/", "/*", -1), -1);
        assert_eq!(last_index_of_all("a*/b*/", "*/"), 4);
    }

    #[test]
    fn hash_code() {
        assert_eq!(string_hash(""), 0);
        assert_eq!(string_hash("Base"), 2063089);
        assert_eq!(string_hash("Axe"), 66286);
    }

    #[test]
    fn hashmap_order_follows_buckets() {
        // "Aa" and "BB" collide (2112); insertion order is kept within a bin.
        let mut m = JHashMap::new();
        for k in ["c", "BB", "a", "Aa", "b"] {
            m.insert(k.to_string(), ());
        }
        let keys: Vec<_> = m.keys().collect();
        assert_eq!(keys, vec!["BB", "Aa", "a", "b", "c"]);
    }

    #[test]
    fn hashmap_resize_keeps_bin_order() {
        let mut m = JHashMap::new();
        let names: Vec<String> = (0..200).map(|i| format!("Item{i}")).collect();
        for n in &names {
            m.insert(n.clone(), ());
        }
        assert_eq!(m.len(), 200);
        assert_eq!(m.table.len(), 512);
        // Within each bin, order is insertion order.
        for bin in &m.table {
            assert!(bin.windows(2).all(|w| w[0] < w[1]));
        }
    }
}
