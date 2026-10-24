//! FBX tokens and the element tree (`FBXTokenizer`, `FBXBinaryTokenizer`,
//! `FBXParser` of assimp 5.3): ASCII and binary files (7.x, 64-bit
//! offsets from 7500) become the same `Element`/`Scope` tree.

use std::collections::BTreeMap;

use crate::import::{fast_atoreal, strtoul10_64, ImportError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenType {
    OpenBracket,
    CloseBracket,
    Data,
    Comma,
    Key,
}

#[derive(Clone, Copy, Debug)]
pub struct Token<'a> {
    pub bytes: &'a [u8],
    pub ty: TokenType,
    /// Line (ASCII) or byte offset (binary).
    pub line: usize,
    pub binary: bool,
}

impl<'a> Token<'a> {
    pub fn string_contents(&self) -> String {
        String::from_utf8_lossy(self.bytes).into_owned()
    }
}

pub fn err<T>(msg: impl Into<String>) -> Result<T, ImportError> {
    Err(ImportError(format!("FBX: {}", msg.into())))
}

// ---------------------------------------------------------------------------
// ASCII tokenizer

fn is_line_end(c: u8) -> bool {
    c == b'\r' || c == b'\n' || c == 0 || c == 0x0c
}

fn is_space_or_newline(c: u8) -> bool {
    c == b' ' || c == b'\t' || is_line_end(c)
}

pub fn tokenize_ascii(input: &[u8]) -> Result<Vec<Token<'_>>, ImportError> {
    let mut out: Vec<Token> = Vec::new();
    let mut line = 1usize;
    let mut comment = false;
    let mut in_quotes = false;
    let mut pending = false;
    let mut begin: Option<usize> = None;
    let mut end: Option<usize> = None;

    fn process<'a>(
        input: &'a [u8],
        out: &mut Vec<Token<'a>>,
        begin: &mut Option<usize>,
        end: &mut Option<usize>,
        line: usize,
        ty: TokenType,
        must: bool,
    ) -> Result<(), ImportError> {
        if let (Some(b), Some(e)) = (*begin, *end) {
            let mut q = false;
            for &c in &input[b..=e] {
                if c == b'"' {
                    q = !q;
                }
                if !q && is_space_or_newline(c) {
                    return err(format!("unexpected whitespace in token (line {line})"));
                }
            }
            if q {
                return err(format!("non-terminated double quotes (line {line})"));
            }
            out.push(Token {
                bytes: &input[b..e + 1],
                ty,
                line,
                binary: false,
            });
        } else if must {
            return err(format!(
                "unexpected character, expected data token (line {line})"
            ));
        }
        *begin = None;
        *end = None;
        Ok(())
    }

    let mut cur = 0usize;
    while cur < input.len() && input[cur] != 0 {
        let c = input[cur];
        if is_line_end(c) {
            comment = false;
            line += 1;
        }
        if comment {
            cur += 1;
            continue;
        }
        if in_quotes {
            if c == b'"' {
                in_quotes = false;
                end = Some(cur);
                process(
                    input,
                    &mut out,
                    &mut begin,
                    &mut end,
                    line,
                    TokenType::Data,
                    false,
                )?;
                pending = false;
            }
            cur += 1;
            continue;
        }
        match c {
            b'"' => {
                if begin.is_some() {
                    return err(format!("unexpected double-quote (line {line})"));
                }
                begin = Some(cur);
                in_quotes = true;
                cur += 1;
                continue;
            }
            b';' => {
                process(
                    input,
                    &mut out,
                    &mut begin,
                    &mut end,
                    line,
                    TokenType::Data,
                    false,
                )?;
                comment = true;
                cur += 1;
                continue;
            }
            b'{' => {
                process(
                    input,
                    &mut out,
                    &mut begin,
                    &mut end,
                    line,
                    TokenType::Data,
                    false,
                )?;
                out.push(Token {
                    bytes: &input[cur..cur + 1],
                    ty: TokenType::OpenBracket,
                    line,
                    binary: false,
                });
                cur += 1;
                continue;
            }
            b'}' => {
                process(
                    input,
                    &mut out,
                    &mut begin,
                    &mut end,
                    line,
                    TokenType::Data,
                    false,
                )?;
                out.push(Token {
                    bytes: &input[cur..cur + 1],
                    ty: TokenType::CloseBracket,
                    line,
                    binary: false,
                });
                cur += 1;
                continue;
            }
            b',' => {
                if pending {
                    process(
                        input,
                        &mut out,
                        &mut begin,
                        &mut end,
                        line,
                        TokenType::Data,
                        true,
                    )?;
                }
                out.push(Token {
                    bytes: &input[cur..cur + 1],
                    ty: TokenType::Comma,
                    line,
                    binary: false,
                });
                cur += 1;
                continue;
            }
            b':' => {
                if pending {
                    process(
                        input,
                        &mut out,
                        &mut begin,
                        &mut end,
                        line,
                        TokenType::Key,
                        true,
                    )?;
                } else {
                    return err(format!("unexpected colon (line {line})"));
                }
                cur += 1;
                continue;
            }
            _ => {}
        }
        if is_space_or_newline(c) {
            if begin.is_some() {
                process(
                    input,
                    &mut out,
                    &mut begin,
                    &mut end,
                    line,
                    TokenType::Data,
                    false,
                )?;
            }
            pending = false;
        } else {
            end = Some(cur);
            if begin.is_none() {
                begin = Some(cur);
            }
            pending = true;
        }
        cur += 1;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Binary tokenizer

struct Bin<'a> {
    input: &'a [u8],
    cursor: usize,
    is64: bool,
}

