//! `java.util.Formatter` (as used by `String.formatted`) and
//! `Double.toString`, for the original's `Translator.getText` formatting.
//!
//! Translations are Java format strings, and the original's fallbacks depend
//! on exactly which `IllegalFormatException` subclass the JDK throws, so the
//! parser and the per-conversion checks follow the JDK 25 `Formatter` (the
//! runtime the game ships) step for step. The default format locale is
//! `Locale.ROOT` (`LocaleManager` sets it), so digits are ASCII, the decimal
//! separator is '.' and the grouping separator ',' with a group size of 3.

/// A Java argument object as it reaches `Translator.getText(String, Object...)`.
/// Kahlua passes Lua numbers as `Double`, strings as `String` and booleans as
/// `Boolean`; Java callers may also pass `Integer`/`Long`.
#[derive(Clone, Debug, PartialEq)]
pub enum JavaArg {
    Null,
    Str(String),
    Double(f64),
    Int(i32),
    Long(i64),
    Bool(bool),
    /// Any other object, described by its `toString()` and `hashCode()`.
    Object {
        to_string: String,
        hash_code: i32,
    },
}

/// The `IllegalFormatException` subclasses the original can observe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    MissingFormatArgument(String),
    UnknownFormatConversion(String),
    IllegalFormatArgumentIndex,
    DuplicateFormatFlags(String),
    UnknownFormatFlags(String),
    IllegalFormatWidth,
    IllegalFormatPrecision,
    MissingFormatWidth(String),
    IllegalFormatFlags(String),
    FormatFlagsConversionMismatch(String, char),
    IllegalFormatConversion(char),
    IllegalFormatCodePoint(i32),
    /// A date/time conversion applied to a `Long`, which needs a calendar;
    /// no translation path reaches it.
    UnsupportedDateTime,
}

impl FormatError {
    /// `e instanceof MissingFormatArgumentException`.
    pub fn is_missing_argument(&self) -> bool {
        matches!(self, FormatError::MissingFormatArgument(_))
    }
}

const LEFT_JUSTIFY: u32 = 1;
const UPPERCASE: u32 = 2;
const ALTERNATE: u32 = 4;
const PLUS: u32 = 8;
const LEADING_SPACE: u32 = 16;
const ZERO_PAD: u32 = 32;
const GROUP: u32 = 64;
const PARENTHESES: u32 = 128;
const PREVIOUS: u32 = 256;

fn contains(flags: u32, f: u32) -> bool {
    flags & f == f
}

fn flags_to_string(n: u32) -> String {
    let mut s = String::new();
    for (bit, c) in [
        (LEFT_JUSTIFY, '-'),
        (UPPERCASE, '^'),
        (ALTERNATE, '#'),
        (PLUS, '+'),
        (LEADING_SPACE, ' '),
        (ZERO_PAD, '0'),
        (GROUP, ','),
        (PARENTHESES, '('),
        (PREVIOUS, '<'),
    ] {
        if contains(n, bit) {
            s.push(c);
        }
    }
    s
}

fn flag_bit(c: u16) -> Option<u32> {
    Some(match c {
        0x2d => LEFT_JUSTIFY,
        0x23 => ALTERNATE,
        0x2b => PLUS,
        0x20 => LEADING_SPACE,
        0x30 => ZERO_PAD,
        0x2c => GROUP,
        0x28 => PARENTHESES,
        0x3c => PREVIOUS,
        _ => return None,
    })
}

fn is_digit(c: u16) -> bool {
    (0x30..=0x39).contains(&c)
}

/// `Formatter.isConversion`.
fn is_conversion(c: u16) -> bool {
    (0x61..=0x7a).contains(&c) || (0x41..=0x5a).contains(&c) || c == 0x25
}

/// `Conversion.isValid`.
fn is_valid_conversion(c: u16) -> bool {
    matches!(
        char::from_u32(c as u32),
        Some(
            'A' | 'B'
                | 'C'
                | 'E'
                | 'G'
                | 'H'
                | 'S'
                | 'X'
                | 'a'
                | 'b'
                | 'c'
                | 'd'
                | 'e'
                | 'f'
                | 'g'
                | 'h'
                | 'n'
                | 'o'
                | 's'
                | 'x'
                | '%'
        )
    )
}

fn is_datetime(c: char) -> bool {
    "HIklMNLQpsSTzZaAbBCdehjmyYrRcDF".contains(c)
}

fn to_lower(c: u16) -> u16 {
    if (0x41..=0x5a).contains(&c) {
        c + 32
    } else {
        c
    }
}

#[derive(Clone, Debug)]
struct Spec {
    /// -2 fixed/text, -1 previous ('<'), 0 ordinary, >0 explicit.
    index: i32,
    flags: u32,
    width: i32,
    precision: i32,
    dt: bool,
    c: char,
}

enum Piece {
    Fixed(usize, usize),
    Spec(Spec),
}

