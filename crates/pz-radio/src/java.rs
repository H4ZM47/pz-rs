//! The Java library behaviour the radio code depends on: `HashMap`
//! iteration order (the save file and the channel lists follow it), string
//! splitting and trimming, number parsing and printing, and line reading.

/// `String.hashCode` (over UTF-16 code units).
pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// A key's `hashCode()`.
pub trait JavaHash {
    fn java_hash(&self) -> i32;
}

impl JavaHash for i32 {
    fn java_hash(&self) -> i32 {
        *self
    }
}

impl JavaHash for i16 {
    fn java_hash(&self) -> i32 {
        *self as i32
    }
}

impl JavaHash for String {
    fn java_hash(&self) -> i32 {
        string_hash(self)
    }
}

/// A `java.util.HashMap` (or the map behind a `HashSet`) that iterates in
/// the original's order: by bucket of the spread hash, entries sharing a
/// bucket in insertion order. The table grows like the original's (it
/// never shrinks on removal). Tree bins are not modelled.
#[derive(Clone, Debug)]
pub struct JavaHashMap<K, V> {
    entries: Vec<(K, V)>,
    table: usize,
}

impl<K, V> Default for JavaHashMap<K, V> {
    fn default() -> Self {
        JavaHashMap {
            entries: Vec::new(),
            table: 16,
        }
    }
}

impl<K: JavaHash + PartialEq, V> JavaHashMap<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn position<Q>(&self, key: &Q) -> Option<usize>
    where
        K: std::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.entries.iter().position(|(k, _)| k.borrow() == key)
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: std::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.position(key).is_some()
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: std::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.position(key).map(|i| &self.entries[i].1)
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: std::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.position(key).map(move |i| &mut self.entries[i].1)
    }

    /// `put`: an existing key keeps its place and takes the new value.
    pub fn insert(&mut self, key: K, value: V) {
        match self.position(&key) {
            Some(i) => self.entries[i].1 = value,
            None => {
                self.entries.push((key, value));
                if self.entries.len() * 4 > self.table * 3 {
                    self.table *= 2;
                }
            }
        }
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.position(key).map(|i| self.entries.remove(i).1)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The entries in the original's iteration order.
    pub fn iter(&self) -> Vec<(&K, &V)> {
        let mask = (self.table - 1) as u32;
        let mut v: Vec<(&K, &V)> = self.entries.iter().map(|(k, v)| (k, v)).collect();
        v.sort_by_key(|(k, _)| {
            let h = k.java_hash() as u32;
            (h ^ (h >> 16)) & mask
        });
        v
    }

    /// The values in iteration order, mutably.
    pub fn values_mut(&mut self) -> Vec<&mut V> {
        let mask = (self.table - 1) as u32;
        let mut v: Vec<(u32, &mut V)> = self
            .entries
            .iter_mut()
            .map(|(k, v)| {
                let h = k.java_hash() as u32;
                ((h ^ (h >> 16)) & mask, v)
            })
            .collect();
        v.sort_by_key(|(b, _)| *b);
        v.into_iter().map(|(_, v)| v).collect()
    }
}

/// `String.trim()`: strips every char `<= ' '` from both ends.
pub fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `Character.isWhitespace`.
pub fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{000B}'
            | '\u{000C}'
            | '\r'
            | '\u{001C}'..='\u{001F}'
            | ' '
            | '\u{1680}'
            | '\u{2000}'..='\u{2006}'
            | '\u{2008}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// `String.isBlank()`.
pub fn is_blank(s: &str) -> bool {
    s.chars().all(is_whitespace)
}