impl<'a> Bin<'a> {
    fn word(&mut self, end: usize) -> Result<u32, ImportError> {
        if end < self.cursor + 4 {
            return err("cannot ReadWord, out of bounds");
        }
        let v = u32::from_le_bytes(self.input[self.cursor..self.cursor + 4].try_into().unwrap());
        self.cursor += 4;
        Ok(v)
    }
    fn dword(&mut self, end: usize) -> Result<u64, ImportError> {
        if end < self.cursor + 8 {
            return err("cannot ReadDoubleWord, out of bounds");
        }
        let v = u64::from_le_bytes(self.input[self.cursor..self.cursor + 8].try_into().unwrap());
        self.cursor += 8;
        Ok(v)
    }
    fn byte(&mut self, end: usize) -> Result<u8, ImportError> {
        if end < self.cursor + 1 {
            return err("cannot ReadByte, out of bounds");
        }
        let v = self.input[self.cursor];
        self.cursor += 1;
        Ok(v)
    }
    fn string(
        &mut self,
        end: usize,
        long: bool,
        allow_null: bool,
    ) -> Result<(usize, usize), ImportError> {
        let len = if long {
            self.word(end)? as usize
        } else {
            self.byte(end)? as usize
        };
        if end < self.cursor + len {
            return err("cannot ReadString, length is out of bounds");
        }
        let b = self.cursor;
        self.cursor += len;
        if !allow_null && self.input[b..b + len].contains(&0) {
            return err("failed ReadString, unexpected NUL character in string");
        }
        Ok((b, self.cursor))
    }
    fn data(&mut self, end: usize) -> Result<(usize, usize), ImportError> {
        if end < self.cursor + 1 {
            return err("cannot ReadData, out of bounds reading length");
        }
        let ty = self.input[self.cursor];
        let b = self.cursor;
        self.cursor += 1;
        match ty {
            b'Y' => self.cursor += 2,
            b'C' => self.cursor += 1,
            b'I' | b'F' => self.cursor += 4,
            b'D' | b'L' => self.cursor += 8,
            b'R' => {
                let l = self.word(end)? as usize;
                self.cursor += l;
            }
            b'b' => self.cursor = end,
            b'f' | b'd' | b'l' | b'i' | b'c' => {
                let length = self.word(end)?;
                let encoding = self.word(end)?;
                let comp_len = self.word(end)?;
                if encoding == 0 {
                    let stride = match ty {
                        b'f' | b'i' => 4,
                        b'd' | b'l' => 8,
                        _ => 1,
                    };
                    if length * stride != comp_len {
                        return err("cannot ReadData, calculated data stride differs from what the file claims");
                    }
                } else if encoding != 1 {
                    return err("cannot ReadData, unknown encoding");
                }
                self.cursor += comp_len as usize;
            }
            b'S' => {
                self.string(end, true, true)?;
            }
            _ => {
                return err(format!(
                    "cannot ReadData, unexpected type code: {}",
                    ty as char
                ))
            }
        }
        if self.cursor > end {
            return err("cannot ReadData, the remaining size is too small for the data type");
        }
        Ok((b, self.cursor))
    }