/// `Integer.parseInt(s, begin, end, 10)` over ASCII digits.
fn parse_int(s: &[u16], begin: usize, end: usize) -> Option<i32> {
    if begin >= end {
        return None;
    }
    let mut v: i64 = 0;
    for &c in &s[begin..end] {
        if !is_digit(c) {
            return None;
        }
        v = v * 10 + (c - 0x30) as i64;
        if v > i32::MAX as i64 {
            return None;
        }
    }
    Some(v as i32)
}

impl Spec {
    /// `FormatSpecifier(char)`: the shortcut taken for "%s"-style specs.
    fn simple(c: u16) -> Spec {
        let mut s = Spec {
            index: 0,
            flags: 0,
            width: -1,
            precision: -1,
            dt: false,
            c: c as u8 as char,
        };
        if (0x41..=0x5a).contains(&c) {
            s.flags = UPPERCASE;
            s.c = to_lower(c) as u8 as char;
        }
        if c == 0x25 || c == 0x6e {
            s.index = -2;
        }
        s
    }

    #[allow(clippy::too_many_arguments)]
    fn full(
        s: &[u16],
        start: usize,
        arg_size: usize,
        flag_size: usize,
        width_size: usize,
        prec_size: usize,
        t: u16,
        c2: u16,
    ) -> Result<Spec, FormatError> {
        let mut spec = Spec {
            index: 0,
            flags: 0,
            width: -1,
            precision: -1,
            dt: false,
            c: '\0',
        };
        let a = start + arg_size;
        let f = a + flag_size;
        let w = f + width_size;
        let p = w + prec_size;
        if arg_size > 0 {
            spec.index = match parse_int(s, start, a - 1) {
                Some(i) if i > 0 => i,
                _ => return Err(FormatError::IllegalFormatArgumentIndex),
            };
        }
        if flag_size > 0 {
            let mut flags = 0;
            for &c in &s[a..f] {
                let bit = flag_bit(c).ok_or_else(|| {
                    FormatError::UnknownFormatFlags(String::from_utf16_lossy(&[c]))
                })?;
                if contains(flags, bit) {
                    return Err(FormatError::DuplicateFormatFlags(flags_to_string(bit)));
                }
                flags |= bit;
            }
            spec.flags = flags;
            if contains(flags, PREVIOUS) {
                spec.index = -1;
            }
        }
        if width_size > 0 {
            spec.width = parse_int(s, f, w).ok_or(FormatError::IllegalFormatWidth)?;
        }
        if prec_size > 0 {
            spec.precision = parse_int(s, w + 1, p).ok_or(FormatError::IllegalFormatPrecision)?;
        }
        if t != 0 {
            spec.dt = true;
            if t == 0x54 {
                spec.flags |= UPPERCASE;
            }
        }
        // conversion()
        spec.c = char::from_u32(c2 as u32).unwrap_or('\0');
        if !spec.dt {
            if !is_valid_conversion(c2) {
                return Err(FormatError::UnknownFormatConversion(spec.c.to_string()));
            }
            if (0x41..=0x5a).contains(&c2) {
                spec.flags |= UPPERCASE;
                spec.c = to_lower(c2) as u8 as char;
            }
            if spec.c == '%' || spec.c == 'n' {
                spec.index = -2;
            }
        }
        spec.check()?;
        Ok(spec)
    }

    fn check(&self) -> Result<(), FormatError> {
        if self.dt {
            if self.precision != -1 {
                return Err(FormatError::IllegalFormatPrecision);
            }
            if !is_datetime(self.c) {
                return Err(FormatError::UnknownFormatConversion(format!("t{}", self.c)));
            }
            self.check_bad_flags(252)?;
            return self.check_width_for_left();
        }
        match self.c {
            'b' | 'h' | 's' => {
                if (self.c == 'b' || self.c == 'h') && contains(self.flags, ALTERNATE) {
                    return Err(self.mismatch(ALTERNATE));
                }
                self.check_width_for_left()?;
                self.check_bad_flags(248)
            }
            'c' => {
                if self.precision != -1 {
                    return Err(FormatError::IllegalFormatPrecision);
                }
                self.check_bad_flags(252)?;
                self.check_width_for_left()
            }
            'd' | 'o' | 'x' => {
                self.check_numeric()?;
                if self.precision != -1 {
                    return Err(FormatError::IllegalFormatPrecision);
                }
                self.check_bad_flags(if self.c == 'd' { ALTERNATE } else { GROUP })
            }
            'a' | 'e' | 'f' | 'g' => {
                self.check_numeric()?;
                match self.c {
                    'a' => self.check_bad_flags(192),
                    'e' => self.check_bad_flags(GROUP),
                    'g' => self.check_bad_flags(ALTERNATE),
                    _ => Ok(()),
                }
            }
            '%' => {
                if self.precision != -1 {
                    return Err(FormatError::IllegalFormatPrecision);
                }
                if self.flags != LEFT_JUSTIFY && self.flags != 0 {
                    return Err(FormatError::IllegalFormatFlags(flags_to_string(self.flags)));
                }
                if self.width == -1 && contains(self.flags, LEFT_JUSTIFY) {
                    return Err(FormatError::MissingFormatWidth(self.to_java_string()));
                }
                Ok(())
            }
            'n' => {
                if self.precision != -1 {
                    return Err(FormatError::IllegalFormatPrecision);
                }
                if self.width != -1 {
                    return Err(FormatError::IllegalFormatWidth);
                }
                if self.flags != 0 {
                    return Err(FormatError::IllegalFormatFlags(flags_to_string(self.flags)));
                }
                Ok(())
            }
            c => Err(FormatError::UnknownFormatConversion(c.to_string())),
        }
    }

