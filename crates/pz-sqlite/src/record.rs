//! Varints, the record format and value conversions.
//!
//! See "2.1. Record Format" and "1.6. B-tree Pages" of the SQLite file
//! format document: varints are big-endian, 7 bits per byte with the high
//! bit as continuation, except that a ninth byte contributes all 8 bits.

use crate::{Error, Result, TextEncoding};

/// Reads a varint from the start of `b`: `(value, length)`.
pub fn varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    for i in 0..8 {
        let c = *b.get(i)?;
        v = (v << 7) | u64::from(c & 0x7f);
        if c & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    let c = *b.get(8)?;
    Some(((v << 8) | u64::from(c), 9))
}

/// One column value, in its storage class.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    /// Decoded from the database text encoding; invalid sequences become
    /// U+FFFD, as when the JDBC driver builds a `String`.
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// `sqlite3_column_int64`: NULL is 0, reals truncate toward zero with
    /// saturation, text and blobs parse a leading integer
    /// (`sqlite3Atoi64`).
    pub fn as_i64(&self) -> i64 {
        match self {
            Value::Null => 0,
            Value::Integer(i) => *i,
            // `doubleToInt64`: saturating, NaN -> 0; Rust's `as` matches.
            Value::Real(r) => *r as i64,
            Value::Text(s) => atoi64(s.as_bytes()),
            Value::Blob(b) => atoi64(b),
        }
    }

    /// `sqlite3_column_int`: the 64-bit value truncated to 32 bits.
    pub fn as_i32(&self) -> i32 {
        self.as_i64() as i32
    }

    /// `sqlite3_column_text`: `None` for NULL; numbers rendered as SQLite
    /// renders them (`%lld`, `%!.15g`).
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Null => None,
            Value::Integer(i) => Some(i.to_string()),
            Value::Real(r) => Some(real_to_text(*r)),
            Value::Text(s) => Some(s.clone()),
            Value::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
        }
    }
}

/// `sqlite3Atoi64` as `sqlite3VdbeIntValue` uses it: leading whitespace, an
/// optional sign, then decimal digits up to the first non-digit. Overflow
/// saturates.
fn atoi64(b: &[u8]) -> i64 {
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    let neg = match b.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: u64 = 0;
    let mut overflow = false;
    while let Some(d) = b.get(i).filter(|c| c.is_ascii_digit()) {
        match v
            .checked_mul(10)
            .and_then(|x| x.checked_add(u64::from(d - b'0')))
        {
            Some(x) => v = x,
            None => overflow = true,
        }
        i += 1;
    }
    if neg {
        if overflow || v > 1 << 63 {
            i64::MIN
        } else {
            (v as i64).wrapping_neg()
        }
    } else if overflow || v > i64::MAX as u64 {
        i64::MAX
    } else {
        v as i64
    }
}