    fn scope(&mut self, out: &mut Vec<Token<'a>>, end: usize) -> Result<bool, ImportError> {
        let end_offset = if self.is64 {
            self.dword(end)?
        } else {
            self.word(end)? as u64
        } as usize;
        if end_offset == 0 {
            return Ok(false);
        }
        if end_offset > end {
            return err("block offset is out of range");
        } else if end_offset < self.cursor {
            return err("block offset is negative out of range");
        }
        let prop_count = if self.is64 {
            self.dword(end)?
        } else {
            self.word(end)? as u64
        };
        let prop_length = if self.is64 {
            self.dword(end)?
        } else {
            self.word(end)? as u64
        } as usize;
        let (sb, se) = self.string(end, false, false)?;
        out.push(Token {
            bytes: &self.input[sb..se],
            ty: TokenType::Key,
            line: self.cursor,
            binary: true,
        });
        let begin_cursor = self.cursor;
        if begin_cursor + prop_length > end {
            return err("property length out of bounds reading length");
        }
        for i in 0..prop_count {
            let (b, e) = self.data(begin_cursor + prop_length)?;
            out.push(Token {
                bytes: &self.input[b..e],
                ty: TokenType::Data,
                line: self.cursor,
                binary: true,
            });
            if i != prop_count - 1 {
                let c = self.cursor.min(self.input.len() - 1);
                out.push(Token {
                    bytes: &self.input[c..c + 1],
                    ty: TokenType::Comma,
                    line: self.cursor,
                    binary: true,
                });
            }
        }
        if self.cursor - begin_cursor != prop_length {
            return err("property length not reached, something is wrong");
        }
        let sentinel = if self.is64 { 8 * 3 + 1 } else { 4 * 3 + 1 };
        if self.cursor < end_offset {
            if end_offset - self.cursor < sentinel {
                return err("insufficient padding bytes at block end");
            }
            let c = self.cursor;
            out.push(Token {
                bytes: &self.input[c..c + 1],
                ty: TokenType::OpenBracket,
                line: c,
                binary: true,
            });
            while self.cursor < end_offset - sentinel {
                self.scope(out, end_offset - sentinel)?;
            }
            let c = self.cursor;
            out.push(Token {
                bytes: &self.input[c..c + 1],
                ty: TokenType::CloseBracket,
                line: c,
                binary: true,
            });
            if self.input[c..c + sentinel].iter().any(|&b| b != 0) {
                return err("failed to read nested block sentinel, expected all bytes to be 0");
            }
            self.cursor += sentinel;
        }
        if self.cursor != end_offset {
            return err("scope length not reached, something is wrong");
        }
        Ok(true)
    }
}

/// `input` includes the trailing NUL assimp appends.
pub fn tokenize_binary(input: &[u8]) -> Result<Vec<Token<'_>>, ImportError> {
    let length = input.len();
    if length < 0x1b {
        return err("file is too short");
    }
    if !input.starts_with(b"Kaydara FBX Binary") {
        return err("magic bytes not found");
    }
    let mut b = Bin {
        input,
        cursor: 18,
        is64: false,
    };
    for _ in 0..5 {
        b.byte(length)?;
    }
    let version = b.word(length)?;
    b.is64 = version >= 7500;
    let mut out = Vec::new();
    while b.cursor < length {
        if !b.scope(&mut out, length)? {
            break;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Elements and scopes

#[derive(Debug, Default)]
pub struct Scope<'a> {
    /// `std::multimap<string, Element*>`: sorted by key, insertion order
    /// within a key.
    pub elements: BTreeMap<String, Vec<Element<'a>>>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Element<'a> {
    pub key: Token<'a>,
    pub tokens: Vec<Token<'a>>,
    pub compound: Option<Box<Scope<'a>>>,
}

impl<'a> Scope<'a> {
    /// `Scope::operator[]`: the first element with this key.
    pub fn get(&self, key: &str) -> Option<&Element<'a>> {
        self.elements.get(key).and_then(|v| v.first())
    }

    pub fn collection(&self, key: &str) -> &[Element<'a>] {
        self.elements.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn count(&self, key: &str) -> usize {
        self.elements.get(key).map_or(0, |v| v.len())
    }

    #[allow(dead_code)]
    pub fn find_case_insensitive(&self, key: &str) -> Option<&Element<'a>> {
        self.elements
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .and_then(|(_, v)| v.first())
    }

    /// All elements in multimap order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Element<'a>)> {
        self.elements
            .iter()
            .flat_map(|(k, v)| v.iter().map(move |e| (k, e)))
    }
}

