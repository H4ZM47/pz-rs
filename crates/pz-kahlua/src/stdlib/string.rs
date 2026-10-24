//! `StringLib`: `sub`, `char`, `byte`, `lower`, `upper`, `reverse`,
//! `format`, `find`, `match`, `gsub`, `trim`, `split`, `sort`, `contains`.
//!
//! Kahlua strings are Java strings, so every index, length and pattern
//! position here counts UTF-16 code units. Strings are converted to
//! `Vec<u16>` wherever indexing matters.
//!
//! Deliberate Kahlua quirks preserved from the original (they differ from
//! reference Lua):
//! * `find`/`match` with a negative or zero `init` start one unit earlier
//!   than in Lua (`init - 1 + len`).
//! * A back-reference (`%1`) to a *closed* capture fails with
//!   "invalid capture index"; the assertion is inverted in the original.
//! * In `gsub` replacement strings `%1` with no captures is an error, a
//!   trailing `%` inserts U+0000, function replacements get only the first
//!   capture (plus two nils), and a non-string/number result appends "null".
//! * `find`, `match` and `gsub` wrap every exception they raise in a new
//!   `RuntimeException`, so error messages read
//!   `java.lang.RuntimeException: <message>`.
//! * `%q` does not escape backslashes; `%s`/`%c` never zero-pad.
//! * `split` takes a Java regular expression (`String.split`).

use super::{arg, lua_assert, number_arg, opt_arg, opt_number_arg, reg, string_arg};
use crate::exec::{java_compare, java_len, java_value_of, raw_tostring};
use crate::number::number_to_string;
use crate::table::TableRef;
use crate::value::Value;
use crate::vm::{LuaError, LuaResult, NativeCall, Thread};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn register(thread: &mut Thread, env: &TableRef) {
    let string = TableRef::new();
    reg(&string, "sub", sub);
    reg(&string, "char", string_char);
    reg(&string, "byte", string_byte);
    reg(&string, "lower", lower);
    reg(&string, "upper", upper);
    reg(&string, "reverse", reverse);
    reg(&string, "format", format);
    reg(&string, "find", |c| find_aux(c, true));
    reg(&string, "match", |c| find_aux(c, false));
    reg(&string, "gsub", gsub);
    reg(&string, "trim", trim);
    reg(&string, "split", split);
    reg(&string, "sort", sort);
    reg(&string, "contains", contains);
    string.set_str("__index", string.clone());
    thread.class_metatables.set_str("java.lang.String", string.clone());
    env.set_str("string", string);
}

// ---------------------------------------------------------------------------
// UTF-16 and Java exception helpers
// ---------------------------------------------------------------------------

const fn u(c: char) -> u16 {
    c as u16
}

/// Java `String` code units of `s`.
fn units(s: &str) -> Vec<u16> {
    if s.is_ascii() {
        s.bytes().map(u16::from).collect()
    } else {
        s.encode_utf16().collect()
    }
}

/// Back to a Rust string; lone surrogates become U+FFFD.
fn from_units(v: &[u16]) -> String {
    if v.iter().all(|&c| c < 0x80) {
        v.iter().map(|&c| c as u8 as char).collect()
    } else {
        String::from_utf16_lossy(v)
    }
}

fn java_err(class: &'static str, message: impl Into<String>) -> LuaError {
    LuaError::Runtime { message: message.into(), class }
}

