//! Java library behaviour the script parser depends on: `String.trim`,
//! `String.split`, `indexOf`/`lastIndexOf` with Java's `fromIndex` rules and
//! `StringUtils.isNullOrWhitespace`.
//!
//! Script text is held as UTF-8. Every pattern the parser searches for is
//! ASCII, so byte offsets give the same splits as Java's UTF-16 offsets; where
//! an offset itself is observable (`utf16_len`) it is converted explicitly.

/// `String.trim()`: strips every char `<= U+0020` from both ends.
pub fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `Character.isWhitespace(char)`: Unicode space separators except the
/// non-breaking ones, plus `\t \n \x0B \f \r` and `\x1C`..`\x1F`. Differs
/// from `char::is_whitespace`, which includes U+0085 and the no-break spaces
/// and excludes `\x1C`..`\x1F`.
pub fn is_whitespace(c: char) -> bool {
    match c {
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{1C}'..='\u{1F}' => true,
        '\u{A0}' | '\u{2007}' | '\u{202F}' => false,
        ' ' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{205F}' | '\u{3000}' => true,
        _ => false,
    }
}

/// `StringUtils.isNullOrWhitespace`: null, empty, or every char
/// `Character.isWhitespace`.
pub fn is_null_or_whitespace(s: Option<&str>) -> bool {
    match s {
        None => true,
        Some(s) => s.chars().all(is_whitespace),
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
    fn whitespace_is_javas() {
        assert!(is_null_or_whitespace(None));
        assert!(is_null_or_whitespace(Some("")));
        assert!(is_null_or_whitespace(Some(" \t\u{1F}\u{3000}")));
        assert!(!is_null_or_whitespace(Some("\u{A0}")));
        assert!(!is_null_or_whitespace(Some("\u{85}")));
        assert!(!is_null_or_whitespace(Some(" a ")));
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
}