struct Parser<'a, 't> {
    tokens: &'t [Token<'a>],
    cursor: usize,
    current: Option<Token<'a>>,
}

impl<'a, 't> Parser<'a, 't> {
    fn advance(&mut self) -> Option<Token<'a>> {
        self.current = self.tokens.get(self.cursor).copied();
        if self.current.is_some() {
            self.cursor += 1;
        }
        self.current
    }

    fn element(&mut self, key: Token<'a>) -> Result<Element<'a>, ImportError> {
        let mut el = Element {
            key,
            tokens: Vec::new(),
            compound: None,
        };
        loop {
            let mut n = match self.advance() {
                Some(n) => n,
                None => return err("unexpected end of file, expected closing bracket"),
            };
            if n.ty == TokenType::Data {
                el.tokens.push(n);
                let prev = n;
                n = match self.advance() {
                    Some(n) => n,
                    None => return err("unexpected end of file, expected bracket, comma or key"),
                };
                if n.ty == TokenType::Data && prev.ty == TokenType::Data && n.line == prev.line + 1
                {
                    el.tokens.push(n);
                    continue;
                }
                if !matches!(
                    n.ty,
                    TokenType::OpenBracket
                        | TokenType::CloseBracket
                        | TokenType::Comma
                        | TokenType::Key
                ) {
                    return err("unexpected token; expected bracket, comma or key");
                }
            }
            if n.ty == TokenType::OpenBracket {
                el.compound = Some(Box::new(self.scope(false)?));
                match self.current {
                    Some(t) if t.ty == TokenType::CloseBracket => {}
                    _ => return err("expected closing bracket"),
                }
                self.advance();
                return Ok(el);
            }
            if n.ty == TokenType::Key || n.ty == TokenType::CloseBracket {
                return Ok(el);
            }
        }
    }

    fn scope(&mut self, top_level: bool) -> Result<Scope<'a>, ImportError> {
        if !top_level {
            match self.current {
                Some(t) if t.ty == TokenType::OpenBracket => {}
                _ => return err("expected open bracket"),
            }
        }
        let mut sc = Scope::default();
        let mut n = match self.advance() {
            Some(n) => n,
            None => return err("unexpected end of file"),
        };
        while n.ty != TokenType::CloseBracket {
            if n.ty != TokenType::Key {
                return err("unexpected token, expected TOK_KEY");
            }
            let s = n.string_contents();
            if s.is_empty() {
                return err("unexpected content: empty string.");
            }
            let e = self.element(n)?;
            match self.current {
                None => {
                    if top_level {
                        sc.elements.entry(s).or_default().push(e);
                        return Ok(sc);
                    }
                    return err("unexpected end of file");
                }
                Some(t) => {
                    sc.elements.entry(s).or_default().push(e);
                    n = t;
                }
            }
        }
        Ok(sc)
    }
}

pub fn parse<'a>(tokens: &[Token<'a>]) -> Result<Scope<'a>, ImportError> {
    let mut p = Parser {
        tokens,
        cursor: 0,
        current: None,
    };
    p.scope(true)
}

// ---------------------------------------------------------------------------
// Token parsing

pub fn parse_id(t: &Token) -> Result<u64, ImportError> {
    if t.ty != TokenType::Data {
        return err("expected TOK_DATA token");
    }
    if t.binary {
        if t.bytes.first() != Some(&b'L') || t.bytes.len() < 9 {
            return err("failed to parse ID, unexpected data type, expected L(ong) (binary)");
        }
        return Ok(u64::from_le_bytes(t.bytes[1..9].try_into().unwrap()));
    }
    let mut len = t.bytes.len() as u32;
    let (v, n) = strtoul10_64(t.bytes, Some(&mut len));
    if n > t.bytes.len() {
        return err("failed to parse ID (text)");
    }
    Ok(v)
}

pub fn parse_dim(t: &Token) -> Result<usize, ImportError> {
    if t.ty != TokenType::Data {
        return err("expected TOK_DATA token");
    }
    if t.binary {
        if t.bytes.first() != Some(&b'L') {
            return err("failed to parse ID, unexpected data type, expected L(ong) (binary)");
        }
        return Ok(u64::from_le_bytes(t.bytes[1..9].try_into().unwrap()) as usize);
    }
    if t.bytes.first() != Some(&b'*') {
        return err("expected asterisk before array dimension");
    }
    let mut len = t.bytes.len() as u32;
    let (v, _) = strtoul10_64(&t.bytes[1..], Some(&mut len));
    Ok(v as usize)
}