/// `StringIndexOutOfBoundsException` from `charAt` (JDK 17+ wording).
fn sioobe_index(index: i64, length: usize) -> LuaError {
    java_err(
        "java.lang.StringIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// `String.substring(begin, end)` with Java's bounds checks.
fn substring(s: &[u16], begin: isize, end: isize) -> LuaResult<&[u16]> {
    if begin < 0 || begin > end || end as usize > s.len() {
        return Err(java_err(
            "java.lang.StringIndexOutOfBoundsException",
            format!("begin {begin}, end {end}, length {}", s.len()),
        ));
    }
    Ok(&s[begin as usize..end as usize])
}

/// Java's StringBuilder would eventually fail; Rust must not loop forever or
/// abort on allocation, so very large outputs raise this instead.
const MAX_BUILDER: usize = 1 << 28;

fn out_of_memory() -> LuaError {
    java_err("java.lang.OutOfMemoryError", "Java heap space")
}

/// `KahluaUtil.round`: round half up, ties on exact .5 go to even.
fn kround(x: f64) -> f64 {
    if x < 0.0 {
        return -kround(-x);
    }
    let x = x + 0.5;
    let x2 = x.floor();
    if x2 == x {
        return x2 - ((x2 as i64) & 1) as f64;
    }
    x2
}

/// `KahluaUtil.ipow` (wrapping like Java `long`).
fn ipow(mut base: i64, mut exponent: i32) -> i64 {
    if exponent <= 0 {
        return 1;
    }
    let mut b = if exponent & 1 != 0 { base } else { 1 };
    exponent >>= 1;
    while exponent != 0 {
        base = base.wrapping_mul(base);
        if exponent & 1 != 0 {
            b = b.wrapping_mul(base);
        }
        exponent >>= 1;
    }
    b
}

/// `KahluaUtil.isNegative`: sign bit of the canonical bits (NaN is positive).
fn is_negative(d: f64) -> bool {
    !d.is_nan() && d.is_sign_negative()
}

/// Java `Character.isDigit(char)`: Unicode category Nd (BMP).
fn java_is_digit(c: u16) -> bool {
    const ZEROS: [u16; 37] = [
        0x0030, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6, 0x0D66,
        0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50,
        0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
    ];
    if c < 0x80 {
        return (u('0')..=u('9')).contains(&c);
    }
    ZEROS.iter().any(|&z| c >= z && c < z + 10)
}

/// Java `Character.isLowerCase(char)` (Ll or Other_Lowercase).
fn java_is_lower(c: u16) -> bool {
    char::from_u32(c as u32).is_some_and(|ch| ch.is_lowercase())
}

/// Java `Character.isUpperCase(char)` (Lu or Other_Uppercase).
fn java_is_upper(c: u16) -> bool {
    char::from_u32(c as u32).is_some_and(|ch| ch.is_uppercase())
}

// ---------------------------------------------------------------------------
// Simple functions
// ---------------------------------------------------------------------------

fn sub(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let s = string_arg(c, 1, "sub")?;
    let start = number_arg(c, 2, "sub")?;
    let end = if c.nargs() >= 3 { number_arg(c, 3, "sub")? } else { -1.0 };
    let mut istart = start as i32;
    let mut iend = end as i32;
    let len = java_len(&s) as i32;
    if istart < 0 {
        istart = len.wrapping_add(istart).wrapping_add(1).max(1);
    } else if istart == 0 {
        istart = 1;
    }
    if iend < 0 {
        iend = iend.wrapping_add(len).wrapping_add(1).max(0);
    } else if iend > len {
        iend = len;
    }
    if istart > iend {
        return Ok(c.push(""));
    }
    let (a, b) = ((istart - 1) as usize, iend as usize);
    if s.is_ascii() {
        let r: &str = &s[a..b];
        return Ok(c.push(r));
    }
    let us = units(&s);
    Ok(c.push(from_units(&us[a..b])))
}

fn string_char(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let mut sb = Vec::with_capacity(c.nargs());
    for i in 0..c.nargs() {
        let num = number_arg(c, i + 1, "char")? as i32;
        sb.push(num as u16);
    }
    Ok(c.push(from_units(&sb)))
}

fn string_byte(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    lua_assert(c.nargs() >= 1, "not enough arguments")?;
    let s = string_arg(c, 1, "byte")?;
    let mut i = opt_number_arg(c, 2).map(|v| v as i32).unwrap_or(1);
    let mut j = opt_number_arg(c, 3).map(|v| v as i32).unwrap_or(i);
    let us = units(&s);
    let len = us.len() as i32;
    if i < 0 {
        i = i.wrapping_add(len.wrapping_add(1));
    }
    if i <= 0 {
        i = 1;
    }
    if j < 0 {
        j = j.wrapping_add(len.wrapping_add(1));
    } else if j > len {
        j = len;
    }
    let n_returns = 1i32.wrapping_add(j).wrapping_sub(i);
    if n_returns <= 0 {
        return Ok(0);
    }
    let offset = i - 1;
    let mut vals = Vec::with_capacity(n_returns as usize);
    for i2 in 0..n_returns {
        let idx = offset.wrapping_add(i2);
        if idx < 0 || idx >= len {
            return Err(sioobe_index(idx as i64, us.len()));
        }
        vals.push(Value::Num(us[idx as usize] as f64));
    }
    c.set_top(n_returns as usize)?;
    for (k, v) in vals.into_iter().enumerate() {
        c.set(k, v);
    }
    Ok(n_returns as usize)
}

fn lower(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    lua_assert(c.nargs() >= 1, "not enough arguments")?;
    let s = string_arg(c, 1, "lower")?;
    let r = if s.is_ascii() { s.to_ascii_lowercase() } else { s.to_lowercase() };
    Ok(c.push(r))
}

fn upper(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    lua_assert(c.nargs() >= 1, "not enough arguments")?;
    let s = string_arg(c, 1, "upper")?;
    let r = if s.is_ascii() { s.to_ascii_uppercase() } else { s.to_uppercase() };
    Ok(c.push(r))
}

fn contains(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    lua_assert(c.nargs() >= 2, "not enough arguments")?;
    let s = string_arg(c, 1, "contains")?;
    let s2 = string_arg(c, 2, "contains")?;
    Ok(c.push(s.contains(&*s2)))
}

fn reverse(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    lua_assert(c.nargs() >= 1, "not enough arguments")?;
    let s = string_arg(c, 1, "reverse")?;
    // StringBuilder.reverse keeps surrogate pairs together, like chars().
    let r: String = s.chars().rev().collect();
    Ok(c.push(r))
}

fn trim(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let s = string_arg(c, 1, "trim")?;
    // Java String.trim strips every char <= U+0020.
    let r: &str = s.trim_matches(|ch: char| ch <= ' ');
    Ok(c.push(r))
}

fn sort(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let a = string_arg(c, 1, "sort")?;
    let b = string_arg(c, 2, "sort")?;
    Ok(c.push(java_compare(&a, &b) == std::cmp::Ordering::Greater))
}

fn split(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let s = string_arg(c, 1, "split")?;
    let re = string_arg(c, 2, "split")?;
    let parts = java_split(&s, &re)?;
    let t = TableRef::new();
    for (n, p) in parts.into_iter().enumerate() {
        t.set_int(n as i64 + 1, p);
    }
    Ok(c.push(t))
}

// ---------------------------------------------------------------------------
// split: Java String.split(regex) with limit 0
// ---------------------------------------------------------------------------

fn remove_trailing_empty(mut v: Vec<String>) -> Vec<String> {
    while v.last().is_some_and(|s| s.is_empty()) {
        v.pop();
    }
    v
}

fn java_split(s: &str, regex: &str) -> LuaResult<Vec<String>> {
    let ru = units(regex);
    // String.split fast path: one non-metachar, or a backslash and a
    // non-alphanumeric char, and not a surrogate.
    let fast = match ru.as_slice() {
        [ch] if !".$|()[{^?*+\\".encode_utf16().any(|m| m == *ch) => Some(*ch),
        [b, ch] if *b == u('\\') && !(*ch < 0x80 && (*ch as u8).is_ascii_alphanumeric()) => Some(*ch),
        _ => None,
    }
    .filter(|&ch| !(0xD800..=0xDFFF).contains(&ch));
    if let Some(ch) = fast {
        let ch = char::from_u32(ch as u32).unwrap_or('\u{FFFD}');
        if !s.contains(ch) {
            return Ok(vec![s.to_string()]);
        }
        return Ok(remove_trailing_empty(s.split(ch).map(str::to_string).collect()));
    }
    let re = compile_java_regex(regex)?;
    let mut list = Vec::new();
    let mut index = 0usize;
    let mut last: Option<(usize, usize)> = None;
    loop {
        let next = match last {
            None => 0,
            Some((first, end)) if first == end => end + s[end..].chars().next().map_or(1, char::len_utf8),
            Some((_, end)) => end,
        };
        if next > s.len() {
            break;
        }
        let Some(m) = re.find_at(s, next) else { break };
        last = Some((m.start(), m.end()));
        if index == 0 && m.start() == 0 && m.start() == m.end() {
            continue;
        }
        list.push(s[index..m.start()].to_string());
        index = m.end();
    }
    if index == 0 {
        return Ok(vec![s.to_string()]);
    }
    list.push(s[index..].to_string());
    Ok(remove_trailing_empty(list))
}

thread_local! {
    static REGEX_CACHE: RefCell<HashMap<String, regex::Regex>> = RefCell::new(HashMap::new());
}

/// Compile a Java regex with the `regex` crate. Java's `\d`, `\s` and `\w`
/// are ASCII-only, so they are rewritten; `\Q...\E` quoting is expanded.
fn compile_java_regex(src: &str) -> LuaResult<regex::Regex> {
    if let Some(r) = REGEX_CACHE.with(|c| c.borrow().get(src).cloned()) {
        return Ok(r);
    }
    let mut out = String::with_capacity(src.len() + 8);
    let mut in_class = 0usize;
    let mut it = src.chars().peekable();
    while let Some(ch) = it.next() {
        match ch {
            '\\' => {
                let Some(n) = it.next() else {
                    out.push('\\');
                    break;
                };
                let rep = match (n, in_class > 0) {
                    ('d', false) => "[0-9]",
                    ('D', false) => "[^0-9]",
                    ('w', false) => "[a-zA-Z_0-9]",
                    ('W', false) => "[^a-zA-Z_0-9]",
                    ('s', false) => "[ \\t\\n\\x0B\\f\\r]",
                    ('S', false) => "[^ \\t\\n\\x0B\\f\\r]",
                    ('d', true) => "0-9",
                    ('w', true) => "a-zA-Z_0-9",
                    ('s', true) => " \\t\\n\\x0B\\f\\r",
                    ('Q', _) => {
                        let mut lit = String::new();
                        while let Some(q) = it.next() {
                            if q == '\\' && it.peek() == Some(&'E') {
                                it.next();
                                break;
                            }
                            lit.push(q);
                        }
                        out.push_str(&regex::escape(&lit));
                        continue;
                    }
                    _ => {
                        out.push('\\');
                        out.push(n);
                        continue;
                    }
                };
                out.push_str(rep);
            }
            '[' => {
                in_class += 1;
                out.push(ch);
            }
            ']' if in_class > 0 => {
                in_class -= 1;
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    let re = regex::Regex::new(&out)
        .map_err(|e| java_err("java.util.regex.PatternSyntaxException", format!("{e}\n{src}")))?;
    REGEX_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 64 {
            c.clear();
        }
        c.insert(src.to_string(), re.clone());
    });
    Ok(re)
}

// ---------------------------------------------------------------------------
// format
// ---------------------------------------------------------------------------

const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// The `StringBuilder` operations `format` relies on, with Java semantics.
struct Builder(Vec<u16>);

impl Builder {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn push(&mut self, c: u16) -> LuaResult<()> {
        if self.0.len() >= MAX_BUILDER {
            return Err(out_of_memory());
        }
        self.0.push(c);
        Ok(())
    }
    fn push_str(&mut self, s: &str) -> LuaResult<()> {
        for c in s.encode_utf16() {
            self.push(c)?;
        }
        Ok(())
    }
    /// `StringLib.extend`: `setLength(len + n)` (which truncates when `n` is
    /// negative), then fill the `n` new chars.
    fn extend(&mut self, n: i32, pad: u16) -> LuaResult<()> {
        let new_len = (self.0.len() as i32).wrapping_add(n);
        if new_len < 0 {
            return Err(java_err(
                "java.lang.StringIndexOutOfBoundsException",
                format!("String index out of range: {new_len}"),
            ));
        }
        if new_len as usize > MAX_BUILDER {
            return Err(out_of_memory());
        }
        self.0.resize(new_len as usize, pad);
        Ok(())
    }
    fn char_at(&self, i: usize) -> LuaResult<u16> {
        self.0.get(i).copied().ok_or_else(|| sioobe_index(i as i64, self.0.len()))
    }
    fn delete(&mut self, start: usize, end: usize) -> LuaResult<()> {
        let end = end.min(self.0.len());
        if start > end {
            return Err(java_err(
                "java.lang.StringIndexOutOfBoundsException",
                format!("start {start}, end {end}, length {}", self.0.len()),
            ));
        }
        self.0.drain(start..end);
        Ok(())
    }
}

/// `StringLib.stringBufferAppend`.
fn sb_append(sb: &mut Builder, mut value: f64, base: u32, print_zero: bool, mut min_digits: i32) -> LuaResult<()> {
    let start = sb.len();
    let base = base as f64;
    while value > 0.0 || min_digits > 0 {
        if value.is_infinite() {
            // The original never terminates here.
            return Err(out_of_memory());
        }
        let nv = (value / base).floor();
        let idx = (value - nv * base) as i32;
        if !(0..16).contains(&idx) {
            return Err(java_err(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {idx} out of bounds for length 16"),
            ));
        }
        sb.push(DIGITS[idx as usize] as u16)?;
        value = nv;
        min_digits -= 1;
    }
    if start == sb.len() && print_zero {
        sb.push(u('0'))?;
    } else {
        sb.0[start..].reverse();
    }
    Ok(())
}

/// `StringLib.roundToPrecision`.
fn round_to_precision(x: f64, precision: i32) -> f64 {
    let off = ipow(10, precision) as f64;
    kround(x * off) / off
}

/// `StringLib.roundToSignificantNumbers`.
fn round_to_significant(x: f64, precision: i32) -> LuaResult<f64> {
    if x == 0.0 {
        return Ok(0.0);
    }
    if x < 0.0 {
        return Ok(-round_to_significant(-x, precision)?);
    }
    let lower = ipow(10, precision - 1) as f64;
    let upper = lower * 10.0;
    let mut m = 1.0;
    while m * x < lower {
        m *= 10.0;
    }
    while m * x >= upper {
        if m.is_infinite() {
            // The original loops forever for values below ~1e-303.
            return Err(LuaError::runtime("string.format: value too small (the original never returns)"));
        }
        m /= 10.0;
    }
    Ok(kround(x * m) / m)
}

fn append_precision_number(sb: &mut Builder, number: f64, precision: i32, require_period: bool) -> LuaResult<()> {
    let number = round_to_precision(number, precision);
    let ipart = number.floor();
    let mut fpart = number - ipart;
    let mut i = 0;
    while i < precision {
        if fpart == 0.0 || !fpart.is_finite() {
            break; // further multiplications change nothing
        }
        fpart *= 10.0;
        i += 1;
    }
    let fpart = kround(ipart + fpart) - ipart;
    sb_append(sb, ipart, 10, true, 0)?;
    if require_period || precision > 0 {
        sb.push(u('.'))?;
    }
    sb_append(sb, fpart, 10, false, precision)
}

fn append_significant_number(
    sb: &mut Builder,
    number: f64,
    mut significant_decimals: i32,
    include_trailing_zeros: bool,
) -> LuaResult<()> {
    let ipart = number.floor();
    sb_append(sb, ipart, 10, true, 0)?;
    let mut fpart = round_to_significant(number - ipart, significant_decimals)?;
    let has_not_started = ipart == 0.0 && fpart != 0.0;
    let mut zero_padding_before = 0i32;
    let mut scan_length = significant_decimals;
    let mut i = 0;
    while i < scan_length {
        if fpart == 0.0 || !fpart.is_finite() {
            break; // every remaining iteration would `continue`
        }
        fpart *= 10.0;
        i += 1;
        if fpart.floor() != 0.0 || fpart == 0.0 {
            continue;
        }
        zero_padding_before += 1;
        if has_not_started {
            scan_length += 1;
        }
    }
    fpart = kround(fpart);
    if !include_trailing_zeros {
        while fpart > 0.0 && fpart % 10.0 == 0.0 {
            fpart /= 10.0;
            significant_decimals -= 1;
        }
    }
    sb.push(u('.'))?;
    let period_pos = sb.len();
    sb.extend(zero_padding_before, u('0'))?;
    let pre = sb.len();
    sb_append(sb, fpart, 10, false, 0)?;
    let len = (sb.len() - pre) as i32;
    if include_trailing_zeros && len < significant_decimals {
        sb.extend(significant_decimals - len - zero_padding_before, u('0'))?;
    }
    if !include_trailing_zeros && period_pos == sb.len() {
        sb.delete(period_pos - 1, sb.len())?;
    }
    Ok(())
}

fn append_scientific_number(
    sb: &mut Builder,
    mut x: f64,
    precision: i32,
    repr: bool,
    use_significant: bool,
) -> LuaResult<()> {
    let mut exponent = 0i32;
    for _ in 0..2 {
        if x.is_infinite() {
            return Err(LuaError::runtime("string.format: value too large (the original never returns)"));
        }
        if x >= 1.0 {
            while x >= 10.0 {
                x /= 10.0;
                exponent += 1;
            }
        } else {
            while x > 0.0 && x < 1.0 {
                x *= 10.0;
                exponent -= 1;
            }
        }
        x = round_to_precision(x, precision);
    }
    let abs_exponent = exponent.wrapping_abs();
    let exp_sign = if exponent >= 0 { u('+') } else { u('-') };
    if use_significant {
        append_significant_number(sb, x, precision, repr)?;
    } else {
        append_precision_number(sb, x, precision, repr)?;
    }
    sb.push(u('e'))?;
    sb.push(exp_sign)?;
    sb_append(sb, abs_exponent as f64, 10, true, 2)
}

fn format(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    const INCOMPLETE: &str = "incomplete option to 'format'";
    let f = units(&string_arg(c, 1, "format")?);
    let len = f.len();
    let mut argc = 2usize;
    let mut result = Builder(Vec::with_capacity(len + 16));
    let mut i = 0usize;
    while i < len {
        let mut ch = f[i];
        if ch != u('%') {
            result.push(ch)?;
            i += 1;
            continue;
        }
        i += 1;
        lua_assert(i < len, INCOMPLETE)?;
        ch = f[i];
        if ch == u('%') {
            result.push(u('%'))?;
            i += 1;
            continue;
        }
        let mut repr = false;
        let mut zero_padding = false;
        let mut left_justify = false;
        let mut show_plus = false;
        let mut space_for_sign = false;
        loop {
            match ch as u8 {
                _ if ch > 0x7F => break,
                b'-' => left_justify = true,
                b'+' => show_plus = true,
                b' ' => space_for_sign = true,
                b'#' => repr = true,
                b'0' => zero_padding = true,
                _ => break,
            }
            i += 1;
            lua_assert(i < len, INCOMPLETE)?;
            ch = f[i];
        }
        let mut width: i32 = 0;
        while (u('0')..=u('9')).contains(&ch) {
            width = width.wrapping_mul(10).wrapping_add(ch as i32 - 48);
            i += 1;
            lua_assert(i < len, INCOMPLETE)?;
            ch = f[i];
        }
        let mut precision: i32 = 0;
        let mut has_precision = false;
        if ch == u('.') {
            has_precision = true;
            i += 1;
            lua_assert(i < len, INCOMPLETE)?;
            ch = f[i];
            while (u('0')..=u('9')).contains(&ch) {
                precision = precision.wrapping_mul(10).wrapping_add(ch as i32 - 48);
                i += 1;
                lua_assert(i < len, INCOMPLETE)?;
                ch = f[i];
            }
        }
        if left_justify {
            zero_padding = false;
        }
        let mut base = 10u32;
        let mut upper_case = false;
        let mut default_precision = 6;
        let mut base_prepend = "";
        let conv = if ch < 0x80 { ch as u8 } else { 0 };
        match conv {
            b'c' => zero_padding = false,
            b'o' => {
                base = 8;
                default_precision = 1;
                base_prepend = "0";
            }
            b'x' => {
                base = 16;
                default_precision = 1;
                base_prepend = "0x";
            }
            b'X' => {
                base = 16;
                default_precision = 1;
                upper_case = true;
                base_prepend = "0X";
            }
            b'u' | b'd' | b'i' => default_precision = 1,
            b'e' | b'g' | b'f' => {}
            b'E' | b'G' => upper_case = true,
            b's' => zero_padding = false,
            b'q' => width = 0,
            _ => {
                return Err(LuaError::runtime(format!("invalid option '%{}' to 'format'", from_units(&[ch]))));
            }
        }
        if !has_precision {
            precision = default_precision;
        }
        if has_precision && base != 10 {
            zero_padding = false;
        }
        let pad_character = if zero_padding { u('0') } else { u(' ') };
        let start = result.len();
        if !left_justify {
            result.extend(width, pad_character)?;
        }
        match conv {
            b'c' => {
                let v = number_arg(c, argc, "format")?;
                result.push((v as i32) as u16)?;
            }
            b'X' | b'o' | b'u' | b'x' => {
                let mut v = number_arg(c, argc, "format")? as i64;
                if v < 0 {
                    v = v.wrapping_add(0x1_0000_0000);
                }
                if repr {
                    if base == 8 {
                        let mut digits = 0;
                        let mut v2 = v;
                        while v2 > 0 {
                            v2 /= 8;
                            digits += 1;
                        }
                        if precision <= digits {
                            result.push_str(base_prepend)?;
                        }
                    } else if base == 16 && v != 0 {
                        result.push_str(base_prepend)?;
                    }
                }
                if !(v == 0 && precision <= 0) {
                    sb_append(&mut result, v as f64, base, false, precision)?;
                }
            }
            b'd' | b'i' => {
                let mut v = number_arg(c, argc, "format")? as i64;
                if v < 0 {
                    result.push(u('-'))?;
                    v = v.wrapping_neg();
                } else if show_plus {
                    result.push(u('+'))?;
                } else if space_for_sign {
                    result.push(u(' '))?;
                }
                if !(v == 0 && precision <= 0) {
                    sb_append(&mut result, v as f64, base, false, precision)?;
                }
            }
            b'E' | b'e' | b'f' => {
                let v = number_arg(c, argc, "format")?;
                let is_nan = v.is_infinite() || v.is_nan();
                let mut vd = v;
                if is_negative(vd) {
                    if !is_nan {
                        result.push(u('-'))?;
                    }
                    vd = -vd;
                } else if show_plus {
                    result.push(u('+'))?;
                } else if space_for_sign {
                    result.push(u(' '))?;
                }
                if is_nan {
                    result.push_str(&number_to_string(v))?;
                } else if conv == b'f' {
                    append_precision_number(&mut result, vd, precision, repr)?;
                } else {
                    append_scientific_number(&mut result, vd, precision, repr, false)?;
                }
            }
            b'G' | b'g' => {
                if precision <= 0 {
                    precision = 1;
                }
                let v = number_arg(c, argc, "format")?;
                let is_nan = v.is_infinite() || v.is_nan();
                let mut vd = v;
                if is_negative(vd) {
                    if !is_nan {
                        result.push(u('-'))?;
                    }
                    vd = -vd;
                } else if show_plus {
                    result.push(u('+'))?;
                } else if space_for_sign {
                    result.push(u(' '))?;
                }
                if is_nan {
                    result.push_str(&number_to_string(v))?;
                } else {
                    let x = round_to_significant(vd, precision)?;
                    if x == 0.0 || (x >= 1.0e-4 && x < ipow(10, precision) as f64) {
                        let ipart_size = if x == 0.0 {
                            1
                        } else if x.floor() == 0.0 {
                            0
                        } else {
                            let mut lv = x;
                            let mut n = 1;
                            while lv >= 10.0 {
                                lv /= 10.0;
                                n += 1;
                            }
                            n
                        };
                        append_significant_number(&mut result, x, precision.wrapping_sub(ipart_size), repr)?;
                    } else {
                        append_scientific_number(&mut result, x, precision.wrapping_sub(1), repr, true)?;
                    }
                }
            }
            b's' => {
                let s = units(&string_arg(c, argc, "format")?);
                let mut n = s.len() as i64;
                if has_precision {
                    n = n.min(precision as i64);
                }
                for &cu in s.iter().take(n.max(0) as usize) {
                    result.push(cu)?;
                }
            }
            b'q' => {
                let q = units(&string_arg(c, argc, "format")?);
                result.push(u('"'))?;
                for &d in &q {
                    match d {
                        // The original appends a single backslash here.
                        0x5C => result.push(u('\\'))?,
                        0x0A => {
                            result.push(u('\\'))?;
                            result.push(u('\n'))?;
                        }
                        0x0D => {
                            result.push(u('\\'))?;
                            result.push(u('r'))?;
                        }
                        0x22 => {
                            result.push(u('\\'))?;
                            result.push(u('"'))?;
                        }
                        _ => result.push(d)?,
                    }
                }
                result.push(u('"'))?;
            }
            _ => return Err(LuaError::runtime("Internal error")),
        }
        if left_justify {
            let cur = result.len() as i64;
            let d = width as i64 - (cur - start as i64);
            if d > 0 {
                result.extend(d as i32, u(' '))?;
            }
        } else {
            let cur = result.len() as i64;
            let d = (cur - start as i64 - width as i64).min(width as i64);
            if d > 0 {
                result.delete(start, start + d as usize)?;
            }
            if zero_padding {
                let sign_pos = start as i64 + width as i64 - d;
                let ch = if sign_pos < 0 {
                    return Err(sioobe_index(sign_pos, result.len()));
                } else {
                    result.char_at(sign_pos as usize)?
                };
                if ch == u('+') || ch == u('-') || ch == u(' ') {
                    result.0[sign_pos as usize] = u('0');
                    if start >= result.len() {
                        return Err(sioobe_index(start as i64, result.len()));
                    }
                    result.0[start] = ch;
                }
            }
        }
        if upper_case {
            for cu in result.0.iter_mut().skip(start) {
                if (u('a')..=u('z')).contains(cu) {
                    *cu -= 32;
                }
            }
        }
        argc += 1;
        i += 1;
    }
    Ok(c.push(from_units(&result.0)))
}

// ---------------------------------------------------------------------------
// Lua patterns (MatchState / StringPointer)
// ---------------------------------------------------------------------------

const CAP_UNFINISHED: i32 = -1;
const CAP_POSITION: i32 = -2;
const MAX_CAPTURES: usize = 32;

#[derive(Clone, Copy)]
struct Capture {
    /// Start index in the source; `None` for a slot never used (Java `null`).
    init: Option<isize>,
    len: i32,
}

struct MatchState<'a> {
    src: &'a [u16],
    pat: &'a [u16],
    /// `endIndex`: the source length.
    end: isize,
    level: usize,
    capture: [Capture; MAX_CAPTURES],
}

impl<'a> MatchState<'a> {
    fn new(src: &'a [u16], pat: &'a [u16]) -> Self {
        MatchState {
            src,
            pat,
            end: src.len() as isize,
            level: 0,
            capture: [Capture { init: None, len: 0 }; MAX_CAPTURES],
        }
    }

    /// `StringPointer.getChar` on the pattern: 0 past the end.
    fn pc(&self, p: isize, off: isize) -> u16 {
        let i = p + off;
        if i < 0 {
            return 0;
        }
        self.pat.get(i as usize).copied().unwrap_or(0)
    }

    /// `StringPointer.getChar` on the source: 0 past the end, and Java's
    /// `charAt` exception for negative positions.
    fn sc(&self, s: isize) -> LuaResult<u16> {
        if s < 0 {
            return Err(sioobe_index(s as i64, self.src.len()));
        }
        Ok(self.src.get(s as usize).copied().unwrap_or(0))
    }

    /// `MatchState.getCapture`.
    fn get_capture(&self, i: usize) -> LuaResult<Option<Value>> {
        if i >= self.level {
            return Ok(None);
        }
        let cap = self.capture[i];
        let init = cap.init.unwrap_or(0);
        if cap.len == CAP_POSITION {
            return Ok(Some(Value::Num((init + 1) as f64)));
        }
        let s = substring(self.src, init, init + cap.len as isize)?;
        Ok(Some(Value::from(from_units(s))))
    }
}

fn class_end(ms: &MatchState<'_>, p: isize) -> LuaResult<isize> {
    let mut q = p;
    let c = ms.pc(q, 0);
    q += 1;
    if c == u('%') {
        lua_assert(ms.pc(q, 0) != 0, "malformed pattern (ends with '%')")?;
        return Ok(q + 1);
    }
    if c == u('[') {
        if ms.pc(q, 0) == u('^') {
            q += 1;
        }
        loop {
            lua_assert(ms.pc(q, 0) != 0, "malformed pattern (missing ']')")?;
            let cc = ms.pc(q, 0);
            q += 1;
            if cc == u('%') && ms.pc(q, 0) != 0 {
                q += 1;
            }
            if ms.pc(q, 0) == u(']') {
                break;
            }
        }
        return Ok(q + 1);
    }
    Ok(q)
}

fn match_class(class_id: u16, c: u16) -> bool {
    let lower = if (u('A')..=u('Z')).contains(&class_id) { class_id + 32 } else { class_id };
    let res = match lower as u8 {
        _ if lower > 0x7F => return class_id == c,
        b'a' => java_is_lower(c) || java_is_upper(c),
        b'c' => c <= 0x1F || c == 0x7F,
        b'd' => java_is_digit(c),
        b'l' => java_is_lower(c),
        b'p' => {
            (u('!')..=u('/')).contains(&c)
                || (u(':')..=u('@')).contains(&c)
                || (u('[')..=u('`')).contains(&c)
                || (u('{')..=u('~')).contains(&c)
        }
        b's' => (u('\t')..=u('\r')).contains(&c) || c == u(' '),
        b'u' => java_is_upper(c),
        b'w' => java_is_lower(c) || java_is_upper(c) || java_is_digit(c),
        b'x' => (u('0')..=u('9')).contains(&c) || (u('a')..=u('f')).contains(&c) || (u('A')..=u('F')).contains(&c),
        b'z' => c == 0,
        _ => return class_id == c,
    };
    (lower == class_id) == res
}

fn match_bracket_class(ms: &MatchState<'_>, c: u16, pp: isize, ec: isize) -> bool {
    let mut p = pp;
    let mut sig = true;
    if ms.pc(p, 1) == u('^') {
        sig = false;
        p += 1;
    }
    loop {
        p += 1;
        if p >= ec {
            break;
        }
        if ms.pc(p, 0) == u('%') {
            p += 1;
            if match_class(ms.pc(p, 0), c) {
                return sig;
            }
            continue;
        }
        if ms.pc(p, 1) == u('-') && p + 2 < ec {
            p += 2;
            if ms.pc(p, -2) <= c && c <= ms.pc(p, 0) {
                return sig;
            }
            continue;
        }
        if ms.pc(p, 0) == c {
            return sig;
        }
    }
    !sig
}

fn single_match(ms: &MatchState<'_>, c: u16, p: isize, ep: isize) -> bool {
    match ms.pc(p, 0) {
        0x2E => true, // '.'
        0x25 => match_class(ms.pc(p, 1), c),
        0x5B => match_bracket_class(ms, c, p, ep - 1),
        pc => pc == c,
    }
}

fn match_balance(ms: &MatchState<'_>, ss: isize, p: isize) -> LuaResult<Option<isize>> {
    lua_assert(ms.pc(p, 0) != 0 && ms.pc(p, 1) != 0, "unbalanced pattern")?;
    let mut s = ss;
    if ms.sc(s)? != ms.pc(p, 0) {
        return Ok(None);
    }
    let b = ms.pc(p, 0);
    let e = ms.pc(p, 1);
    let mut cont = 1;
    loop {
        s += 1;
        if s >= ms.end {
            break;
        }
        let ch = ms.sc(s)?;
        if ch == e {
            cont -= 1;
            if cont == 0 {
                return Ok(Some(s + 1));
            }
        } else if ch == b {
            cont += 1;
        }
    }
    Ok(None)
}

fn max_expand(ms: &mut MatchState<'_>, s: isize, p: isize, ep: isize) -> LuaResult<Option<isize>> {
    let mut i: isize = 0;
    while s + i < ms.end && single_match(ms, ms.sc(s + i)?, p, ep) {
        i += 1;
    }
    while i >= 0 {
        if let Some(r) = do_match(ms, s + i, ep + 1)? {
            return Ok(Some(r));
        }
        i -= 1;
    }
    Ok(None)
}

fn min_expand(ms: &mut MatchState<'_>, ss: isize, p: isize, ep: isize) -> LuaResult<Option<isize>> {
    let mut s = ss;
    loop {
        if let Some(r) = do_match(ms, s, ep + 1)? {
            return Ok(Some(r));
        }
        if s >= ms.end || !single_match(ms, ms.sc(s)?, p, ep) {
            return Ok(None);
        }
        s += 1;
    }
}

fn start_capture(ms: &mut MatchState<'_>, s: isize, p: isize, what: i32) -> LuaResult<Option<isize>> {
    let level = ms.level;
    lua_assert(level < MAX_CAPTURES, "too many captures")?;
    ms.capture[level] = Capture { init: Some(s), len: what };
    ms.level = level + 1;
    let res = do_match(ms, s, p)?;
    if res.is_none() {
        ms.level -= 1;
    }
    Ok(res)
}

fn end_capture(ms: &mut MatchState<'_>, s: isize, p: isize) -> LuaResult<Option<isize>> {
    let l = (0..ms.level)
        .rev()
        .find(|&l| ms.capture[l].len == CAP_UNFINISHED)
        .ok_or_else(|| LuaError::runtime("invalid pattern capture"))?;
    ms.capture[l].len = (s - ms.capture[l].init.unwrap_or(0)) as i32;
    let res = do_match(ms, s, p)?;
    if res.is_none() {
        ms.capture[l].len = CAP_UNFINISHED;
    }
    Ok(res)
}

fn match_capture(ms: &MatchState<'_>, s: isize, lch: u16) -> LuaResult<Option<isize>> {
    let l = lch as i32 - 49;
    // Inverted with respect to Lua: only an *invalid* capture passes.
    lua_assert(
        l < 0 || l as usize >= ms.level || ms.capture[l as usize].len == CAP_UNFINISHED,
        "invalid capture index",
    )?;
    if l < 0 || l as usize >= MAX_CAPTURES {
        return Err(java_err(
            "java.lang.ArrayIndexOutOfBoundsException",
            format!("Index {l} out of bounds for length {MAX_CAPTURES}"),
        ));
    }
    let cap = ms.capture[l as usize];
    let len = cap.len as isize;
    if s >= len {
        let Some(init) = cap.init else {
            return Err(java_err(
                "java.lang.NullPointerException",
                "Cannot invoke \"se.krka.kahlua.stdlib.StringLib$StringPointer.compareTo(se.krka.kahlua.stdlib.StringLib$StringPointer, int)\" because \"ms.capture[l].init\" is null",
            ));
        };
        for i in 0..len.max(0) {
            if ms.sc(init + i)? != ms.sc(s + i)? {
                return Ok(None);
            }
        }
        return Ok(Some(s + len));
    }
    Ok(None)
}

fn do_match(ms: &mut MatchState<'_>, mut s: isize, mut p: isize) -> LuaResult<Option<isize>> {
    loop {
        let pch = ms.pc(p, 0);
        if pch == u('(') {
            return if ms.pc(p, 1) == u(')') {
                start_capture(ms, s, p + 2, CAP_POSITION)
            } else {
                start_capture(ms, s, p + 1, CAP_UNFINISHED)
            };
        } else if pch == u(')') {
            return end_capture(ms, s, p + 1);
        } else if pch == u('%') {
            let n = ms.pc(p, 1);
            if n == u('b') {
                match match_balance(ms, s, p + 2)? {
                    Some(ns) => s = ns,
                    None => return Ok(None),
                }
                p += 4;
                continue;
            } else if n == u('f') {
                p += 2;
                lua_assert(ms.pc(p, 0) == u('['), "missing '[' after '%%f' in pattern")?;
                let ep = class_end(ms, p)?;
                let previous = if s == 0 { 0 } else { ms.sc(s - 1)? };
                if match_bracket_class(ms, previous, p, ep - 1) || !match_bracket_class(ms, ms.sc(s)?, p, ep - 1) {
                    return Ok(None);
                }
                p = ep;
                continue;
            } else if java_is_digit(n) {
                match match_capture(ms, s, n)? {
                    Some(ns) => s = ns,
                    None => return Ok(None),
                }
                p += 2;
                continue;
            }
        } else if pch == 0 {
            return Ok(Some(s));
        } else if pch == u('$') && ms.pc(p, 1) == 0 {
            return Ok(if s == ms.end { Some(s) } else { None });
        }
        // default
        let ep = class_end(ms, p)?;
        let m = s < ms.end && single_match(ms, ms.sc(s)?, p, ep);
        let epc = ms.pc(ep, 0);
        if epc == u('?') {
            if m {
                if let Some(r) = do_match(ms, s + 1, ep + 1)? {
                    return Ok(Some(r));
                }
            }
            p = ep + 1;
            continue;
        } else if epc == u('*') {
            return max_expand(ms, s, p, ep);
        } else if epc == u('+') {
            return if m { max_expand(ms, s + 1, p, ep) } else { Ok(None) };
        } else if epc == u('-') {
            return min_expand(ms, s, p, ep);
        }
        if !m {
            return Ok(None);
        }
        s += 1;
        p = ep;
    }
}

/// `push_onecapture` / `push_captures`. `whole` is the match span (`s`, `e`
/// in the original; `None` for `find`, which passes nulls).
fn captures(ms: &MatchState<'_>, whole: Option<(isize, isize)>) -> LuaResult<Vec<Value>> {
    let nlevels = if ms.level == 0 && whole.is_some() { 1 } else { ms.level };
    lua_assert(nlevels <= MAX_CAPTURES, "too many captures")?;
    let mut out = Vec::with_capacity(nlevels);
    for i in 0..nlevels {
        if i >= ms.level {
            // Only reachable for i == 0 with a whole match.
            let (s, e) = whole.unwrap_or((0, 0));
            out.push(Value::from(from_units(substring(ms.src, s, e)?)));
            continue;
        }
        let cap = ms.capture[i];
        let init = cap.init.unwrap_or(0);
        match cap.len {
            CAP_UNFINISHED => return Err(LuaError::runtime("unfinished capture")),
            CAP_POSITION => out.push(Value::Num((init + 1) as f64)),
            l => out.push(Value::from(from_units(substring(ms.src, init, init + l as isize)?))),
        }
    }
    Ok(out)
}

/// `new RuntimeException(e)` around an exception: the message becomes the
/// cause's `toString()`. Java `Error`s are not `Exception`s and pass through.
fn wrap_exception(e: LuaError) -> LuaError {
    match e {
        LuaError::Kahlua(v) => {
            let msg = if v.is_nil() { "nil".to_string() } else { java_value_of(&v) };
            java_err("java.lang.RuntimeException", format!("se.krka.kahlua.vm.KahluaException: {msg}"))
        }
        LuaError::Runtime { message, class } if class.ends_with("Error") => LuaError::Runtime { message, class },
        LuaError::Runtime { message, class } => java_err("java.lang.RuntimeException", format!("{class}: {message}")),
    }
}

fn no_special_chars(p: &[u16]) -> bool {
    !p.iter().any(|&c| c < 0x100 && b"^$*+?.([%-".contains(&(c as u8)))
}

/// Java `String.indexOf(target, from)` over code units.
fn index_of(s: &[u16], target: &[u16], from: usize) -> Option<usize> {
    if from >= s.len() {
        return if target.is_empty() { Some(s.len()) } else { None };
    }
    if target.is_empty() {
        return Some(from);
    }
    s[from..].windows(target.len()).position(|w| w == target).map(|p| p + from)
}

fn find_aux(c: &mut NativeCall<'_>, find: bool) -> LuaResult<usize> {
    find_aux_inner(c, find).map_err(wrap_exception)
}

fn find_aux_inner(c: &mut NativeCall<'_>, find: bool) -> LuaResult<usize> {
    let f = if find { "find" } else { "match" };
    let source = units(&string_arg(c, 1, f)?);
    let pattern = units(&string_arg(c, 2, f)?);
    let i = opt_number_arg(c, 3);
    let plain = opt_arg(c, 4).truthy();
    let n = source.len() as i32;
    let mut init = match i {
        None => 0,
        Some(v) => (v as i32).wrapping_sub(1),
    };
    if init < 0 {
        init = init.wrapping_add(n);
        if init < 0 {
            init = 0;
        }
    } else if init > n {
        init = n;
    }
    if find && (plain || no_special_chars(&pattern)) {
        if let Some(pos) = index_of(&source, &pattern, init as usize) {
            c.push((pos + 1) as f64);
            return Ok(1 + c.push((pos + pattern.len()) as f64));
        }
    } else {
        let mut p = 0isize;
        let mut anchor = false;
        let mut ms = MatchState::new(&source, &pattern);
        if ms.pc(0, 0) == u('^') {
            anchor = true;
            p = 1;
        }
        let mut s1 = init as isize;
        loop {
            ms.level = 0;
            if let Some(res) = do_match(&mut ms, s1, p)? {
                let vals = if find {
                    let mut v = vec![Value::Num((s1 + 1) as f64), Value::Num(res as f64)];
                    v.extend(captures(&ms, None)?);
                    v
                } else {
                    captures(&ms, Some((s1, res)))?
                };
                let k = vals.len();
                for v in vals {
                    c.push(v);
                }
                return Ok(k);
            }
            let old = s1;
            s1 += 1;
            if !(old < ms.end && !anchor) {
                break;
            }
        }
    }
    Ok(c.push(Value::Nil))
}

// ---------------------------------------------------------------------------
// gsub
// ---------------------------------------------------------------------------

fn gsub(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    gsub_inner(c).map_err(wrap_exception)
}

fn gsub_inner(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let src = units(&string_arg(c, 1, "gsub")?);
    let pat = units(&string_arg(c, 2, "gsub")?);
    let mut repl = arg(c, 3, "gsub")?;
    if let Some(t) = raw_tostring(&repl) {
        repl = Value::Str(t);
    }
    let max_substitutions = opt_number_arg(c, 4).map_or(i32::MAX, |v| v as i32);
    let mut ms = MatchState::new(&src, &pat);
    let mut p = 0isize;
    let mut anchor = false;
    if ms.pc(0, 0) == u('^') {
        anchor = true;
        p = 1;
    }
    if !matches!(repl, Value::Num(_) | Value::Str(_) | Value::Closure(_) | Value::Native(_) | Value::Table(_)) {
        return Err(LuaError::runtime(format!("string/function/table expected, got {}", java_value_of(&repl))));
    }
    let repl_units = raw_tostring(&repl).map(|s| units(&s));
    let mut n = 0i32;
    let mut b: Vec<u16> = Vec::with_capacity(src.len());
    let mut s: isize = 0;
    while n < max_substitutions {
        ms.level = 0;
        let e = do_match(&mut ms, s, p)?;
        if let Some(e) = e {
            n += 1;
            add_value(c.thread, &ms, &repl, repl_units.as_deref(), &mut b, s, e)?;
        }
        match e {
            Some(e) if e > s => s = e,
            _ => {
                if s >= ms.end {
                    break;
                }
                b.push(ms.sc(s)?);
                s += 1;
            }
        }
        if anchor {
            break;
        }
    }
    let rest = if s == 0 { &src[..] } else { substring(&src, s, src.len() as isize)? };
    b.extend_from_slice(rest);
    c.push(from_units(&b));
    Ok(1 + c.push(n as f64))
}

fn add_value(
    thread: &mut Thread,
    ms: &MatchState<'_>,
    repl: &Value,
    repl_units: Option<&[u16]>,
    b: &mut Vec<u16>,
    s: isize,
    e: isize,
) -> LuaResult<()> {
    if let Some(r) = repl_units {
        let add = add_string(ms, r, s, e)?;
        b.extend_from_slice(&add);
        return Ok(());
    }
    let m: Rc<str> = match ms.get_capture(0)? {
        Some(v) => raw_tostring(&v).unwrap_or_else(|| Rc::from("")),
        None => Rc::from(from_units(substring(ms.src, s, e)?)),
    };
    let res = match repl {
        Value::Table(t) => t.rawget(&Value::Str(m.clone())),
        f => thread.call1(f, &[Value::Str(m.clone()), Value::Nil, Value::Nil])?,
    };
    let res = if res.is_nil() { Value::Str(m) } else { res };
    match raw_tostring(&res) {
        Some(r) => b.extend(units(&r)),
        None => b.extend("null".encode_utf16()),
    }
    Ok(())
}

fn add_string(ms: &MatchState<'_>, repl: &[u16], s: isize, e: isize) -> LuaResult<Vec<u16>> {
    let get = |i: usize| repl.get(i).copied().unwrap_or(0);
    let mut buf = Vec::with_capacity(repl.len());
    let mut i = 0usize;
    while i < repl.len() {
        let ch = get(i);
        if ch != u('%') {
            buf.push(ch);
            i += 1;
            continue;
        }
        i += 1;
        let ch = get(i);
        if !java_is_digit(ch) {
            buf.push(ch);
        } else if ch == u('0') {
            buf.extend_from_slice(substring(ms.src, s, e)?);
        } else {
            match ms.get_capture((ch - 49) as usize)? {
                None => return Err(LuaError::kahlua("invalid capture index")),
                Some(Value::Num(d)) => buf.extend(number_to_string(d).encode_utf16()),
                Some(v) => buf.extend(units(v.as_str().unwrap_or(""))),
            }
        }
        i += 1;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::LuaClosure;

    fn thread() -> (Thread, TableRef) {
        let env = TableRef::new();
        let mut t = Thread::new(env.clone());
        register(&mut t, &env);
        (t, env)
    }

    fn try_run(src: &str) -> LuaResult<Vec<Value>> {
        let (mut t, env) = thread();
        // Non-ASCII test strings go in as globals: the compiler's lexer
        // currently stores source chars as bytes.
        for (k, v) in [("U_SS", "straße"), ("U_E", "é"), ("U_EMOJI", "a😀b"), ("U_WORD", "Ünïcode"), ("U_DIGITS", "٣4")] {
            env.set_str(k, v);
        }
        let p = crate::compiler::compile(src, Some("test"), None, None)?;
        let f = Value::Closure(LuaClosure::new(Rc::new(p), env));
        t.call(&f, &[])
    }

    fn run(src: &str) -> Vec<Value> {
        try_run(src).unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    /// The results rendered like `tostring`, joined by `|`.
    fn show(src: &str) -> String {
        run(src)
            .iter()
            .map(|v| match v {
                Value::Nil => "nil".to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Num(n) => number_to_string(*n),
                Value::Str(s) => s.to_string(),
                other => format!("{other:?}"),
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    fn err(src: &str) -> String {
        match try_run(src) {
            Ok(v) => panic!("{src}: expected an error, got {v:?}"),
            Err(e) => match e.pcall_message() {
                Value::Str(s) => s.to_string(),
                v => format!("{v:?}"),
            },
        }
    }

    fn fmt(args: &str) -> String {
        show(&format!("return string.format({args})"))
    }

    #[test]
    fn format_integers() {
        assert_eq!(fmt("'%d', 42"), "42");
        assert_eq!(fmt("'%d', -42.9"), "-42");
        assert_eq!(fmt("'%5d|%-5d|%05d', 42, 42, -42"), "   42|42   |-0042");
        assert_eq!(fmt("'%+d % d %+05d', 7, 7, 7"), "+7  7 +0007");
        assert_eq!(fmt("'%.3d', 7"), "007");
        assert_eq!(fmt("'%.0d|', 0"), "|");
        assert_eq!(fmt("'%i', '12'"), "12");
        assert_eq!(fmt("'%x %X %o', 255, 255, 8"), "ff FF 10");
        assert_eq!(fmt("'%#x %#X %#o %#x', 255, 255, 8, 0"), "0xff 0XFF 010 0");
        assert_eq!(fmt("'%x', -1"), "ffffffff");
        assert_eq!(fmt("'%04x|%.4x|%04.2x', 10, 10, 10"), "000a|000a|  0a");
        assert_eq!(fmt("'%u', 3.7"), "3");
        assert_eq!(fmt("'%c%c%c', 72, 105, 33"), "Hi!");
        assert_eq!(fmt("'%%|%3c|', 65"), "%|  A|");
    }

    #[test]
    fn format_floats() {
        assert_eq!(fmt("'%f', 3.14159"), "3.141590");
        assert_eq!(fmt("'%5.2f', 3.14159"), " 3.14");
        assert_eq!(fmt("'%-8.3f|', 2.5"), "2.500   |");
        assert_eq!(fmt("'%08.2f', -3.5"), "-0003.50");
        assert_eq!(fmt("'%.0f %.0f %.0f', 0.5, 1.5, 2.5"), "0 2 2");
        assert_eq!(fmt("'%#.0f', 3"), "3.");
        assert_eq!(fmt("'%f', -0.0"), "-0.000000");
        assert_eq!(fmt("'%+.1f', 1"), "+1.0");
        assert_eq!(fmt("'%f %f %f', 1/0, -1/0, 0/0"), "inf -inf nan");
        assert_eq!(fmt("'%5.1f|%E', 1/0, 1/0"), "  inf|INF");
        assert_eq!(fmt("'%e', 12345.678"), "1.234568e+04");
        assert_eq!(fmt("'%.2e', 0.000123"), "1.23e-04");
        assert_eq!(fmt("'%E', 0"), "0.000000E+00");
        assert_eq!(fmt("'%g', 100000"), "100000");
        assert_eq!(fmt("'%g', 1000000"), "1e+06");
        assert_eq!(fmt("'%g', 0.1"), "0.1");
        assert_eq!(fmt("'%g', 0.0001"), "0.0001");
        assert_eq!(fmt("'%g', 0.00001"), "1e-05");
        assert_eq!(fmt("'%g', 3.14159265"), "3.14159");
        assert_eq!(fmt("'%g %g', 0, 2.5"), "0 2.5");
        assert_eq!(fmt("'%.3g', 1234.5"), "1.23e+03");
        assert_eq!(fmt("'%#g', 1.5"), "1.50000");
        assert_eq!(fmt("'%G', 1e-10"), "1E-10");
        assert_eq!(fmt("'%10.4g|', 3.14159"), "     3.142|");
    }

    #[test]
    fn format_strings() {
        assert_eq!(fmt("'[%s]', 'abc'"), "[abc]");
        assert_eq!(fmt("'[%5s][%-5s][%.2s][%05s]', 'ab', 'ab', 'abc', 'x'"), "[   ab][ab   ][ab][    x]");
        assert_eq!(fmt("'%s %s', 1, 2.5"), "1 2.5");
        // Backslashes are not escaped by Kahlua's %q.
        assert_eq!(fmt(r#"'%q', 'a"b\\c\nd\re'"#), "\"a\\\"b\\c\\\nd\\re\"");
        assert_eq!(fmt("'%10q', 'x'"), "\"x\"");
    }

    #[test]
    fn format_errors() {
        assert_eq!(err("return string.format('%')"), "incomplete option to 'format' java.lang.RuntimeException");
        assert_eq!(err("return string.format('%5')"), "incomplete option to 'format' java.lang.RuntimeException");
        assert_eq!(err("return string.format('%y', 1)"), "invalid option '%y' to 'format' java.lang.RuntimeException");
        assert_eq!(err("return string.format('%d')"), "missing argument #2 to 'format' java.lang.RuntimeException");
        assert_eq!(
            err("return string.format('%d', 'x')"),
            "bad argument #2 to 'format' (double expected, got nil) java.lang.RuntimeException"
        );
    }

    #[test]
    fn simple_functions() {
        assert_eq!(show("return ('hello'):sub(2, 3), ('hello'):sub(-3), ('hello'):sub(0), ('hello'):sub(4, 2)"), "el|llo|hello|");
        assert_eq!(show("return ('hello'):sub(2, 100), ('hello'):sub(-100, 2)"), "ello|he");
        assert_eq!(show("return string.byte('ABC'), string.byte('ABC', 2, -1)"), "65|66|67");
        assert_eq!(show("return string.byte('ABC', 10)"), "");
        assert_eq!(show("return string.char(72, 105), string.char()"), "Hi|");
        assert_eq!(show("return ('MiXeD'):lower(), ('MiXeD'):upper(), U_SS:upper()"), "mixed|MIXED|STRASSE");
        assert_eq!(show("return ('abc'):reverse(), string.reverse('')"), "cba|");
        assert_eq!(show("return string.trim('  \\t hi \\n')"), "hi");
        assert_eq!(show("return string.contains('hello', 'ell'), string.contains('hello', 'x')"), "true|false");
        assert_eq!(show("return string.sort('b', 'a'), string.sort('a', 'b'), string.sort('a', 'a')"), "true|false|false");
        // UTF-16 code units: é is one unit, the emoji two.
        assert_eq!(show("return string.byte(U_E), U_EMOJI:sub(2, 3), U_EMOJI:sub(4)"), "233|😀|b");
        assert_eq!(err("return string.lower()"), "not enough arguments java.lang.RuntimeException");
        assert_eq!(err("return string.sub('x')"), "missing argument #2 to 'sub' java.lang.RuntimeException");
    }

    #[test]
    fn split_uses_java_regex() {
        let src = "local t = string.split(...) local s = #t .. ':' for i = 1, #t do s = s .. '[' .. t[i] .. ']' end return s";
        let split = |a: &str, b: &str| show(&src.replace("...", &format!("'{a}', '{b}'")));
        assert_eq!(split("a,b,,c,,", ","), "4:[a][b][][c]");
        assert_eq!(split(",a", ","), "2:[][a]");
        assert_eq!(split("", ","), "1:[]");
        assert_eq!(split(",,", ","), "0:");
        assert_eq!(split("a b  c", " +"), "3:[a][b][c]");
        assert_eq!(split("a1b22c", "\\\\d+"), "3:[a][b][c]");
        assert_eq!(split("a.b", "."), "0:");
        assert_eq!(split("a.b", "\\\\."), "2:[a][b]");
        assert_eq!(split("abc", ""), "3:[a][b][c]");
        assert_eq!(split("abc", "b*"), "3:[a][][c]");
        assert_eq!(split("a|b", "|"), "3:[a][|][b]");
    }

    #[test]
    fn find_plain_and_patterns() {
        assert_eq!(show("return string.find('hello world', 'o w')"), "5|7");
        assert_eq!(show("return string.find('a.b', '.', 1, true)"), "2|2");
        assert_eq!(show("return string.find('hello', 'l+')"), "3|4");
        assert_eq!(show("return string.find('hello', '(l)(l)')"), "3|4|l|l");
        assert_eq!(show("return string.find('hello', 'xyz')"), "nil");
        assert_eq!(show("return string.find('hello', '')"), "1|0");
        assert_eq!(show("return string.find('abc', '', 10)"), "4|3");
        // Kahlua's negative init is one unit further left than Lua's.
        assert_eq!(show("return string.find('abcabc', 'c', -1)"), "6|6");
        assert_eq!(show("return string.find('abcabc', 'b', -1)"), "5|5");
        assert_eq!(show("return string.find('abcabc', 'b', -2)"), "5|5");
        assert_eq!(show("return string.find('abcabc', 'a', 0)"), "nil");
        assert_eq!(show("return string.find('abc', '^b'), string.find('abc', '^a')"), "nil|1|1");
        assert_eq!(show("return string.find('abc', 'c$')"), "3|3");
        assert_eq!(show("return string.find(U_EMOJI, 'b')"), "4|4");
    }

    #[test]
    fn match_captures_and_classes() {
        assert_eq!(show("return string.match('key = value', '(%w+)%s*=%s*(%w+)')"), "key|value");
        assert_eq!(show("return string.match('hello 123', '%d+')"), "123");
        assert_eq!(show("return string.match('  x', '()x()')"), "3|4");
        assert_eq!(show("return string.match('f(a(b)c)d', '%b()')"), "(a(b)c)");
        assert_eq!(show("return string.match('THE (quick) fox', '%f[%a]%a+', 5)"), "quick");
        assert_eq!(show("return string.match('abc', '[^a]+'), string.match('a-z', '[%-]')"), "bc|-");
        assert_eq!(show("return string.match('x123y', '[0-9]+'), string.match('aAbB', '%u+')"), "123|A");
        assert_eq!(show("return string.match('abc', 'a-'), string.match('abc', 'a.-c'), string.match('ac', 'ab?c')"), "|abc|ac");
        assert_eq!(show("return string.match(U_WORD, '%a+')"), "Ünïcode");
        assert_eq!(show("return string.match(U_DIGITS, '%d+')"), "٣4");
        assert_eq!(show("return string.match('hello', '.-(l+)(.*)')"), "ll|o");
        assert_eq!(show("return string.match('  trim  ', '^%s*(.-)%s*$')"), "trim");
        assert_eq!(show("return string.match('a,b', '%P+')"), "a");
        assert_eq!(show("return string.match('abc', 'x')"), "nil");
    }

    #[test]
    fn pattern_errors_are_wrapped() {
        assert_eq!(
            err("return string.find('abc', '%')"),
            "java.lang.RuntimeException: malformed pattern (ends with '%') java.lang.RuntimeException"
        );
        assert_eq!(
            err("return string.match('abc', '[a')"),
            "java.lang.RuntimeException: malformed pattern (missing ']') java.lang.RuntimeException"
        );
        assert_eq!(
            err("return string.match('abc', '(a')"),
            "java.lang.RuntimeException: unfinished capture java.lang.RuntimeException"
        );
        assert_eq!(
            err("return string.match('abc', 'a)')"),
            "java.lang.RuntimeException: invalid pattern capture java.lang.RuntimeException"
        );
        assert_eq!(
            err("return string.match('abc', '%f')"),
            "java.lang.RuntimeException: missing '[' after '%%f' in pattern java.lang.RuntimeException"
        );
        // A back-reference to a closed capture is rejected by Kahlua.
        assert_eq!(
            err("return string.match('aa', '(a)%1')"),
            "java.lang.RuntimeException: invalid capture index java.lang.RuntimeException"
        );
        assert_eq!(
            err("return string.find(nil, 'x')"),
            "java.lang.RuntimeException: missing argument #1 to 'find' java.lang.RuntimeException"
        );
    }

    #[test]
    fn gsub_string_replacement() {
        assert_eq!(show("return string.gsub('hello world', 'o', '0')"), "hell0 w0rld|2");
        assert_eq!(show("return string.gsub('hello world', 'o', '0', 1)"), "hell0 world|1");
        assert_eq!(show("return string.gsub('abc', 'x', 'y')"), "abc|0");
        assert_eq!(show("return string.gsub('hello world', '(%w+)', '<%1>')"), "<hello> <world>|2");
        assert_eq!(show("return string.gsub('abc', '%w', '%0%0')"), "aabbcc|3");
        assert_eq!(show("return string.gsub('abc', '', '-')"), "-a-b-c-|4");
        assert_eq!(show("return string.gsub('aaa', '^a', 'b')"), "baa|1");
        assert_eq!(show("return string.gsub('a.b', '%.', '%%')"), "a%b|1");
        assert_eq!(show("return string.gsub('ab', 'b', 5)"), "a5|1");
        assert_eq!(show("return string.gsub('x', '()', '%1')"), "1x2|2");
        assert_eq!(show("return string.byte(string.gsub('ab', 'a', '%'), 1)"), "0");
        assert_eq!(show("return string.gsub('abc', 'b', 'x', 0)"), "abc|0");
    }

    #[test]
    fn gsub_function_and_table_replacement() {
        assert_eq!(show("return string.gsub('$a $b $c', '%$(%w)', {a = 'A', b = 2})"), "A 2 c|3");
        assert_eq!(show("return string.gsub('abc', '%w', function(c) return c:upper() .. '.' end)"), "A.B.C.|3");
        // Only the first capture reaches the function, plus two nils.
        assert_eq!(
            show("return string.gsub('k=v', '(%w)=(%w)', function(a, b, c) return a .. (b == nil and c == nil and '!' or '?') end)"),
            "k!|1"
        );
        // nil keeps the match; other non-strings append "null".
        assert_eq!(show("return string.gsub('ab', '%w', function(c) if c == 'a' then return nil end return false end)"), "anull|2");
        assert_eq!(show("return string.gsub('ab', '%w', function(c) return 1.5 end)"), "1.51.5|2");
    }

    #[test]
    fn gsub_errors() {
        assert_eq!(
            err("return string.gsub('abc', 'b', '%1')"),
            "se.krka.kahlua.vm.KahluaException: invalid capture index java.lang.RuntimeException"
        );
        assert_eq!(
            err("return string.gsub('abc', 'b', true)"),
            "java.lang.RuntimeException: string/function/table expected, got true java.lang.RuntimeException"
        );
        assert_eq!(
            err("return string.gsub('abc', 'b')"),
            "java.lang.RuntimeException: missing argument #3 to 'gsub' java.lang.RuntimeException"
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(kround(2.5), 2.0);
        assert_eq!(kround(3.5), 4.0);
        assert_eq!(kround(-2.5), -2.0);
        assert_eq!(ipow(10, 3), 1000);
        assert_eq!(ipow(10, 0), 1);
        assert!(java_is_digit(u('7')) && java_is_digit(0x0663) && !java_is_digit(u('a')) && !java_is_digit(0x00B2));
        assert!(match_class(u('a'), u('é')) && !match_class(u('A'), u('é')) && match_class(u('z'), 0));
        assert!(!match_class(u('a'), 0x4E2D)); // CJK is neither upper nor lower case
    }
}
