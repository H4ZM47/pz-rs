//! The subset of `org.json` (`JSONTokener` / `JSONObject`, the version the
//! game bundles) that translation and `language.json` loading go through.
//!
//! org.json is lenient by default: unquoted and single-quoted strings, ';'
//! as a separator and trailing commas are accepted, duplicate keys are an
//! error. Strict mode (used by the original only when `Core.IS_DEV`) rejects
//! the lenient forms. Text is handled as UTF-16 code units like Java.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    String(String),
    /// A number literal, kept as written (`JSONObject.stringToValue`).
    Number(String),
    Bool(bool),
    Null,
    Object(JsonObject),
    Array(Vec<JsonValue>),
}

/// `JSONObject` (a `HashMap` in the original: key order is not observable).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JsonObject {
    pub map: HashMap<String, JsonValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError(pub String);

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

struct Tokener {
    s: Vec<u16>,
    pos: usize,
    eof: bool,
    use_previous: bool,
    previous: u16,
    index: i64,
    character: i64,
    character_previous_line: i64,
    line: i64,
    strict: bool,
}

const DELIMS: &str = ",:]}/\\\"[{;=#";

impl Tokener {
    fn new(src: &str, strict: bool) -> Self {
        Tokener {
            s: src.encode_utf16().collect(),
            pos: 0,
            eof: false,
            use_previous: false,
            previous: 0,
            index: 0,
            character: 1,
            character_previous_line: 0,
            line: 1,
            strict,
        }
    }

    fn syntax_error(&self, msg: &str) -> JsonError {
        JsonError(format!(
            "{msg} at {} [character {} line {}]",
            self.index, self.character, self.line
        ))
    }

    fn back(&mut self) -> Result<(), JsonError> {
        if self.use_previous || self.index <= 0 {
            return Err(JsonError("Stepping back two steps is not supported".into()));
        }
        self.index -= 1;
        if self.previous == 0x0d || self.previous == 0x0a {
            self.line -= 1;
            self.character = self.character_previous_line;
        } else if self.character > 0 {
            self.character -= 1;
        }
        self.use_previous = true;
        self.eof = false;
        Ok(())
    }

    fn end(&self) -> bool {
        self.eof && !self.use_previous
    }

    fn next(&mut self) -> u16 {
        let c: u16 = if self.use_previous {
            self.use_previous = false;
            self.previous
        } else if self.pos < self.s.len() {
            self.pos += 1;
            self.s[self.pos - 1]
        } else {
            0
        };
        // Reader.read() returning <= 0 (end of input, or a NUL char) is EOF.
        if c == 0 {
            self.eof = true;
            return 0;
        }
        self.index += 1;
        if c == 0x0d {
            self.line += 1;
            self.character_previous_line = self.character;
            self.character = 0;
        } else if c == 0x0a {
            if self.previous != 0x0d {
                self.line += 1;
                self.character_previous_line = self.character;
            }
            self.character = 0;
        } else {
            self.character += 1;
        }
        self.previous = c;
        c
    }