pub fn parse_float(t: &Token) -> Result<f32, ImportError> {
    if t.ty != TokenType::Data {
        return err("expected TOK_DATA token");
    }
    if t.binary {
        return match t.bytes.first() {
            Some(b'F') => Ok(f32::from_le_bytes(t.bytes[1..5].try_into().unwrap())),
            Some(b'D') => Ok(f64::from_le_bytes(t.bytes[1..9].try_into().unwrap()) as f32),
            _ => err("failed to parse F(loat) or D(ouble), unexpected data type (binary)"),
        };
    }
    if t.bytes.len() > 31 {
        return Ok(0.0);
    }
    let mut tmp = t.bytes.to_vec();
    tmp.push(0);
    Ok(fast_atoreal(&tmp, true)?.0)
}

/// assimp's `strtol10` (32-bit, with sign).
fn strtol10(s: &[u8]) -> (i32, usize) {
    let mut i = 0;
    let inv = s.first() == Some(&b'-');
    if inv || s.first() == Some(&b'+') {
        i += 1;
    }
    let mut value: u32 = 0;
    while let Some(&c) = s.get(i) {
        if !c.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add((c - b'0') as u32);
        i += 1;
    }
    let mut v = value as i32;
    if inv && v < i32::MAX {
        v = -v;
    }
    (v, i)
}

pub fn parse_int(t: &Token) -> Result<i32, ImportError> {
    if t.ty != TokenType::Data {
        return err("expected TOK_DATA token");
    }
    if t.binary {
        if t.bytes.first() != Some(&b'I') {
            return err("failed to parse I(nt), unexpected data type (binary)");
        }
        return Ok(i32::from_le_bytes(t.bytes[1..5].try_into().unwrap()));
    }
    let (v, n) = strtol10(t.bytes);
    if n != t.bytes.len() {
        return err("failed to parse ID");
    }
    Ok(v)
}

pub fn parse_int64(t: &Token) -> Result<i64, ImportError> {
    if t.ty != TokenType::Data {
        return err("expected TOK_DATA token");
    }
    if t.binary {
        if t.bytes.first() != Some(&b'L') {
            return err("failed to parse Int64, unexpected data type");
        }
        return Ok(i64::from_le_bytes(t.bytes[1..9].try_into().unwrap()));
    }
    let mut s = t.bytes;
    let inv = s.first() == Some(&b'-');
    if inv || s.first() == Some(&b'+') {
        s = &s[1..];
    }
    let mut len = t.bytes.len() as u32;
    let (v, _) = strtoul10_64(s, Some(&mut len));
    let v = v as i64;
    Ok(if inv { v.wrapping_neg() } else { v })
}

pub fn parse_string(t: &Token) -> Result<String, ImportError> {
    if t.ty != TokenType::Data {
        return err("expected TOK_DATA token");
    }
    if t.binary {
        if t.bytes.first() != Some(&b'S') {
            return err("failed to parse S(tring), unexpected data type (binary)");
        }
        let len = i32::from_le_bytes(t.bytes[1..5].try_into().unwrap()) as usize;
        return Ok(bytes_to_string(&t.bytes[5..5 + len]));
    }
    let b = t.bytes;
    if b.len() < 2 {
        return err("token is too short to hold a string");
    }
    if b[0] != b'"' || b[b.len() - 1] != b'"' {
        return err("expected double quoted string");
    }
    Ok(bytes_to_string(&b[1..b.len() - 1]))
}

