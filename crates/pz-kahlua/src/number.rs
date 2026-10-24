//! Number formatting and parsing with Java/Kahlua semantics.

/// `KahluaUtil.numberToString`.
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "nan".into();
    }
    if n.is_infinite() {
        return if n < 0.0 { "-inf".into() } else { "inf".into() };
    }
    if n.floor() == n && n.abs() < 1.0e14 {
        return format!("{}", n as i64);
    }
    java_double_to_string(n)
}

/// Java `Double.toString` (JDK 19+ shortest-digits algorithm).
pub fn java_double_to_string(d: f64) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d < 0.0 { "-Infinity".into() } else { "Infinity".into() };
    }
    if d == 0.0 {
        return if d.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // Shortest round-trip digits, as Rust's `{:e}` produces them.
    let sci = format!("{:e}", d.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let sign = if d < 0.0 { "-" } else { "" };
    let abs = d.abs();
    if (1.0e-3..1.0e7).contains(&abs) {
        let point = exp + 1; // digits before the decimal point
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        format!("{sign}{s}")
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{}", &digits[..1], frac, exp)
    }
}

/// Java `Double.parseDouble`. Returns `None` on `NumberFormatException`.
pub fn java_parse_double(s: &str) -> Option<f64> {
    // Java trims chars <= ' ' at both ends.
    let t = s.trim_matches(|c: char| c <= ' ');
    if t.is_empty() {
        return None;
    }
    let (neg, body) = match t.as_bytes()[0] {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    let v = match body {
        "NaN" => Some(f64::NAN),
        "Infinity" => Some(f64::INFINITY),
        _ if body.starts_with("0x") || body.starts_with("0X") => parse_hex_float(&body[2..]),
        _ => {
            // Optional float-type suffix.
            let core = body
                .strip_suffix(['d', 'D', 'f', 'F'])
                .unwrap_or(body);
            let ok = !core.is_empty()
                && core.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
                && core.bytes().any(|b| b.is_ascii_digit());
            if !ok {
                return None;
            }
            // Reject forms Rust accepts but Java does not, and vice versa.
            let mantissa_end = core.find(['e', 'E']).unwrap_or(core.len());
            let m = &core[..mantissa_end];
            if m.contains(['+', '-']) || m.matches('.').count() > 1 || !m.bytes().any(|b| b.is_ascii_digit()) {
                return None;
            }
            if mantissa_end < core.len() {
                let e = &core[mantissa_end + 1..];
                let e = e.strip_prefix(['+', '-']).unwrap_or(e);
                if e.is_empty() || !e.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
            }
            core.parse::<f64>().ok()
        }
    }?;
    Some(if neg { -v } else { v })
}

/// Hex floating literal body after `0x`: requires a `p` exponent in Java.
fn parse_hex_float(s: &str) -> Option<f64> {
    let s = s.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(s);
    let (m, e) = s.split_once(['p', 'P'])?;
    let exp: i32 = e.parse().ok()?;
    let (int, frac) = m.split_once('.').unwrap_or((m, ""));
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    let mut v = 0f64;
    for c in int.chars() {
        v = v * 16.0 + c.to_digit(16)? as f64;
    }
    let mut scale = 1.0 / 16.0;
    for c in frac.chars() {
        v += c.to_digit(16)? as f64 * scale;
        scale /= 16.0;
    }
    Some(v * 2f64.powi(exp))
}

/// Java `Integer.parseInt(s, radix)`.
pub fn java_parse_int(s: &str, radix: u32) -> Option<i32> {
    if s.is_empty() {
        return None;
    }
    let (neg, digits) = match s.as_bytes()[0] {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() {
        return None;
    }
    let mut v: i64 = 0;
    for c in digits.chars() {
        v = v * radix as i64 + c.to_digit(radix)? as i64;
        if v > i32::MAX as i64 + 1 {
            return None;
        }
    }
    let v = if neg { -v } else { v };
    i32::try_from(v).ok()
}

/// `KahluaUtil.tonumber(s, radix)`. `Err` mirrors the "base out of range"
/// RuntimeException.
pub fn kahlua_tonumber(s: &str, radix: i32) -> Result<Option<f64>, &'static str> {
    if !(2..=36).contains(&radix) {
        return Err("base out of range");
    }
    let parsed = if radix == 10 {
        java_parse_double(s)
    } else {
        java_parse_int(s, radix as u32).map(|v| v as f64)
    };
    if parsed.is_some() {
        return Ok(parsed);
    }
    let lower = s.to_lowercase();
    if lower.ends_with("nan") {
        return Ok(Some(f64::NAN));
    }
    if lower.ends_with("inf") {
        return Ok(Some(if lower.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_kahlua() {
        assert_eq!(number_to_string(1.0), "1");
        assert_eq!(number_to_string(-3.0), "-3");
        assert_eq!(number_to_string(0.1), "0.1");
        assert_eq!(number_to_string(1.5), "1.5");
        assert_eq!(number_to_string(1.0e14), "1.0E14");
        assert_eq!(number_to_string(1.0e-4), "1.0E-4");
        assert_eq!(number_to_string(0.001), "0.001");
        assert_eq!(number_to_string(1234567.5), "1234567.5");
        assert_eq!(number_to_string(12345678.5), "1.23456785E7");
        assert_eq!(number_to_string(f64::NAN), "nan");
        assert_eq!(number_to_string(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    fn parses_like_java() {
        assert_eq!(java_parse_double(" 12 "), Some(12.0));
        assert_eq!(java_parse_double("1e3"), Some(1000.0));
        assert_eq!(java_parse_double("2f"), Some(2.0));
        assert_eq!(java_parse_double(".5"), Some(0.5));
        assert_eq!(java_parse_double("5."), Some(5.0));
        assert_eq!(java_parse_double("0x10"), None);
        assert_eq!(java_parse_double("0x1p4"), Some(16.0));
        assert_eq!(java_parse_double("abc"), None);
        assert_eq!(java_parse_double("1e"), None);
        assert_eq!(kahlua_tonumber("ff", 16), Ok(Some(255.0)));
        assert_eq!(kahlua_tonumber("-inf", 10), Ok(Some(f64::NEG_INFINITY)));
    }
}