    fn next_n(&mut self, n: usize) -> Result<Vec<u16>, JsonError> {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.next());
            if self.end() {
                return Err(self.syntax_error("Substring bounds error"));
            }
        }
        Ok(v)
    }

    fn next_clean(&mut self) -> u16 {
        loop {
            let c = self.next();
            if c == 0 || c > 0x20 {
                return c;
            }
        }
    }

    fn next_string(&mut self, quote: u16) -> Result<String, JsonError> {
        let mut sb: Vec<u16> = Vec::new();
        loop {
            let c = self.next();
            match c {
                0 | 0x0a | 0x0d => {
                    return Err(self.syntax_error(&format!(
                        "Unterminated string. Character with int code {c} is not allowed within a quoted string."
                    )))
                }
                0x5c => {
                    let e = self.next();
                    match e {
                        0x62 => sb.push(0x08),
                        0x74 => sb.push(0x09),
                        0x6e => sb.push(0x0a),
                        0x66 => sb.push(0x0c),
                        0x72 => sb.push(0x0d),
                        0x75 => {
                            let hex = self.next_n(4)?;
                            let hs = String::from_utf16_lossy(&hex);
                            // Integer.parseInt(next, 16) also accepts a sign.
                            match i32::from_str_radix(&hs, 16) {
                                Ok(v) => sb.push(v as u16),
                                Err(_) => {
                                    return Err(self.syntax_error(&format!(
                                        "Illegal escape. \\u must be followed by a 4 digit hexadecimal number. \\{hs} is not valid."
                                    )))
                                }
                            }
                        }
                        0x22 | 0x27 | 0x2f | 0x5c => sb.push(e),
                        _ => {
                            return Err(self.syntax_error(&format!(
                                "Illegal escape. Escape sequence  \\{} is not valid.",
                                String::from_utf16_lossy(&[e])
                            )))
                        }
                    }
                }
                _ if c == quote => return Ok(String::from_utf16_lossy(&sb)),
                _ => sb.push(c),
            }
        }
    }

    fn next_value(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        let c = self.next_clean();
        match c {
            0x7b => {
                self.back()?;
                Ok(JsonValue::Object(self.object(depth + 1)?))
            }
            0x5b => {
                self.back()?;
                Ok(JsonValue::Array(self.array(depth + 1)?))
            }
            _ => self.next_simple_value(c),
        }
    }

    fn next_simple_value(&mut self, mut c: u16) -> Result<JsonValue, JsonError> {
        if self.strict && c == 0x27 {
            return Err(
                self.syntax_error("Strict mode error: Single quoted strings are not allowed")
            );
        }
        if c == 0x22 || c == 0x27 {
            return Ok(JsonValue::String(self.next_string(c)?));
        }
        let mut sb: Vec<u16> = Vec::new();
        while c >= 0x20 && !DELIMS.encode_utf16().any(|d| d == c) {
            sb.push(c);
            c = self.next();
        }
        if !self.eof {
            self.back()?;
        }
        let string = java_trim(&String::from_utf16_lossy(&sb));
        if string.is_empty() {
            return Err(self.syntax_error("Missing value"));
        }
        let v = string_to_value(&string);
        if self.strict && matches!(v, JsonValue::String(_)) {
            return Err(self.syntax_error(&format!(
                "Strict mode error: Value '{string}' is not surrounded by quotes"
            )));
        }
        Ok(v)
    }

    fn object(&mut self, depth: usize) -> Result<JsonObject, JsonError> {
        let is_initial = self.previous == 0;
        let mut obj = JsonObject::default();
        if self.next_clean() != 0x7b {
            return Err(self.syntax_error("A JSONObject text must begin with '{'"));
        }
        loop {
            let c = self.next_clean();
            match c {
                0 => return Err(self.syntax_error("A JSONObject text must end with '}'")),
                0x7d => return self.finish_object(obj, is_initial),
                _ => {}
            }
            let key = self.next_simple_value(c)?.java_to_string();
            if self.next_clean() != 0x3a {
                return Err(self.syntax_error("Expected a ':' after a key"));
            }
            if obj.map.contains_key(&key) {
                return Err(self.syntax_error(&format!("Duplicate key \"{key}\"")));
            }
            let value = self.next_value(depth)?;
            obj.map.insert(key, value);
            match self.next_clean() {
                0x3b | 0x2c => {
                    if self.previous == 0x3b && self.strict {
                        return Err(
                            self.syntax_error("Strict mode error: Invalid character ';' found")
                        );
                    }
                    if self.next_clean() == 0x7d {
                        if self.strict {
                            return Err(self.syntax_error(
                                "Strict mode error: Expected another object element",
                            ));
                        }
                        return Ok(obj);
                    }
                    if self.end() {
                        return Err(self.syntax_error("A JSONObject text must end with '}'"));
                    }
                    self.back()?;
                }
                0x7d => return self.finish_object(obj, is_initial),
                _ => return Err(self.syntax_error("Expected a ',' or '}'")),
            }
        }
    }

    fn finish_object(
        &mut self,
        obj: JsonObject,
        is_initial: bool,
    ) -> Result<JsonObject, JsonError> {
        if is_initial && self.strict && self.next_clean() != 0 {
            return Err(self.syntax_error(
                "Strict mode error: Unparsed characters found at end of input text",
            ));
        }
        Ok(obj)
    }

    fn array(&mut self, depth: usize) -> Result<Vec<JsonValue>, JsonError> {
        let mut out = Vec::new();
        if self.next_clean() != 0x5b {
            return Err(self.syntax_error("A JSONArray text must start with '['"));
        }
        let mut c = self.next_clean();
        if c == 0 {
            return Err(self.syntax_error("Expected a ',' or ']'"));
        }
        if c == 0x5d {
            return Ok(out);
        }
        self.back()?;
        loop {
            if self.next_clean() == 0x2c {
                if self.strict {
                    return Err(
                        self.syntax_error("Strict mode error: Expected a valid array element")
                    );
                }
                self.back()?;
                out.push(JsonValue::Null);
            } else {
                self.back()?;
                out.push(self.next_value(depth)?);
            }
            c = self.next_clean();
            match c {
                0 => return Err(self.syntax_error("Expected a ',' or ']'")),
                0x2c => {
                    c = self.next_clean();
                    if c == 0 {
                        return Err(self.syntax_error("Expected a ',' or ']'"));
                    }
                    if c == 0x5d {
                        if self.strict {
                            return Err(self.syntax_error(
                                "Strict mode error: Expected another array element",
                            ));
                        }
                        return Ok(out);
                    }
                    self.back()?;
                }
                0x5d => return Ok(out),
                _ => return Err(self.syntax_error("Expected a ',' or ']'")),
            }
        }
    }
}