/// Drops the trailing empty pieces, as `String.split` does.
fn drop_trailing_empty(mut parts: Vec<&str>) -> Vec<&str> {
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// `String.split(sep)` for a single literal character: trailing empty
/// pieces are dropped; no match gives the whole string.
pub fn split_char(s: &str, sep: char) -> Vec<&str> {
    if !s.contains(sep) {
        return vec![s];
    }
    drop_trailing_empty(s.split(sep).collect())
}

/// `String.split("\\s+")` (`\s` is `[ \t\n\x0B\f\r]`).
pub fn split_whitespace(s: &str) -> Vec<&str> {
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r');
    if !s.contains(is_ws) {
        return vec![s];
    }
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_ws = false;
    for (i, c) in s.char_indices() {
        if is_ws(c) {
            if !in_ws {
                parts.push(&s[start..i]);
                in_ws = true;
            }
        } else if in_ws {
            start = i;
            in_ws = false;
        }
    }
    parts.push(if in_ws { "" } else { &s[start..] });
    drop_trailing_empty(parts)
}

/// `Integer.parseInt` (ASCII digits); `None` where it throws.
pub fn parse_int(s: &str) -> Option<i32> {
    s.parse::<i32>().ok()
}

/// `Float.parseFloat`; `None` where it throws. Hexadecimal literals are
/// not accepted (the radio data never uses them).
pub fn parse_float(s: &str) -> Option<f32> {
    let t = trim(s);
    let body = t
        .strip_suffix(['f', 'F', 'd', 'D'])
        .filter(|b| !b.is_empty() && !b.ends_with(['f', 'F', 'd', 'D']))
        .unwrap_or(t);
    let negative = body.starts_with('-');
    let unsigned = body.strip_prefix(['+', '-']).unwrap_or(body);
    if unsigned == "Infinity" {
        return Some(if negative {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        });
    }
    if unsigned == "NaN" {
        return Some(f32::NAN);
    }
    if !unsigned.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    body.parse().ok()
}

/// `Float.toString` (the shortest decimal that reads back as the same
/// float, laid out the way Java prints it).
pub fn float_to_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    // `{:e}` gives the shortest round-trip digits: "d.ddde<exp>".
    let e = format!("{:e}", f.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if f < 0.0 { "-" } else { "" };
    let abs = f.abs();
    if (1e-3..1e7).contains(&abs) {
        let point = exp + 1;
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
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

/// `BufferedReader.readLine` over a whole text: lines end at `\n`, `\r`
/// or `\r\n`; a final line without an ending still counts.
pub fn read_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let b = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                i += 1;
                if i < b.len() && b[i] == b'\n' {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < b.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// `System.lineSeparator()`.
pub fn line_separator() -> &'static str {
    if cfg!(windows) {
        "\r\n"
    } else {
        "\n"
    }
}

/// `PZMath.fastfloor`.
pub fn fastfloor(f: f32) -> i32 {
    f.floor() as i32
}

/// `Character.toLowerCase(char)` / `toUpperCase(char)`: a one-to-one
/// mapping; characters whose full mapping is several characters keep
/// their simple form where Rust has none (the original maps them to one
/// character).
pub fn simple_lower(c: char) -> char {
    let mut m = c.to_lowercase();
    match (m.next(), m.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

pub fn simple_upper(c: char) -> char {
    let mut m = c.to_uppercase();
    match (m.next(), m.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

/// `String.compareToIgnoreCase`.
pub fn compare_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |c: char| simple_lower(simple_upper(c));
    let mut x = a.chars().map(fold);
    let mut y = b.chars().map(fold);
    loop {
        match (x.next(), y.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(p), Some(q)) if p != q => return p.cmp(&q),
            _ => {}
        }
    }
}

/// The game RNG as the radio code calls it: `rng(bound)` is the next
/// `Random.nextInt(bound)` of the generator behind `Rand`, always called
/// with `bound > 0`.
pub type Rng<'a> = dyn FnMut(i32) -> i32 + 'a;

/// `Rand.Next(max)`.
pub fn rand_next(rng: &mut Rng<'_>, max: i32) -> i32 {
    if max <= 0 {
        return 0;
    }
    rng(max)
}

/// `Rand.Next(min, max)`.
pub fn rand_range(rng: &mut Rng<'_>, min: i32, max: i32) -> i32 {
    if max == min {
        return min;
    }
    let (min, max) = if min > max { (max, min) } else { (min, max) };
    rng(max.wrapping_sub(min)).wrapping_add(min)
}

/// `Rand.NextBool(invProbability)`.
pub fn rand_bool(rng: &mut Rng<'_>, inv_probability: i32) -> bool {
    rand_next(rng, inv_probability) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_java() {
        assert_eq!(split_whitespace(" a  b "), vec!["", "a", "b"]);
        assert_eq!(split_whitespace("   "), Vec::<&str>::new());
        assert_eq!(split_whitespace(""), vec![""]);
        assert_eq!(split_whitespace("x"), vec!["x"]);
        assert_eq!(split_char("1,2,,", ','), vec!["1", "2"]);
        assert_eq!(split_char("a=b=c", '='), vec!["a", "b", "c"]);
        assert_eq!(split_char("", ','), vec![""]);
    }

    #[test]
    fn floats_print_like_java() {
        assert_eq!(float_to_string(2.0), "2.0");
        assert_eq!(float_to_string(0.5), "0.5");
        assert_eq!(float_to_string(1.1), "1.1");
        assert_eq!(float_to_string(1e7), "1.0E7");
        assert_eq!(float_to_string(1.5e-4), "1.5E-4");
        assert_eq!(float_to_string(0.001), "0.001");
        assert_eq!(float_to_string(123456.7), "123456.7");
        assert_eq!(float_to_string(-3.25), "-3.25");
        assert_eq!(parse_float(" 2.5f "), Some(2.5));
        assert_eq!(parse_float("abc"), None);
        assert_eq!(parse_float("inf"), None);
    }

    #[test]
    fn lines_split_like_buffered_reader() {
        assert_eq!(read_lines("a\r\nb\rc\n\nd"), vec!["a", "b", "c", "", "d"]);
        assert_eq!(read_lines("a\n"), vec!["a"]);
        assert_eq!(read_lines(""), Vec::<&str>::new());
    }

    #[test]
    fn hash_map_order() {
        let mut m = JavaHashMap::new();
        for k in [200, 17, 1, 33] {
            m.insert(k, ());
        }
        let keys: Vec<i32> = m.iter().into_iter().map(|(k, _)| *k).collect();
        // Buckets of a 16-slot table: 200 -> 8, 17 -> 1, 1 -> 1, 33 -> 1.
        assert_eq!(keys, vec![17, 1, 33, 200]);
    }
}