    fn check_width_for_left(&self) -> Result<(), FormatError> {
        if self.width == -1 && contains(self.flags, LEFT_JUSTIFY) {
            return Err(FormatError::MissingFormatWidth(self.to_java_string()));
        }
        Ok(())
    }

    fn check_numeric(&self) -> Result<(), FormatError> {
        if self.width == -1 && self.flags & (LEFT_JUSTIFY | ZERO_PAD) != 0 {
            return Err(FormatError::MissingFormatWidth(self.to_java_string()));
        }
        if contains(self.flags, PLUS | LEADING_SPACE)
            || contains(self.flags, LEFT_JUSTIFY | ZERO_PAD)
        {
            return Err(FormatError::IllegalFormatFlags(flags_to_string(self.flags)));
        }
        Ok(())
    }

    fn check_bad_flags(&self, bad: u32) -> Result<(), FormatError> {
        if self.flags & bad != 0 {
            return Err(self.mismatch(self.flags & bad));
        }
        Ok(())
    }

    fn mismatch(&self, f: u32) -> FormatError {
        FormatError::FormatFlagsConversionMismatch(flags_to_string(f), self.c)
    }

    /// `FormatSpecifier.toString()`.
    fn to_java_string(&self) -> String {
        let mut s = String::from("%");
        s.push_str(&flags_to_string(self.flags & !UPPERCASE));
        if self.index > 0 {
            s.push_str(&format!("{}$", self.index));
        }
        if self.width != -1 {
            s.push_str(&self.width.to_string());
        }
        if self.precision != -1 {
            s.push_str(&format!(".{}", self.precision));
        }
        let upper = contains(self.flags, UPPERCASE);
        if self.dt {
            s.push(if upper { 'T' } else { 't' });
        }
        s.push(if upper {
            self.c.to_ascii_uppercase()
        } else {
            self.c
        });
        s
    }
}

/// `Formatter.parse`.
fn parse(s: &[u16]) -> Result<Vec<Piece>, FormatError> {
    let mut out = Vec::new();
    let max = s.len();
    let mut i = 0;
    while i < max {
        let Some(n) = s[i..].iter().position(|&c| c == 0x25).map(|p| p + i) else {
            out.push(Piece::Fixed(i, max));
            break;
        };
        if i != n {
            out.push(Piece::Fixed(i, n));
        }
        i = n + 1;
        if i >= max {
            return Err(FormatError::UnknownFormatConversion("%".into()));
        }
        let c = s[i];
        if is_valid_conversion(c) {
            out.push(Piece::Spec(Spec::simple(c)));
            i += 1;
            continue;
        }
        let consumed = parse_spec(s, i, max, &mut out)?;
        if consumed > 0 {
            i += consumed;
            continue;
        }
        return Err(FormatError::UnknownFormatConversion(
            String::from_utf16_lossy(&[c]),
        ));
    }
    Ok(out)
}

/// Advances over chars matching `pred`, leaving the last char read in `c`
/// (the parser's `this.c`).
fn scan(s: &[u16], from: usize, max: usize, c: &mut u16, pred: impl Fn(u16) -> bool) -> usize {
    let mut n = from;
    while n < max {
        *c = s[n];
        if !pred(*c) {
            break;
        }
        n += 1;
    }
    n
}

/// `FormatSpecifierParser.parse`: returns the number of chars consumed, 0 if
/// the text after '%' is not a specifier.
fn parse_spec(
    s: &[u16],
    start: usize,
    max: usize,
    out: &mut Vec<Piece>,
) -> Result<usize, FormatError> {
    let first = s[start];
    let mut c = first;
    let mut off = start;
    // parseArgument
    let mut n = scan(s, off, max, &mut c, is_digit);
    let mut arg_size = 0;
    if n == off || c != 0x24 {
        c = first;
    } else {
        n += 1;
        if n < max {
            c = s[n];
        }
        arg_size = n - off;
        off = n;
    }
    // parseFlag
    let end = scan(s, off, max, &mut c, |c| flag_bit(c).is_some());
    let flag_size = end - off;
    off = end;
    // parseWidth
    let end = scan(s, off, max, &mut c, is_digit);
    let width_size = end - off;
    off = end;
    // parsePrecision
    let mut prec_size = 0;
    if c == 0x2e {
        off += 1;
        let end = scan(s, off, max, &mut c, is_digit);
        if end == off {
            return Ok(0);
        }
        prec_size = end - off + 1;
        off = end;
    }
    let mut t = 0u16;
    let mut c2 = 0u16;
    if (c == 0x74 || c == 0x54) && off + 1 < max {
        let c3 = s[off + 1];
        if is_conversion(c3) {
            t = c;
            c2 = c3;
            off += 2;
        }
    } else if is_conversion(c) {
        c2 = c;
        off += 1;
    } else {
        return Ok(0);
    }
    if arg_size + flag_size + width_size + prec_size + t as usize + c2 as usize != 0 {
        let spec = Spec::full(s, start, arg_size, flag_size, width_size, prec_size, t, c2)?;
        out.push(Piece::Spec(spec));
        return Ok(off - start);
    }
    Ok(0)
}