/// `String.trim()`: strips chars <= ' ' from both ends.
pub(crate) fn java_trim(s: &str) -> String {
    s.trim_matches(|c: char| c <= ' ').to_string()
}

/// `JSONObject.stringToValue`.
fn string_to_value(s: &str) -> JsonValue {
    if s.eq_ignore_ascii_case("true") {
        return JsonValue::Bool(true);
    }
    if s.eq_ignore_ascii_case("false") {
        return JsonValue::Bool(false);
    }
    if s.eq_ignore_ascii_case("null") {
        return JsonValue::Null;
    }
    let initial = s.as_bytes()[0];
    if (initial.is_ascii_digit() || initial == b'-') && is_java_number(s) {
        return JsonValue::Number(s.to_string());
    }
    JsonValue::String(s.to_string())
}

/// Whether `JSONObject.stringToNumber` accepts `s` (decimal via `BigDecimal`,
/// otherwise an integer via `BigInteger`).
fn is_java_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'-') {
        i = 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - digits_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - fs;
    }
    if int_digits + frac_digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let es = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == es {
            return false;
        }
    }
    i == b.len()
}

impl JsonValue {
    /// `Object.toString()` of the parsed value.
    pub fn java_to_string(&self) -> String {
        match self {
            JsonValue::String(s) | JsonValue::Number(s) => s.clone(),
            JsonValue::Bool(b) => b.to_string(),
            JsonValue::Null => "null".into(),
            JsonValue::Object(_) => "{...}".into(),
            JsonValue::Array(_) => "[...]".into(),
        }
    }
}

/// `new JSONObject(source, new JSONParserConfiguration().withStrictMode(strict))`.
pub fn parse_object(src: &str, strict: bool) -> Result<JsonObject, JsonError> {
    let mut t = Tokener::new(src, strict);
    t.object(0)
}

impl JsonObject {
    /// `optString(key, default)`.
    pub fn opt_string(&self, key: &str) -> Option<String> {
        match self.map.get(key) {
            None | Some(JsonValue::Null) => None,
            Some(v) => Some(v.java_to_string()),
        }
    }

    /// `optInt(key, default)`.
    pub fn opt_int(&self, key: &str, default: i32) -> i32 {
        match self.map.get(key) {
            None | Some(JsonValue::Null) => default,
            Some(JsonValue::Number(s)) => number_int_value(s).unwrap_or(default),
            Some(v) => {
                let s = v.java_to_string();
                let first = s.as_bytes().first().copied().unwrap_or(0);
                if (first.is_ascii_digit() || first == b'-') && is_java_number(&s) {
                    number_int_value(&s).unwrap_or(default)
                } else {
                    default
                }
            }
        }
    }

    /// `optBoolean(key, default)`: booleans, or the strings "true"/"false"
    /// in any case.
    pub fn opt_boolean(&self, key: &str, default: bool) -> bool {
        match self.map.get(key) {
            None | Some(JsonValue::Null) => default,
            Some(JsonValue::Bool(b)) => *b,
            Some(JsonValue::String(s)) if s.eq_ignore_ascii_case("true") => true,
            Some(JsonValue::String(s)) if s.eq_ignore_ascii_case("false") => false,
            Some(_) => default,
        }
    }
}

/// `Number.intValue()` of a parsed number literal.
fn number_int_value(s: &str) -> Option<i32> {
    if s.contains(['.', 'e', 'E']) {
        let d: f64 = s.parse().ok()?;
        // BigDecimal.intValue truncates toward zero; out-of-range values
        // keep the low 32 bits, which no language.json relies on.
        return Some(d.trunc() as i64 as i32);
    }
    let v: i128 = s.parse().ok()?;
    Some(v as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_forms() {
        let o = parse_object("{a: 'x', \"b\": \"y\";c:1,}", false).unwrap();
        assert_eq!(o.map["a"], JsonValue::String("x".into()));
        assert_eq!(o.map["b"], JsonValue::String("y".into()));
        assert_eq!(o.map["c"], JsonValue::Number("1".into()));
        assert!(parse_object("{a: 'x'}", true).is_err());
    }

    #[test]
    fn duplicate_key_is_an_error() {
        assert!(parse_object("{\"a\":\"1\",\"a\":\"2\"}", false).is_err());
    }

    #[test]
    fn escapes_and_options() {
        let o = parse_object(r#"{"k":"aA\n","version":"1","azerty":"TRUE"}"#, false).unwrap();
        assert_eq!(o.map["k"], JsonValue::String("aA\n".into()));
        assert_eq!(o.opt_int("version", -1), 1);
        assert!(o.opt_boolean("azerty", false));
    }

    #[test]
    fn bom_is_rejected() {
        assert!(parse_object("\u{feff}{}", false).is_err());
    }
}