/// SQLite's `%!.15g` rendering of a REAL: 15 significant digits, trailing
/// zeros dropped, and always a decimal point (`1.0`, `1.0e+20`).
pub fn real_to_text(r: f64) -> String {
    if r.is_nan() {
        return "NaN".into();
    }
    if r.is_infinite() {
        return if r > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    if r == 0.0 {
        return if r.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    // Round to 15 significant digits through the exponent form.
    let e = format!("{:.14e}", r);
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let sign = if neg { "-" } else { "" };
    if !(-4..15).contains(&exp) {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() { "0" } else { rest };
        let es = if exp < 0 { '-' } else { '+' };
        return format!("{sign}{first}.{rest}e{es}{:02}", exp.abs());
    }
    if exp < 0 {
        let zeros = "0".repeat((-exp - 1) as usize);
        return format!("{sign}0.{zeros}{digits}");
    }
    let int_len = exp as usize + 1;
    if digits.len() <= int_len {
        let pad = "0".repeat(int_len - digits.len());
        format!("{sign}{digits}{pad}.0")
    } else {
        format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
    }
}

/// Body length of a serial type, or `None` for the reserved types 10, 11.
fn serial_len(t: u64) -> Option<usize> {
    Some(match t {
        0 | 8 | 9 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 6,
        6 | 7 => 8,
        10 | 11 => return None,
        n => ((n - 12) / 2) as usize,
    })
}

/// Sign-extends a big-endian two's complement integer of 1..=8 bytes.
fn be_int(b: &[u8]) -> i64 {
    let mut v: i64 = if b[0] & 0x80 != 0 { -1 } else { 0 };
    for &c in b {
        v = (v << 8) | i64::from(c);
    }
    v
}

fn decode_text(b: &[u8], enc: TextEncoding) -> String {
    match enc {
        TextEncoding::Utf8 => String::from_utf8_lossy(b).into_owned(),
        TextEncoding::Utf16le | TextEncoding::Utf16be => {
            let units: Vec<u16> = b
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| {
                    if enc == TextEncoding::Utf16le {
                        u16::from_le_bytes(c)
                    } else {
                        u16::from_be_bytes(c)
                    }
                })
                .collect();
            String::from_utf16_lossy(&units)
        }
    }
}

/// Decodes a record (a complete cell payload) into its column values.
pub fn decode_record(payload: &[u8], enc: TextEncoding) -> Result<Vec<Value>> {
    let corrupt = || Error::Corrupt("malformed record".into());
    let (hlen, n) = varint(payload).ok_or_else(corrupt)?;
    let hlen = usize::try_from(hlen).map_err(|_| corrupt())?;
    if hlen < n || hlen > payload.len() {
        return Err(corrupt());
    }
    let mut pos = n;
    let mut body = hlen;
    let mut out = Vec::new();
    while pos < hlen {
        let (t, n) = varint(&payload[pos..hlen]).ok_or_else(corrupt)?;
        pos += n;
        let len = serial_len(t).ok_or_else(corrupt)?;
        let b = payload.get(body..body + len).ok_or_else(corrupt)?;
        body += len;
        out.push(match t {
            0 => Value::Null,
            1..=6 => Value::Integer(be_int(b)),
            7 => Value::Real(f64::from_bits(u64::from_be_bytes(b.try_into().unwrap()))),
            8 => Value::Integer(0),
            9 => Value::Integer(1),
            t if t % 2 == 0 => Value::Blob(b.to_vec()),
            _ => Value::Text(decode_text(b, enc)),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints() {
        assert_eq!(varint(&[0x00]), Some((0, 1)));
        assert_eq!(varint(&[0x7f]), Some((127, 1)));
        assert_eq!(varint(&[0x81, 0x00]), Some((128, 2)));
        assert_eq!(varint(&[0x82, 0x80, 0x00]), Some((0x8000, 3)));
        assert_eq!(varint(&[0xff; 9]), Some((u64::MAX, 9)));
        assert_eq!(varint(&[0x81]), None);
        // The ninth byte keeps its high bit as data.
        let b = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x81];
        assert_eq!(varint(&b), Some((0x81, 9)));
    }

    #[test]
    fn integers_sign_extend() {
        assert_eq!(be_int(&[0xff]), -1);
        assert_eq!(be_int(&[0x7f, 0xff]), 32767);
        assert_eq!(be_int(&[0xff, 0xff, 0xfe]), -2);
        assert_eq!(be_int(&[0x80, 0, 0, 0, 0, 0]), -(1 << 47));
    }

    #[test]
    fn records() {
        // header: len 6, types NULL, int8, 0, 1, text(2)  ; then body
        let rec = [6, 0, 1, 8, 9, 17, 0xfe, b'h', b'i'];
        let v = decode_record(&rec, TextEncoding::Utf8).unwrap();
        assert_eq!(
            v,
            vec![
                Value::Null,
                Value::Integer(-2),
                Value::Integer(0),
                Value::Integer(1),
                Value::Text("hi".into())
            ]
        );
        assert!(decode_record(&[3, 10, 0], TextEncoding::Utf8).is_err());
        assert!(decode_record(&[2, 4, 0], TextEncoding::Utf8).is_err());
        let rec = [2, 17, 0, b'h'];
        assert_eq!(
            decode_record(&rec, TextEncoding::Utf16be).unwrap(),
            vec![Value::Text("h".into())]
        );
    }

    #[test]
    fn conversions() {
        assert_eq!(Value::Text("  -12abc".into()).as_i64(), -12);
        assert_eq!(Value::Text("1.9".into()).as_i64(), 1);
        assert_eq!(Value::Text("x".into()).as_i64(), 0);
        assert_eq!(
            Value::Text("99999999999999999999".into()).as_i64(),
            i64::MAX
        );
        assert_eq!(Value::Real(-2.7).as_i64(), -2);
        assert_eq!(Value::Real(f64::NAN).as_i64(), 0);
        assert_eq!(Value::Integer(1 << 32 | 5).as_i32(), 5);
        assert_eq!(Value::Null.as_text(), None);
        assert_eq!(Value::Integer(-7).as_text().unwrap(), "-7");
        assert_eq!(real_to_text(1.0), "1.0");
        assert_eq!(real_to_text(1.5), "1.5");
        assert_eq!(real_to_text(0.1), "0.1");
        assert_eq!(real_to_text(-123.25), "-123.25");
        assert_eq!(real_to_text(1e20), "1.0e+20");
        assert_eq!(real_to_text(1.25e-7), "1.25e-07");
        assert_eq!(real_to_text(0.001), "0.001");
        assert_eq!(real_to_text(123456789012345.0), "123456789012345.0");
        assert_eq!(real_to_text(1e15), "1.0e+15");
    }
}