/// `String.format(Locale.ROOT, fmt, args)`.
pub fn format(fmt: &str, args: &[JavaArg]) -> Result<String, FormatError> {
    let s: Vec<u16> = fmt.encode_utf16().collect();
    let pieces = parse(&s)?;
    let mut out: Vec<u16> = Vec::with_capacity(s.len());
    let mut last: i64 = -1;
    let mut lasto: i64 = -1;
    for piece in &pieces {
        match piece {
            Piece::Fixed(a, b) => out.extend_from_slice(&s[*a..*b]),
            Piece::Spec(spec) => match spec.index {
                -2 => spec.print(&mut out, &JavaArg::Null)?,
                -1 => {
                    if last < 0 || last > args.len() as i64 - 1 {
                        return Err(FormatError::MissingFormatArgument(spec.to_java_string()));
                    }
                    spec.print(&mut out, &args[last as usize])?;
                }
                0 => {
                    lasto += 1;
                    last = lasto;
                    if lasto > args.len() as i64 - 1 {
                        return Err(FormatError::MissingFormatArgument(spec.to_java_string()));
                    }
                    spec.print(&mut out, &args[lasto as usize])?;
                }
                i => {
                    last = i as i64 - 1;
                    if last > args.len() as i64 - 1 {
                        return Err(FormatError::MissingFormatArgument(spec.to_java_string()));
                    }
                    spec.print(&mut out, &args[last as usize])?;
                }
            },
        }
    }
    Ok(String::from_utf16_lossy(&out))
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `String.hashCode()`.
pub fn string_hash_code(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

fn long_hash(v: i64) -> i32 {
    (v ^ ((v as u64) >> 32) as i64) as i32
}

impl JavaArg {
    /// `Object.toString()`.
    pub fn java_to_string(&self) -> String {
        match self {
            JavaArg::Null => "null".into(),
            JavaArg::Str(s) => s.clone(),
            JavaArg::Double(d) => double_to_string(*d),
            JavaArg::Int(i) => i.to_string(),
            JavaArg::Long(l) => l.to_string(),
            JavaArg::Bool(b) => b.to_string(),
            JavaArg::Object { to_string, .. } => to_string.clone(),
        }
    }

    fn hash_code(&self) -> i32 {
        match self {
            JavaArg::Null => 0,
            JavaArg::Str(s) => string_hash_code(s),
            JavaArg::Double(d) => {
                let bits = if d.is_nan() {
                    0x7ff8000000000000
                } else {
                    d.to_bits() as i64
                };
                long_hash(bits)
            }
            JavaArg::Int(i) => *i,
            JavaArg::Long(l) => long_hash(*l),
            JavaArg::Bool(b) => {
                if *b {
                    1231
                } else {
                    1237
                }
            }
            JavaArg::Object { hash_code, .. } => *hash_code,
        }
    }
}

impl Spec {
    fn print(&self, out: &mut Vec<u16>, arg: &JavaArg) -> Result<(), FormatError> {
        if self.dt {
            return match arg {
                JavaArg::Null => {
                    self.print_str(out, "null");
                    Ok(())
                }
                JavaArg::Long(_) => Err(FormatError::UnsupportedDateTime),
                _ => Err(FormatError::IllegalFormatConversion(self.c)),
            };
        }
        match self.c {
            'd' | 'o' | 'x' => match arg {
                JavaArg::Null => self.print_str(out, "null"),
                JavaArg::Int(i) => {
                    let mut l = *i as i64;
                    if *i < 0 && self.c != 'd' {
                        l += 1 << 32;
                    }
                    self.print_long(out, l)?
                }
                JavaArg::Long(l) => self.print_long(out, *l)?,
                _ => return Err(FormatError::IllegalFormatConversion(self.c)),
            },
            'a' | 'e' | 'f' | 'g' => match arg {
                JavaArg::Null => self.print_str(out, "null"),
                JavaArg::Double(d) => self.print_double(out, *d),
                _ => return Err(FormatError::IllegalFormatConversion(self.c)),
            },
            'c' => match arg {
                JavaArg::Null => self.print_str(out, "null"),
                JavaArg::Int(i) => {
                    let ch = u32::try_from(*i)
                        .ok()
                        .filter(|&v| v <= 0x10ffff)
                        .ok_or(FormatError::IllegalFormatCodePoint(*i))?;
                    // Lone surrogates come out as U+FFFD (Rust strings are UTF-8).
                    let s = char::from_u32(ch)
                        .map_or_else(|| "\u{fffd}".to_string(), |c| c.to_string());
                    self.print_str(out, &s);
                }
                _ => return Err(FormatError::IllegalFormatConversion(self.c)),
            },
            'b' => {
                let s = match arg {
                    JavaArg::Null => "false".to_string(),
                    JavaArg::Bool(b) => b.to_string(),
                    _ => "true".to_string(),
                };
                self.print_str(out, &s);
            }
            's' => {
                if contains(self.flags, ALTERNATE) {
                    return Err(self.mismatch(ALTERNATE));
                }
                self.print_str(out, &arg.java_to_string());
            }
            'h' => {
                let s = match arg {
                    JavaArg::Null => "null".to_string(),
                    a => format!("{:x}", a.hash_code() as u32),
                };
                self.print_str(out, &s);
            }
            'n' => out.extend(utf16(if cfg!(windows) { "\r\n" } else { "\n" })),
            '%' => self.print_str(out, "%"),
            _ => unreachable!(),
        }
        Ok(())
    }

    fn print_str(&self, out: &mut Vec<u16>, s: &str) {
        let mut v = utf16(s);
        if self.precision != -1 && (self.precision as usize) < v.len() {
            v.truncate(self.precision as usize);
        }
        if contains(self.flags, UPPERCASE) {
            v = utf16(&String::from_utf16_lossy(&v).to_uppercase());
        }
        self.append_justified(out, &v);
    }

    fn append_justified(&self, out: &mut Vec<u16>, v: &[u16]) {
        if self.width == -1 {
            out.extend_from_slice(v);
            return;
        }
        let left = contains(self.flags, LEFT_JUSTIFY);
        let pad = (self.width as i64 - v.len() as i64).max(0) as usize;
        if left {
            out.extend_from_slice(v);
        }
        out.extend(std::iter::repeat_n(0x20, pad));
        if !left {
            out.extend_from_slice(v);
        }
    }

    fn leading_sign(&self, sb: &mut String, neg: bool) {
        if !neg {
            if contains(self.flags, PLUS) {
                sb.push('+');
            } else if contains(self.flags, LEADING_SPACE) {
                sb.push(' ');
            }
        } else if contains(self.flags, PARENTHESES) {
            sb.push('(');
        } else {
            sb.push('-');
        }
    }

    fn trailing_sign(&self, sb: &mut String, neg: bool) {
        if neg && contains(self.flags, PARENTHESES) {
            sb.push(')');
        }
    }

    fn adjust_width(&self, width: i32, neg: bool) -> i32 {
        if width != -1 && neg && contains(self.flags, PARENTHESES) {
            width - 1
        } else {
            width
        }
    }

    fn print_long(&self, out: &mut Vec<u16>, l: i64) -> Result<(), FormatError> {
        let mut sb = String::new();
        match self.c {
            'd' => {
                let neg = l < 0;
                let digits = l.to_string();
                self.leading_sign(&mut sb, neg);
                let w = self.adjust_width(self.width, neg);
                localized_magnitude(&mut sb, &digits[neg as usize..], self.flags, w);
                self.trailing_sign(&mut sb, neg);
            }
            'o' | 'x' => {
                self.check_bad_flags(PLUS | LEADING_SPACE | PARENTHESES)?;
                let mut digits = if self.c == 'o' {
                    format!("{:o}", l)
                } else {
                    format!("{:x}", l)
                };
                let prefix = match (self.c, contains(self.flags, ALTERNATE)) {
                    ('o', true) => "0",
                    ('x', true) if contains(self.flags, UPPERCASE) => "0X",
                    ('x', true) => "0x",
                    _ => "",
                };
                sb.push_str(prefix);
                if contains(self.flags, ZERO_PAD) {
                    let len = (digits.len() + prefix.len()) as i32;
                    trailing_zeros(&mut sb, self.width - len);
                }
                if self.c == 'x' && contains(self.flags, UPPERCASE) {
                    digits = digits.to_uppercase();
                }
                sb.push_str(&digits);
            }
            _ => unreachable!(),
        }
        self.append_justified(out, &utf16(&sb));
        Ok(())
    }

    fn print_double(&self, out: &mut Vec<u16>, d: f64) {
        let mut sb = String::new();
        if d.is_nan() {
            sb.push_str(if contains(self.flags, UPPERCASE) {
                "NAN"
            } else {
                "NaN"
            });
        } else {
            // Double.compare(d, 0.0) == -1 counts -0.0 as negative.
            let neg = d < 0.0 || (d == 0.0 && d.is_sign_negative());
            let v = d.abs();
            self.leading_sign(&mut sb, neg);
            if v.is_infinite() {
                sb.push_str(if contains(self.flags, UPPERCASE) {
                    "INFINITY"
                } else {
                    "Infinity"
                });
            } else {
                self.print_finite(&mut sb, v, neg);
            }
            self.trailing_sign(&mut sb, neg);
        }
        self.append_justified(out, &utf16(&sb));
    }

    fn print_finite(&self, sb: &mut String, d: f64, neg: bool) {
        let upper = contains(self.flags, UPPERCASE);
        match self.c {
            'e' => {
                let prec = if self.precision == -1 {
                    6
                } else {
                    self.precision
                };
                let fd = FpDecimal::split(d).scientific(prec);
                let mut mant = fd.digits.clone();
                add_zeros(&mut mant, prec);
                if contains(self.flags, ALTERNATE) && prec == 0 {
                    mant.push('.');
                }
                let exp = if d == 0.0 {
                    "+00".to_string()
                } else {
                    fd.exp.clone()
                };
                let w = if self.width != -1 {
                    self.adjust_width(self.width - exp.len() as i32 - 1, neg)
                } else {
                    -1
                };
                localized_magnitude(sb, &mant, self.flags, w);
                sb.push(if upper { 'E' } else { 'e' });
                sb.push_str(&exp);
            }
            'f' => {
                let prec = if self.precision == -1 {
                    6
                } else {
                    self.precision
                };
                let fd = FpDecimal::split(d).plain(prec);
                let mut mant = fd.digits.clone();
                add_zeros(&mut mant, prec);
                if contains(self.flags, ALTERNATE) && prec == 0 {
                    mant.push('.');
                }
                let w = if self.width != -1 {
                    self.adjust_width(self.width, neg)
                } else {
                    -1
                };
                localized_magnitude(sb, &mant, self.flags, w);
            }
            'g' => {
                let mut prec = match self.precision {
                    -1 => 6,
                    0 => 1,
                    p => p,
                };
                let (mut mant, exp, exp_rounded) = if d == 0.0 {
                    ("0".to_string(), None, 0)
                } else {
                    let fd = FpDecimal::split(d).general(prec);
                    let er = fd.exponent_rounded();
                    (fd.digits.clone(), fd.exp_used.then(|| fd.exp.clone()), er)
                };
                prec = if exp.is_some() {
                    prec - 1
                } else {
                    prec - (exp_rounded + 1)
                };
                add_zeros(&mut mant, prec);
                if contains(self.flags, ALTERNATE) && prec == 0 {
                    mant.push('.');
                }
                let w = if self.width != -1 {
                    match &exp {
                        Some(e) => self.adjust_width(self.width - e.len() as i32 - 1, neg),
                        None => self.adjust_width(self.width, neg),
                    }
                } else {
                    -1
                };
                localized_magnitude(sb, &mant, self.flags, w);
                if let Some(e) = exp {
                    sb.push(if upper { 'E' } else { 'e' });
                    sb.push_str(&e);
                }
            }
            'a' => {
                let prec = match self.precision {
                    -1 => 0,
                    0 => 1,
                    p => p,
                };
                let s = hex_double(d, prec);
                sb.push_str(if upper { "0X" } else { "0x" });
                if contains(self.flags, ZERO_PAD) {
                    let mut n = 2;
                    if contains(self.flags, LEADING_SPACE) || contains(self.flags, PLUS) || neg {
                        n = 3;
                    }
                    trailing_zeros(sb, self.width - s.len() as i32 - n);
                }
                let p = s.find('p').unwrap();
                let mut mant = if upper {
                    s[..p].to_uppercase()
                } else {
                    s[..p].to_string()
                };
                if prec != 0 {
                    add_zeros(&mut mant, prec);
                }
                sb.push_str(&mant);
                sb.push(if upper { 'P' } else { 'p' });
                sb.push_str(&s[p + 1..]);
            }
            _ => unreachable!(),
        }
    }
}

fn trailing_zeros(sb: &mut String, n: i32) {
    for _ in 0..n.max(0) {
        sb.push('0');
    }
}

/// `FormatSpecifier.addZeros`: pad the fraction to `prec` digits.
fn add_zeros(sb: &mut String, prec: i32) {
    let len = sb.len();
    let dot = sb.find('.');
    let have = match dot {
        Some(i) => (len - i - 1) as i32,
        None => 0,
    };
    if have == prec {
        return;
    }
    if dot.is_none() {
        sb.push('.');
    }
    trailing_zeros(sb, prec - have);
}

/// `FormatSpecifier.localizedMagnitude` for `Locale.ROOT`.
fn localized_magnitude(sb: &mut String, value: &str, flags: u32, width: i32) {
    let begin = sb.encode_utf16().count();
    let start = sb.len();
    let chars: Vec<char> = value.chars().collect();
    let dot = chars.iter().position(|&c| c == '.').unwrap_or(chars.len());
    let mut grouping = contains(flags, GROUP);
    for (j, &c) in chars.iter().enumerate() {
        if j == dot {
            sb.push('.');
            grouping = false;
            continue;
        }
        sb.push(c);
        if grouping && j != dot - 1 && (dot - j) % 3 == 1 {
            sb.push(',');
        }
    }
    let len = sb.encode_utf16().count();
    if width as i64 > len as i64 && contains(flags, ZERO_PAD) {
        let pad = "0".repeat(width as usize - len);
        let _ = begin;
        sb.insert_str(start, &pad);
    }
}

/// Decimal `f × 10^e` with `n` significant digits (`FormattedFPDecimal`).
#[derive(Clone, Debug)]
struct FpDecimal {
    f: u64,
    e: i32,
    n: i32,
    digits: String,
    exp: String,
    exp_used: bool,
}

fn pow10(n: i32) -> u64 {
    10u64.pow(n as u32)
}

impl FpDecimal {
    /// `DoubleToDecimal.split`: the shortest decimal that rounds back to `d`.
    fn split(d: f64) -> FpDecimal {
        if d == 0.0 {
            return FpDecimal {
                f: 0,
                e: 0,
                n: 1,
                digits: String::new(),
                exp: String::new(),
                exp_used: false,
            };
        }
        // Rust's `{:e}` is the shortest round-trip representation, which is
        // what Java's Schubfach-based `DoubleToDecimal` also produces.
        let s = format!("{:e}", d);
        let (m, e) = s.split_once('e').unwrap();
        let digits: String = m.chars().filter(|c| c.is_ascii_digit()).collect();
        let n = digits.len() as i32;
        let f: u64 = digits.parse().unwrap();
        let exp10: i32 = e.parse().unwrap();
        FpDecimal {
            f,
            e: exp10 - (n - 1),
            n,
            digits: String::new(),
            exp: String::new(),
            exp_used: false,
        }
    }

    fn exponent_rounded(&self) -> i32 {
        self.n + self.e - 1
    }

    fn round(&mut self, prec: i64) {
        if self.n == 0 || prec < 0 {
            self.f = 0;
            self.e = 0;
            self.n = 1;
            return;
        }
        if prec >= self.n as i64 {
            return;
        }
        let p = prec as i32;
        self.e += self.n - p;
        let l2 = pow10(self.n - p);
        self.f = (self.f + (l2 >> 1)) / l2;
        if p == 0 {
            self.n = 1;
            if self.f == 0 {
                self.e = 0;
            }
            return;
        }
        self.n = p;
        if self.f == pow10(p) {
            self.f /= 10;
            self.e += 1;
        }
    }

    fn plain(mut self, prec: i32) -> FpDecimal {
        self.round((self.n + self.e) as i64 + prec as i64);
        self.plain_chars();
        self
    }

    fn plain_chars(&mut self) {
        let fs = format!("{:0width$}", self.f, width = self.n as usize);
        self.digits = if self.e >= 0 {
            format!("{}{}", fs, "0".repeat(self.e as usize))
        } else if self.n + self.e > 0 {
            let k = (self.n + self.e) as usize;
            format!("{}.{}", &fs[..k], &fs[k..])
        } else {
            format!("0.{}{}", "0".repeat((-self.e - self.n) as usize), fs)
        };
    }

    fn scientific(mut self, prec: i32) -> FpDecimal {
        self.round(prec as i64 + 1);
        self.scientific_chars(prec);
        self
    }

    fn scientific_chars(&mut self, prec: i32) {
        let fs = format!("{:0width$}", self.f, width = self.n as usize);
        self.digits = if prec != 0 {
            format!("{}.{}", &fs[..1], &fs[1..])
        } else {
            fs[..1].to_string()
        };
        let er = self.exponent_rounded();
        let a = er.abs();
        self.exp = format!("{}{:02}", if er >= 0 { '+' } else { '-' }, a);
        self.exp_used = true;
    }

    fn general(mut self, prec: i32) -> FpDecimal {
        self.round(prec as i64);
        let er = self.exponent_rounded();
        if (-4..prec).contains(&er) {
            self.plain_chars();
        } else {
            self.scientific_chars(prec - 1);
        }
        self
    }
}

/// `Double.toString(d)` (JDK 19+ shortest-digits algorithm).
pub fn double_to_string(d: f64) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if d == 0.0 {
        return if d.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let sign = if d < 0.0 { "-" } else { "" };
    let fd = FpDecimal::split(d.abs());
    let digits = fd.f.to_string();
    let exp = fd.exponent_rounded();
    if (-3..7).contains(&exp) {
        let s = if exp >= 0 {
            let k = exp as usize + 1;
            if digits.len() <= k {
                format!("{}{}.0", digits, "0".repeat(k - digits.len()))
            } else {
                format!("{}.{}", &digits[..k], &digits[k..])
            }
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        };
        format!("{sign}{s}")
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{}", &digits[..1], frac, exp)
    }
}

/// `Double.toHexString(d)`.
pub fn double_to_hex_string(d: f64) -> String {
    if !d.is_finite() {
        return double_to_string(d);
    }
    let sign = if d.is_sign_negative() { "-" } else { "" };
    if d == 0.0 {
        return format!("{sign}0x0.0p0");
    }
    let bits = d.to_bits();
    let subnormal = (bits >> 52) & 0x7ff == 0;
    let mant = bits & 0x000f_ffff_ffff_ffff;
    let mut hex = format!("{:013x}", mant);
    // Strip trailing zeros, keeping at least one digit.
    while hex.len() > 1 && hex.ends_with('0') {
        hex.pop();
    }
    if subnormal {
        format!("{sign}0x0.{hex}p-1022")
    } else {
        let exp = ((bits >> 52) & 0x7ff) as i32 - 1023;
        format!("{sign}0x1.{hex}p{exp}")
    }
}

/// `FormatSpecifier.hexDouble`.
fn hex_double(d: f64, prec: i32) -> String {
    if !d.is_finite() || d == 0.0 || prec == 0 || prec >= 13 {
        return double_to_hex_string(d)[2..].to_string();
    }
    let mut d = d;
    let mut exponent = ((d.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    let subnormal = exponent == -1023;
    if subnormal {
        d *= 2f64.powi(54);
        exponent = ((d.to_bits() >> 52) & 0x7ff) as i32 - 1023;
        let _ = exponent;
    }
    let precision = 1 + prec * 4;
    let shift = 53 - precision;
    let bits = d.to_bits() as i64;
    let mut new_sig = (bits & i64::MAX) >> shift;
    let rounding_bits = bits & !(-1i64 << shift);
    let least_zero = new_sig & 1 == 0;
    let round = (1i64 << (shift - 1)) & rounding_bits != 0;
    let sticky = shift > 1 && !(1i64 << (shift - 1)) & rounding_bits != 0;
    if (least_zero && round && sticky) || (!least_zero && round) {
        new_sig += 1;
    }
    let sign_bit = bits & i64::MIN;
    new_sig = sign_bit | (new_sig << shift);
    let result = f64::from_bits(new_sig as u64);
    if result.is_infinite() {
        return "1.0p1024".into();
    }
    let res = double_to_hex_string(result)[2..].to_string();
    if !subnormal {
        return res;
    }
    let p = res.find('p').unwrap();
    let e: i32 = res[p + 1..].parse().unwrap();
    format!("{}p{}", &res[..p], e - 54)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> JavaArg {
        JavaArg::Str(v.into())
    }

    #[test]
    fn positional_and_escapes() {
        assert_eq!(
            format("%1$s and %2$s", &[s("a"), s("b")]).unwrap(),
            "a and b"
        );
        assert_eq!(format("100%%", &[]).unwrap(), "100%");
        assert_eq!(format("%s %<s", &[s("x")]).unwrap(), "x x");
    }

    #[test]
    fn error_kinds() {
        assert!(format("%1$s", &[]).unwrap_err().is_missing_argument());
        assert_eq!(
            format("5% luck", &[]),
            Err(FormatError::UnknownFormatConversion("l".into()))
        );
        assert!(format("50% off", &[]).unwrap_err().is_missing_argument());
        assert_eq!(
            format("a %", &[]),
            Err(FormatError::UnknownFormatConversion("%".into()))
        );
        assert_eq!(
            format("% %", &[]),
            Err(FormatError::IllegalFormatFlags(" ".into()))
        );
        assert_eq!(
            format("%d", &[s("1")]),
            Err(FormatError::IllegalFormatConversion('d'))
        );
        assert!(format("% d", &[]).unwrap_err().is_missing_argument());
        assert_eq!(
            format("%0$s", &[]),
            Err(FormatError::IllegalFormatArgumentIndex)
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(format("%.1f", &[JavaArg::Double(0.25)]).unwrap(), "0.3");
        assert_eq!(format("%.1f", &[JavaArg::Double(0.15)]).unwrap(), "0.2");
        assert_eq!(
            format("%5.2f|", &[JavaArg::Double(-1.005)]).unwrap(),
            "-1.01|"
        );
        assert_eq!(
            format("%,d", &[JavaArg::Long(1234567)]).unwrap(),
            "1,234,567"
        );
        assert_eq!(
            format("%08.3f", &[JavaArg::Double(-3.5)]).unwrap(),
            "-003.500"
        );
        assert_eq!(
            format("%e", &[JavaArg::Double(12345.678)]).unwrap(),
            "1.234568e+04"
        );
        assert_eq!(
            format("%g", &[JavaArg::Double(0.0001)]).unwrap(),
            "0.000100000"
        );
        assert_eq!(format("%x", &[JavaArg::Int(-1)]).unwrap(), "ffffffff");
        assert_eq!(format("%a", &[JavaArg::Double(1.0)]).unwrap(), "0x1.0p0");
    }

    #[test]
    fn double_to_string_matches_java() {
        assert_eq!(double_to_string(1.0), "1.0");
        assert_eq!(double_to_string(0.1), "0.1");
        assert_eq!(double_to_string(1e7), "1.0E7");
        assert_eq!(double_to_string(1234567.0), "1234567.0");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(0.0001), "1.0E-4");
        assert_eq!(double_to_string(-2.5e-10), "-2.5E-10");
        assert_eq!(double_to_string(1e23), "1.0E23");
    }
}