/// FBX strings are byte strings; keep every byte (lossless for ASCII and
/// UTF-8, Latin-1 bytes map to U+0080..U+00FF).
pub fn bytes_to_string(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

pub fn required_token<'e, 'a>(el: &'e Element<'a>, i: usize) -> Result<&'e Token<'a>, ImportError> {
    el.tokens
        .get(i)
        .ok_or_else(|| ImportError(format!("FBX: missing token at index {i}")))
}

pub fn required_scope<'e, 'a>(el: &'e Element<'a>) -> Result<&'e Scope<'a>, ImportError> {
    el.compound
        .as_deref()
        .ok_or_else(|| ImportError("FBX: expected compound scope".into()))
}

pub fn required_element<'s, 'a>(
    sc: &'s Scope<'a>,
    key: &str,
) -> Result<&'s Element<'a>, ImportError> {
    sc.get(key)
        .ok_or_else(|| ImportError(format!("FBX: did not find required element \"{key}\"")))
}

// ---------------------------------------------------------------------------
// Data arrays

fn binary_array(t: &Token) -> Result<(u8, u32, Vec<u8>), ImportError> {
    let data = t.bytes;
    if data.len() < 5 {
        return err("binary data array is too short");
    }
    let ty = data[0];
    let count = u32::from_le_bytes(data[1..5].try_into().unwrap());
    if count == 0 {
        return Ok((ty, 0, Vec::new()));
    }
    let enc = u32::from_le_bytes(data[5..9].try_into().unwrap());
    let comp_len = u32::from_le_bytes(data[9..13].try_into().unwrap()) as usize;
    let payload = &data[13..13 + comp_len];
    let stride = match ty {
        b'f' | b'i' => 4,
        b'd' | b'l' => 8,
        _ => 1,
    };
    let full = stride * count as usize;
    let buf = if enc == 0 {
        payload.to_vec()
    } else {
        // zlib stream
        let mut out = miniz_oxide::inflate::decompress_to_vec_zlib(payload)
            .map_err(|_| ImportError("FBX: failed to inflate data array".into()))?;
        out.resize(full, 0);
        out
    };
    Ok((ty, count, buf))
}

fn ascii_values<'e, 'a>(el: &'e Element<'a>) -> Result<&'e [Token<'a>], ImportError> {
    let first = el
        .tokens
        .first()
        .ok_or_else(|| ImportError("FBX: unexpected empty element".into()))?;
    parse_dim(first)?;
    let sc = required_scope(el)?;
    Ok(&required_element(sc, "a")?.tokens)
}

/// `ParseVectorDataArray(std::vector<float>&)` (doubles narrowed).
pub fn read_floats(el: &Element) -> Result<Vec<f32>, ImportError> {
    let first = el
        .tokens
        .first()
        .ok_or_else(|| ImportError("FBX: unexpected empty element".into()))?;
    if first.binary {
        let (ty, count, buf) = binary_array(first)?;
        if count == 0 {
            return Ok(Vec::new());
        }
        return match ty {
            b'd' => Ok(buf
                .chunks_exact(8)
                .map(|c| f64::from_le_bytes(c.try_into().unwrap()) as f32)
                .collect()),
            b'f' => Ok(buf
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect()),
            _ => err("expected float or double array (binary)"),
        };
    }
    ascii_values(el)?.iter().map(parse_float).collect()
}

/// Float arrays grouped by `n` (vec2/vec3/color4) with the element-count
/// checks of the typed overloads.
pub fn read_float_tuples(el: &Element, n: usize) -> Result<Vec<f32>, ImportError> {
    let v = read_floats(el)?;
    if v.len() % n != 0 {
        return err(format!("number of floats is not a multiple of {n}"));
    }
    Ok(v)
}

pub fn read_ints(el: &Element) -> Result<Vec<i32>, ImportError> {
    let first = el
        .tokens
        .first()
        .ok_or_else(|| ImportError("FBX: unexpected empty element".into()))?;
    if first.binary {
        let (ty, count, buf) = binary_array(first)?;
        if count == 0 {
            return Ok(Vec::new());
        }
        if ty != b'i' {
            return err("expected int array (binary)");
        }
        return Ok(buf
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
            .collect());
    }
    ascii_values(el)?.iter().map(parse_int).collect()
}

pub fn read_int64s(el: &Element) -> Result<Vec<i64>, ImportError> {
    let first = el
        .tokens
        .first()
        .ok_or_else(|| ImportError("FBX: unexpected empty element".into()))?;
    if first.binary {
        let (ty, count, buf) = binary_array(first)?;
        if count == 0 {
            return Ok(Vec::new());
        }
        if ty != b'l' {
            return err("expected long array (binary)");
        }
        return Ok(buf
            .chunks_exact(8)
            .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
            .collect());
    }
    ascii_values(el)?.iter().map(parse_int64).collect()
}

/// `ReadMatrix`: 16 values, transposed into assimp's row-major layout.
pub fn read_matrix(el: &Element) -> Result<crate::import::aimath::AiMat4, ImportError> {
    let v = read_floats(el)?;
    if v.len() != 16 {
        return err("expected 16 matrix elements");
    }
    let mut m = crate::import::aimath::AiMat4::IDENTITY;
    for r in 0..4 {
        for c in 0..4 {
            m.m[r][c] = v[r * 4 + c];
        }
    }
    Ok(m.transpose())
}
