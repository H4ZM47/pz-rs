//! Java library semantics the file-system code depends on.
//!
//! The original calls `String.trim`, `String.split`, `Integer.parseInt`,
//! `String.CASE_INSENSITIVE_ORDER`, `BufferedReader.readLine` and similar.
//! Their exact rules differ from the nearest Rust std functions (for example
//! `trim` strips only chars `<= ' '`), so each is ported here once.

use std::cmp::Ordering;

/// `String.trim()`: strips leading and trailing chars `<= U+0020`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `Character.isWhitespace(char)`.
pub fn is_whitespace(c: char) -> bool {
    match c {
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{1C}'..='\u{1F}' => true,
        // Unicode space separators, minus the non-breaking ones.
        '\u{A0}' | '\u{2007}' | '\u{202F}' => false,
        ' ' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{205F}'
        | '\u{3000}' => true,
        _ => false,
    }
}

/// `StringUtils.isNullOrWhitespace(s)` for a non-null string.
pub fn is_null_or_whitespace(s: &str) -> bool {
    s.chars().all(is_whitespace)
}

/// `toLowerCase(Locale.ENGLISH)`.
pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `String.split(literal)` for a single-char literal that is not a regex
/// metacharacter: trailing empty strings are removed, and a string without
/// the separator yields itself.
pub fn split_char(s: &str, sep: char) -> Vec<String> {
    let mut parts: Vec<String> = s.split(sep).map(str::to_string).collect();
    if parts.len() > 1 {
        while parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
            parts.pop();
        }
        // "".split(",") is [""], but ",".split(",") is [].
        if parts.len() == 1 && parts[0].is_empty() && s.contains(sep) {
            parts.clear();
        }
    }
    parts
}

/// `String.split("\\s+")`: Java's `\s` is `[ \t\n\x0B\f\r]`. A leading
/// separator yields a leading empty string; trailing empties are removed.
pub fn split_ws(s: &str) -> Vec<String> {
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r');
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut in_sep = false;
    for c in s.chars() {
        if is_ws(c) {
            if !in_sep {
                parts.push(std::mem::take(&mut cur));
                in_sep = true;
            }
        } else {
            in_sep = false;
            cur.push(c);
        }
    }
    parts.push(cur);
    if parts.len() > 1 {
        while parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
            parts.pop();
        }
        if parts.len() == 1 && parts[0].is_empty() {
            parts.clear();
        }
    }
    parts
}

/// `Integer.parseInt(s)` (radix 10). Only ASCII digits are accepted here;
/// Java also accepts other Unicode decimal digits, which mod files do not use.
pub fn parse_int(s: &str) -> Option<i32> {
    let (neg, digits) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut v: i64 = 0;
    for b in digits.bytes() {
        v = v * 10 + i64::from(b - b'0');
        if v > i64::from(i32::MAX) + 1 {
            return None;
        }
    }
    let v = if neg { -v } else { v };
    i32::try_from(v).ok()
}

/// `PZMath.tryParseInt(s, default)`.
pub fn try_parse_int(s: &str, default: i32) -> i32 {
    if is_null_or_whitespace(s) {
        return default;
    }
    parse_int(trim(s)).unwrap_or(default)
}

/// `String.CASE_INSENSITIVE_ORDER`: compares UTF-16 code units after
/// `toUpperCase` then `toLowerCase` on each char.
pub fn case_insensitive_cmp(a: &str, b: &str) -> Ordering {
    let fold = |u: u16| -> u16 {
        let Some(c) = char::from_u32(u32::from(u)) else { return u };
        let up = single_upper(c);
        let low = single_lower(up);
        u16::try_from(u32::from(low)).unwrap_or(u)
    };
    let (mut ia, mut ib) = (a.encode_utf16(), b.encode_utf16());
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                if x != y {
                    let (fx, fy) = (fold(x), fold(y));
                    if fx != fy {
                        return fx.cmp(&fy);
                    }
                }
            }
        }
    }
}

/// `Character.toUpperCase(char)`: single-char mapping only.
fn single_upper(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

/// `Character.toLowerCase(char)`: single-char mapping only.
fn single_lower(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// `BufferedReader.readLine` over a whole decoded text: lines end at `\n`,
/// `\r` or `\r\n`; a final line without terminator is still returned.
pub fn read_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                out.push(&text[start..i]);
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        out.push(&text[start..]);
    }
    out
}

/// `new InputStreamReader(in, "UTF-8")`: malformed input becomes U+FFFD.
pub fn decode_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_matches_java() {
        assert_eq!(split_char("a,b,,", ','), ["a", "b"]);
        assert_eq!(split_char("", ','), [""]);
        assert!(split_char(",", ',').is_empty());
        assert_eq!(split_char(",a", ','), ["", "a"]);
        assert_eq!(split_ws(" a  b "), ["", "a", "b"]);
        assert_eq!(split_ws("a"), ["a"]);
    }

    #[test]
    fn parse_int_matches_java() {
        assert_eq!(parse_int("-2147483648"), Some(i32::MIN));
        assert_eq!(parse_int("2147483648"), None);
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(parse_int(" 7"), None);
        assert_eq!(try_parse_int(" 7 ", 0), 7);
        assert_eq!(try_parse_int("x", 3), 3);
    }

    #[test]
    fn trim_and_lines() {
        assert_eq!(trim("\u{1}a b\u{20}"), "a b");
        assert_eq!(trim("\u{A0}a"), "\u{A0}a");
        assert_eq!(read_lines("a\r\nb\rc\n\nd"), ["a", "b", "c", "", "d"]);
    }

    #[test]
    fn case_insensitive_order() {
        let mut v = vec!["b.lua", "A.lua", "a_.lua", "a.lua"];
        v.sort_by(|a, b| case_insensitive_cmp(a, b));
        assert_eq!(v, ["A.lua", "a.lua", "a_.lua", "b.lua"]);
    }
}
