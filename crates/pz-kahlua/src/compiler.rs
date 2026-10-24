//! Lua 5.1 compiler: port of the game's `org.luaj.kahluafork.compiler`
//! (`LexState`, `FuncState`). It produces standard Lua 5.1 bytecode with the
//! same register allocation and constant ordering as the original.
//!
//! Source text quirk preserved: the original reads files as UTF-8 into Java
//! chars, stores each char truncated to a byte in the token buffer, then
//! decodes that buffer as UTF-8 again. Non-ASCII text inside names and
//! string literals is therefore mangled exactly as the game mangles it.

use crate::value::{Key, Value};
use crate::vm::{LuaError, Prototype};
use std::collections::HashMap;
use std::rc::Rc;

// Opcodes.
pub const OP_MOVE: u32 = 0;
pub const OP_LOADK: u32 = 1;
pub const OP_LOADBOOL: u32 = 2;
pub const OP_LOADNIL: u32 = 3;
pub const OP_GETUPVAL: u32 = 4;
pub const OP_GETGLOBAL: u32 = 5;
pub const OP_GETTABLE: u32 = 6;
pub const OP_SETGLOBAL: u32 = 7;
pub const OP_SETUPVAL: u32 = 8;
pub const OP_SETTABLE: u32 = 9;
pub const OP_NEWTABLE: u32 = 10;
pub const OP_SELF: u32 = 11;
pub const OP_ADD: u32 = 12;
pub const OP_SUB: u32 = 13;
pub const OP_MUL: u32 = 14;
pub const OP_DIV: u32 = 15;
pub const OP_MOD: u32 = 16;
pub const OP_POW: u32 = 17;
pub const OP_UNM: u32 = 18;
pub const OP_NOT: u32 = 19;
pub const OP_LEN: u32 = 20;
pub const OP_CONCAT: u32 = 21;
pub const OP_JMP: u32 = 22;
pub const OP_EQ: u32 = 23;
pub const OP_LT: u32 = 24;
pub const OP_LE: u32 = 25;
pub const OP_TEST: u32 = 26;
pub const OP_TESTSET: u32 = 27;
pub const OP_CALL: u32 = 28;
pub const OP_TAILCALL: u32 = 29;
pub const OP_RETURN: u32 = 30;
pub const OP_FORLOOP: u32 = 31;
pub const OP_FORPREP: u32 = 32;
pub const OP_TFORLOOP: u32 = 33;
pub const OP_SETLIST: u32 = 34;
pub const OP_CLOSE: u32 = 35;
pub const OP_CLOSURE: u32 = 36;
pub const OP_VARARG: u32 = 37;

const NO_JUMP: i32 = -1;
const NO_REG: i32 = 255;
const MAXSTACK: i32 = 250;
const LUAI_MAXUPVALUES: i32 = 60;
const LUAI_MAXVARS: i32 = 200;
const LUAI_MAXCCALLS: i32 = 200;
const MAX_INT: i32 = 0x7FFF_FFFD;
const MAXARG_SBX: i32 = 131071;
const LFIELDS_PER_FLUSH: i32 = 50;
const EOZ: i32 = -1;

// Expression kinds.
const VVOID: i32 = 0;
const VNIL: i32 = 1;
const VTRUE: i32 = 2;
const VFALSE: i32 = 3;
const VK: i32 = 4;
const VKNUM: i32 = 5;
const VLOCAL: i32 = 6;
const VUPVAL: i32 = 7;
const VGLOBAL: i32 = 8;
const VINDEXED: i32 = 9;
const VJMP: i32 = 10;
const VRELOCABLE: i32 = 11;
const VNONRELOC: i32 = 12;
const VCALL: i32 = 13;
const VVARARG: i32 = 14;

// Tokens.
const TK_AND: i32 = 257;
const TK_BREAK: i32 = 258;
const TK_DO: i32 = 259;
const TK_ELSE: i32 = 260;
const TK_ELSEIF: i32 = 261;
const TK_END: i32 = 262;
const TK_FALSE: i32 = 263;
const TK_FOR: i32 = 264;
const TK_FUNCTION: i32 = 265;
const TK_IF: i32 = 266;
const TK_IN: i32 = 267;
const TK_LOCAL: i32 = 268;
const TK_NIL: i32 = 269;
const TK_NOT: i32 = 270;
const TK_OR: i32 = 271;
const TK_REPEAT: i32 = 272;
const TK_RETURN: i32 = 273;
const TK_THEN: i32 = 274;
const TK_TRUE: i32 = 275;
const TK_UNTIL: i32 = 276;
const TK_WHILE: i32 = 277;
const TK_CONCAT: i32 = 278;
const TK_DOTS: i32 = 279;
const TK_EQ: i32 = 280;
const TK_GE: i32 = 281;
const TK_LE: i32 = 282;
const TK_NE: i32 = 283;
const TK_NUMBER: i32 = 284;
const TK_NAME: i32 = 285;
const TK_STRING: i32 = 286;
const TK_EOS: i32 = 287;

const LUAX_TOKENS: [&str; 31] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "..", "...", "==",
    ">=", "<=", "~=", "<number>", "<name>", "<string>", "<eof>",
];

// Binary operators.
const OPR_ADD: i32 = 0;
const OPR_SUB: i32 = 1;
const OPR_MUL: i32 = 2;
const OPR_DIV: i32 = 3;
const OPR_MOD: i32 = 4;
const OPR_POW: i32 = 5;
const OPR_CONCAT: i32 = 6;
const OPR_NE: i32 = 7;
const OPR_EQ: i32 = 8;
const OPR_LT: i32 = 9;
const OPR_LE: i32 = 10;
const OPR_GT: i32 = 11;
const OPR_GE: i32 = 12;
const OPR_AND: i32 = 13;
const OPR_OR: i32 = 14;
const OPR_NOBINOPR: i32 = 15;
const OPR_MINUS: i32 = 0;
const OPR_NOT: i32 = 1;
const OPR_LEN: i32 = 2;
const OPR_NOUNOPR: i32 = 3;
const PRIORITY_LEFT: [i32; 15] = [6, 6, 7, 7, 7, 10, 5, 3, 3, 3, 3, 3, 3, 2, 1];
const PRIORITY_RIGHT: [i32; 15] = [6, 6, 7, 7, 7, 9, 4, 3, 3, 3, 3, 3, 3, 2, 1];
const UNARY_PRIORITY: i32 = 8;

const OPMODES: [u32; 38] = [
    96, 113, 84, 96, 80, 113, 108, 49, 16, 60, 84, 108, 124, 124, 124, 124, 124, 124, 96, 96, 96,
    104, 34, 188, 188, 188, 228, 228, 84, 84, 16, 98, 98, 132, 20, 0, 81, 80,
];

fn test_t_mode(op: u32) -> bool {
    OPMODES[op as usize] & 0x80 != 0
}

pub fn get_opcode(i: u32) -> u32 {
    i & 0x3F
}
pub fn getarg_a(i: u32) -> i32 {
    ((i >> 6) & 0xFF) as i32
}
pub fn getarg_b(i: u32) -> i32 {
    ((i >> 23) & 0x1FF) as i32
}
pub fn getarg_c(i: u32) -> i32 {
    ((i >> 14) & 0x1FF) as i32
}
pub fn getarg_bx(i: u32) -> i32 {
    ((i >> 14) & 0x3FFFF) as i32
}
pub fn getarg_sbx(i: u32) -> i32 {
    getarg_bx(i) - MAXARG_SBX
}
fn set_opcode(i: &mut u32, o: u32) {
    *i = (*i & 0xFFFF_FFC0) | (o & 0x3F);
}
fn setarg_a(i: &mut u32, u: i32) {
    *i = (*i & 0xFFFF_C03F) | (((u as u32) << 6) & 0x3FC0);
}
fn setarg_b(i: &mut u32, u: i32) {
    *i = (*i & 0x007F_FFFF) | (((u as u32) << 23) & 0xFF80_0000);
}
fn setarg_c(i: &mut u32, u: i32) {
    *i = (*i & 0xFF80_3FFF) | (((u as u32) << 14) & 0x007F_C000);
}
fn setarg_bx(i: &mut u32, u: i32) {
    *i = (*i & 0x3FFF) | (((u as u32) << 14) & 0xFFFF_C000);
}
fn setarg_sbx(i: &mut u32, u: i32) {
    setarg_bx(i, u + MAXARG_SBX);
}
fn create_abc(o: u32, a: i32, b: i32, c: i32) -> u32 {
    (o & 0x3F) | (((a as u32) << 6) & 0x3FC0) | (((b as u32) << 23) & 0xFF80_0000) | (((c as u32) << 14) & 0x007F_C000)
}
fn create_abx(o: u32, a: i32, bc: i32) -> u32 {
    (o & 0x3F) | (((a as u32) << 6) & 0x3FC0) | (((bc as u32) << 14) & 0xFFFF_C000)
}
fn isk(x: i32) -> bool {
    x & 0x100 != 0
}
fn rkask(x: i32) -> i32 {
    x | 0x100
}

#[derive(Clone, Copy, Default)]
struct ExpDesc {
    k: i32,
    info: i32,
    aux: i32,
    nval: f64,
    has_nval: bool,
    t: i32,
    f: i32,
}

impl ExpDesc {
    fn new() -> Self {
        ExpDesc { t: NO_JUMP, f: NO_JUMP, ..Default::default() }
    }
    fn set_nval(&mut self, r: f64) {
        self.nval = r;
        self.has_nval = true;
    }
    fn nval(&self) -> f64 {
        if self.has_nval {
            self.nval
        } else {
            self.info as f64
        }
    }
    fn init(&mut self, k: i32, i: i32) {
        self.f = NO_JUMP;
        self.t = NO_JUMP;
        self.k = k;
        self.info = i;
    }
    fn hasjumps(&self) -> bool {
        self.t != self.f
    }
    fn isnumeral(&self) -> bool {
        self.k == VKNUM && self.t == NO_JUMP && self.f == NO_JUMP
    }
}

#[derive(Clone, Copy)]
struct BlockCnt {
    breaklist: i32,
    nactvar: i32,
    upval: bool,
    isbreakable: bool,
}

#[derive(Clone, Default)]
struct Token {
    token: i32,
    r: f64,
    ts: Option<Rc<str>>,
}

struct FuncState {
    f: Prototype,
    upvalue_names: Vec<Rc<str>>,
    htable: HashMap<Key, i32>,
    bl: Vec<BlockCnt>,
    pc: i32,
    lasttarget: i32,
    jpc: i32,
    freereg: i32,
    nk: i32,
    np: i32,
    nlocvars: i32,
    nactvar: i32,
    upvalues_k: [i32; 60],
    upvalues_info: [i32; 60],
    actvar: [i16; 200],
    linedefined: i32,
    is_vararg: i32,
    locvars: Vec<Rc<str>>,
}

impl FuncState {
    fn new(name: Option<Rc<str>>) -> Self {
        let f = Prototype { name, max_stack_size: 2, ..Default::default() };
        FuncState {
            f,
            upvalue_names: Vec::new(),
            htable: HashMap::new(),
            bl: Vec::new(),
            pc: 0,
            lasttarget: -1,
            jpc: NO_JUMP,
            freereg: 0,
            nk: 0,
            np: 0,
            nlocvars: 0,
            nactvar: 0,
            upvalues_k: [0; 60],
            upvalues_info: [0; 60],
            actvar: [0; 200],
            linedefined: 0,
            is_vararg: 0,
            locvars: Vec::new(),
        }
    }
    fn getlocvar(&self, i: i32) -> &Rc<str> {
        &self.locvars[self.actvar[i as usize] as usize]
    }
}

/// Compile Lua source text. `name` is the chunk name (the original passes
/// the file's base name); `file`/`filename` are recorded on prototypes for
/// stack traces (`FuncState.currentFile`/`currentfullFile`).
pub fn compile(
    source: &str,
    name: Option<&str>,
    file: Option<&str>,
    filename: Option<&str>,
) -> Result<Prototype, LuaError> {
    let chars: Vec<u16> = source.encode_utf16().collect();
    let (chunk_name, src_name) = match name {
        Some(n) => (n.to_string(), n.to_string()),
        None => ("stdin".to_string(), format!("[string \"{}\"]", trim(source, 80))),
    };
    let mut ls = LexState::new(chars, src_name);
    ls.file = file.map(Rc::from);
    ls.filename = filename.map(Rc::from);
    let mut fs = FuncState::new(Some(Rc::from(chunk_name.as_str())));
    fs.is_vararg = 2;
    ls.fs.push(fs);
    ls.next()?;
    ls.chunk()?;
    ls.check(TK_EOS)?;
    let f = ls.close_func()?;
    Ok(f)
}

fn trim(s: &str, max: usize) -> String {
    let u: Vec<u16> = s.encode_utf16().collect();
    if u.len() > max {
        format!("{}...", String::from_utf16_lossy(&u[..max - 3]))
    } else {
        s.to_string()
    }
}

struct LexState {
    chars: Vec<u16>,
    pos: usize,
    current: i32,
    linenumber: i32,
    lastline: i32,
    t: Token,
    lookahead: Token,
    fs: Vec<FuncState>,
    buff: Vec<u8>,
    source: String,
    n_ccalls: i32,
    file: Option<Rc<str>>,
    filename: Option<Rc<str>>,
}

type R<T> = Result<T, LuaError>;

fn compile_error(msg: String) -> LuaError {
    LuaError::Kahlua(Value::from(msg))
}

impl LexState {
    fn new(chars: Vec<u16>, source: String) -> Self {
        let mut ls = LexState {
            chars,
            pos: 0,
            current: EOZ,
            linenumber: 1,
            lastline: 1,
            t: Token::default(),
            lookahead: Token { token: TK_EOS, ..Default::default() },
            fs: Vec::new(),
            buff: Vec::with_capacity(32),
            source,
            n_ccalls: 0,
            file: None,
            filename: None,
        };
        ls.next_char();
        // skipShebang
        if ls.current == '#' as i32 {
            while !ls.curr_is_newline() && ls.current != EOZ {
                ls.next_char();
            }
        }
        ls
    }

    fn fs(&mut self) -> &mut FuncState {
        self.fs.last_mut().unwrap()
    }

    // ---- lexer ----

    fn next_char(&mut self) {
        self.current = match self.chars.get(self.pos) {
            Some(c) => {
                self.pos += 1;
                *c as i32
            }
            None => EOZ,
        };
    }
    fn curr_is_newline(&self) -> bool {
        self.current == 10 || self.current == 13
    }
    fn save_and_next(&mut self) {
        self.save(self.current);
        self.next_char();
    }
    fn save(&mut self, c: i32) {
        self.buff.push(c as u8);
    }
    fn isalnum(c: i32) -> bool {
        (48..=57).contains(&c) || (97..=122).contains(&c) || (65..=90).contains(&c) || c == 95
    }
    fn isalpha(c: i32) -> bool {
        (97..=122).contains(&c) || (65..=90).contains(&c)
    }
    fn isdigit(c: i32) -> bool {
        (48..=57).contains(&c)
    }
    fn isspace(c: i32) -> bool {
        c <= 32
    }

    fn token2str(token: i32) -> String {
        if token < 257 {
            if token < 32 {
                format!("char({token})")
            } else {
                char::from_u32(token as u32).map(String::from).unwrap_or_default()
            }
        } else {
            LUAX_TOKENS[(token - 257) as usize].to_string()
        }
    }

    fn txt_token(&self, token: i32) -> String {
        match token {
            TK_NUMBER | TK_NAME | TK_STRING => String::from_utf8_lossy(&self.buff).into_owned(),
            _ => Self::token2str(token),
        }
    }

    fn lexerror<T>(&self, msg: &str, token: i32) -> R<T> {
        let m = if token != 0 {
            format!("{}:{}: {} near `{}`", self.source, self.linenumber, msg, self.txt_token(token))
        } else {
            format!("{}:{}: {}", self.source, self.linenumber, msg)
        };
        Err(compile_error(m))
    }

    fn syntaxerror<T>(&self, msg: &str) -> R<T> {
        self.lexerror(msg, self.t.token)
    }

    fn newstring(bytes: &[u8]) -> Rc<str> {
        Rc::from(String::from_utf8_lossy(bytes).as_ref())
    }

    fn inclinenumber(&mut self) -> R<()> {
        let old = self.current;
        self.next_char();
        if self.curr_is_newline() && self.current != old {
            self.next_char();
        }
        self.linenumber += 1;
        if self.linenumber >= MAX_INT {
            return self.syntaxerror("chunk has too many lines");
        }
        Ok(())
    }

    fn check_next(&mut self, set: &str) -> bool {
        if self.current < 0 || !set.encode_utf16().any(|c| c as i32 == self.current) {
            return false;
        }
        self.save_and_next();
        true
    }

    fn str2d(&self, s: &str) -> R<f64> {
        let r = if let Some(hex) = s.strip_prefix("0x") {
            i64::from_str_radix(hex, 16).ok().map(|v| v as f64)
        } else {
            crate::number::java_parse_double(s)
        };
        match r {
            Some(d) => Ok(d),
            None => self.lexerror("malformed number", TK_NUMBER),
        }
    }

    fn read_numeral(&mut self, token_is_lookahead: bool) -> R<()> {
        loop {
            self.save_and_next();
            if !(Self::isdigit(self.current) || self.current == 46) {
                break;
            }
        }
        if self.check_next("Ee") {
            self.check_next("+-");
        }
        while Self::isalnum(self.current) || self.current == 95 {
            self.save_and_next();
        }
        let s = String::from_utf8_lossy(&self.buff).into_owned();
        let d = self.str2d(&s)?;
        self.tok(token_is_lookahead).r = d;
        Ok(())
    }

    fn tok(&mut self, lookahead: bool) -> &mut Token {
        if lookahead {
            &mut self.lookahead
        } else {
            &mut self.t
        }
    }

    fn skip_sep(&mut self) -> i32 {
        let mut count = 0;
        let s = self.current;
        self.save_and_next();
        while self.current == 61 {
            self.save_and_next();
            count += 1;
        }
        if self.current == s {
            count
        } else {
            -count - 1
        }
    }

    /// `token == None` reads a long comment.
    fn read_long_string(&mut self, token: Option<bool>, sep: i32) -> R<()> {
        self.save_and_next();
        if self.curr_is_newline() {
            self.inclinenumber()?;
        }
        loop {
            match self.current {
                EOZ => {
                    let msg = if token.is_some() { "unfinished long string" } else { "unfinished long comment" };
                    return self.lexerror(msg, TK_EOS);
                }
                91 => {
                    if self.skip_sep() == sep {
                        self.save_and_next();
                    }
                }
                93 => {
                    if self.skip_sep() == sep {
                        self.save_and_next();
                        break;
                    }
                }
                10 | 13 => {
                    self.save(10);
                    self.inclinenumber()?;
                    if token.is_none() {
                        self.buff.clear();
                    }
                }
                _ => {
                    if token.is_some() {
                        self.save_and_next();
                    } else {
                        self.next_char();
                    }
                }
            }
        }
        if let Some(la) = token {
            let start = (2 + sep) as usize;
            let len = self.buff.len() as i32 - 2 * (2 + sep);
            let s = Self::newstring(&self.buff[start..start + len.max(0) as usize]);
            self.tok(la).ts = Some(s);
        }
        Ok(())
    }

    fn read_string(&mut self, del: i32, la: bool) -> R<()> {
        self.save_and_next();
        while self.current != del {
            match self.current {
                EOZ => return self.lexerror("unfinished string", TK_EOS),
                10 | 13 => return self.lexerror("unfinished string", TK_STRING),
                92 => {
                    self.next_char();
                    let c = match self.current {
                        97 => 7,
                        98 => 8,
                        102 => 12,
                        110 => 10,
                        114 => 13,
                        116 => 9,
                        118 => 11,
                        10 | 13 => {
                            self.save(10);
                            self.inclinenumber()?;
                            continue;
                        }
                        EOZ => continue,
                        _ => {
                            if !Self::isdigit(self.current) {
                                self.save_and_next();
                                continue;
                            }
                            let mut i = 0;
                            let mut c = 0;
                            loop {
                                c = 10 * c + self.current - 48;
                                self.next_char();
                                i += 1;
                                if !(i < 3 && Self::isdigit(self.current)) {
                                    break;
                                }
                            }
                            if c > 255 {
                                return self.lexerror("escape sequence too large", TK_STRING);
                            }
                            self.save(c);
                            continue;
                        }
                    };
                    self.save(c);
                    self.next_char();
                }
                _ => self.save_and_next(),
            }
        }
        self.save_and_next();
        let n = self.buff.len();
        let s = Self::newstring(&self.buff[1..n - 1]);
        self.tok(la).ts = Some(s);
        Ok(())
    }

    fn llex(&mut self, la: bool) -> R<i32> {
        self.buff.clear();
        loop {
            match self.current {
                10 | 13 => {
                    self.inclinenumber()?;
                    continue;
                }
                45 => {
                    self.next_char();
                    if self.current != 45 {
                        return Ok(45);
                    }
                    self.next_char();
                    if self.current == 91 {
                        let sep = self.skip_sep();
                        self.buff.clear();
                        if sep >= 0 {
                            self.read_long_string(None, sep)?;
                            self.buff.clear();
                            continue;
                        }
                    }
                    while !self.curr_is_newline() && self.current != EOZ {
                        self.next_char();
                    }
                    continue;
                }
                91 => {
                    let sep = self.skip_sep();
                    if sep >= 0 {
                        self.read_long_string(Some(la), sep)?;
                        return Ok(TK_STRING);
                    }
                    if sep == -1 {
                        return Ok(91);
                    }
                    return self.lexerror("invalid long string delimiter", TK_STRING);
                }
                61 => {
                    self.next_char();
                    if self.current != 61 {
                        return Ok(61);
                    }
                    self.next_char();
                    return Ok(TK_EQ);
                }
                60 => {
                    self.next_char();
                    if self.current != 61 {
                        return Ok(60);
                    }
                    self.next_char();
                    return Ok(TK_LE);
                }
                62 => {
                    self.next_char();
                    if self.current != 61 {
                        return Ok(62);
                    }
                    self.next_char();
                    return Ok(TK_GE);
                }
                126 => {
                    self.next_char();
                    if self.current != 61 {
                        return Ok(126);
                    }
                    self.next_char();
                    return Ok(TK_NE);
                }
                34 | 39 => {
                    self.read_string(self.current, la)?;
                    return Ok(TK_STRING);
                }
                46 => {
                    self.save_and_next();
                    if self.check_next(".") {
                        if self.check_next(".") {
                            return Ok(TK_DOTS);
                        }
                        return Ok(TK_CONCAT);
                    }
                    if !Self::isdigit(self.current) {
                        return Ok(46);
                    }
                    self.read_numeral(la)?;
                    return Ok(TK_NUMBER);
                }
                EOZ => return Ok(TK_EOS),
                _ => {}
            }
            if !Self::isspace(self.current) {
                break;
            }
            self.next_char();
        }
        if Self::isdigit(self.current) {
            self.read_numeral(la)?;
            return Ok(TK_NUMBER);
        }
        if Self::isalpha(self.current) || self.current == 95 {
            loop {
                self.save_and_next();
                if !(Self::isalnum(self.current) || self.current == 95) {
                    break;
                }
            }
            let ts = Self::newstring(&self.buff);
            if let Some(i) = LUAX_TOKENS[..21].iter().position(|t| **t == *ts) {
                return Ok(257 + i as i32);
            }
            self.tok(la).ts = Some(ts);
            return Ok(TK_NAME);
        }
        let c = self.current;
        self.next_char();
        Ok(c)
    }

    fn next(&mut self) -> R<()> {
        self.lastline = self.linenumber;
        if self.lookahead.token != TK_EOS {
            self.t = self.lookahead.clone();
            self.lookahead.token = TK_EOS;
        } else {
            self.t.token = self.llex(false)?;
        }
        Ok(())
    }

    fn lookahead_tok(&mut self) -> R<()> {
        self.lookahead.token = self.llex(true)?;
        Ok(())
    }

    // ---- parser helpers ----

    fn hasmultret(k: i32) -> bool {
        k == VCALL || k == VVARARG
    }
    fn error_expected<T>(&self, token: i32) -> R<T> {
        self.syntaxerror(&format!("'{}' expected", Self::token2str(token)))
    }
    fn testnext(&mut self, c: i32) -> R<bool> {
        if self.t.token == c {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn check(&self, c: i32) -> R<()> {
        if self.t.token != c {
            return self.error_expected(c);
        }
        Ok(())
    }
    fn checknext(&mut self, c: i32) -> R<()> {
        self.check(c)?;
        self.next()
    }
    fn check_condition(&self, c: bool, msg: &str) -> R<()> {
        if !c {
            return self.syntaxerror(msg);
        }
        Ok(())
    }
    fn check_match(&mut self, what: i32, who: i32, where_: i32) -> R<()> {
        if !self.testnext(what)? {
            if where_ == self.linenumber {
                return self.error_expected(what);
            }
            return self.syntaxerror(&format!(
                "'{}' expected (to close '{}' at line {})",
                Self::token2str(what),
                Self::token2str(who),
                where_
            ));
        }
        Ok(())
    }
    fn str_checkname(&mut self) -> R<Rc<str>> {
        self.check(TK_NAME)?;
        let ts = self.t.ts.clone().unwrap_or_else(|| Rc::from(""));
        self.next()?;
        Ok(ts)
    }
    fn codestring(&mut self, e: &mut ExpDesc, s: Rc<str>) {
        let k = self.string_k(s);
        e.init(VK, k);
    }
    fn checkname(&mut self, e: &mut ExpDesc) -> R<()> {
        let n = self.str_checkname()?;
        self.codestring(e, n);
        Ok(())
    }
    fn registerlocalvar(&mut self, varname: Rc<str>) -> i32 {
        let fs = self.fs();
        fs.locvars.truncate(fs.nlocvars as usize);
        fs.locvars.push(varname);
        fs.nlocvars += 1;
        fs.nlocvars - 1
    }
    fn new_localvar(&mut self, name: Rc<str>, n: i32) -> R<()> {
        let v = self.fs().nactvar + n + 1;
        self.checklimit(v, LUAI_MAXVARS, "local variables")?;
        let idx = self.registerlocalvar(name);
        let fs = self.fs();
        fs.actvar[(fs.nactvar + n) as usize] = idx as i16;
        Ok(())
    }
    fn adjustlocalvars(&mut self, nvars: i32) {
        self.fs().nactvar += nvars;
    }
    fn removevars(&mut self, tolevel: i32) {
        self.fs().nactvar = tolevel;
    }
    fn singlevar(&mut self, var: &mut ExpDesc) -> R<()> {
        let varname = self.str_checkname()?;
        let top = self.fs.len() - 1;
        if self.singlevaraux(top, &varname, var, 1)? == VGLOBAL {
            var.info = self.string_k(varname);
        }
        Ok(())
    }
    fn adjust_assign(&mut self, nvars: i32, nexps: i32, e: &mut ExpDesc) -> R<()> {
        let mut extra = nvars - nexps;
        if Self::hasmultret(e.k) {
            extra += 1;
            if extra < 0 {
                extra = 0;
            }
            self.setreturns(e, extra);
            if extra > 1 {
                self.reserveregs(extra - 1)?;
            }
        } else {
            if e.k != VVOID {
                self.exp2nextreg(e)?;
            }
            if extra > 0 {
                let reg = self.fs().freereg;
                self.reserveregs(extra)?;
                self.nil(reg, extra);
            }
        }
        Ok(())
    }
    fn enterlevel(&mut self) -> R<()> {
        self.n_ccalls += 1;
        if self.n_ccalls > LUAI_MAXCCALLS {
            return self.lexerror("chunk has too many syntax levels", 0);
        }
        Ok(())
    }
    fn leavelevel(&mut self) {
        self.n_ccalls -= 1;
    }

    fn pushclosure(&mut self, func: FuncState, v: &mut ExpDesc) {
        let nup = func.f.num_upvalues as usize;
        let upk = func.upvalues_k;
        let upi = func.upvalues_info;
        let fs = self.fs();
        fs.f.prototypes.truncate(fs.np as usize);
        fs.f.prototypes.push(Rc::new(func.f));
        fs.np += 1;
        let np = fs.np;
        let pc = self.code_abx(OP_CLOSURE, 0, np - 1);
        v.init(VRELOCABLE, pc);
        for i in 0..nup {
            let o = if upk[i] == VLOCAL { OP_MOVE } else { OP_GETUPVAL };
            self.code_abc(o, 0, upi[i], 0);
        }
    }

    fn close_func(&mut self) -> R<Prototype> {
        self.fs().f.is_vararg = self.fs().is_vararg != 0;
        self.removevars(0);
        self.ret(0, 0);
        let mut fs = self.fs.pop().unwrap();
        let pc = fs.pc as usize;
        fs.f.code.truncate(pc);
        fs.f.lines.truncate(pc);
        fs.f.constants.truncate(fs.nk as usize);
        fs.f.prototypes.truncate(fs.np as usize);
        fs.locvars.truncate(fs.nlocvars as usize);
        fs.f.locvars = fs.locvars;
        Ok(fs.f)
    }

    fn field(&mut self, v: &mut ExpDesc) -> R<()> {
        let mut key = ExpDesc::new();
        self.exp2anyreg(v)?;
        self.next()?;
        self.checkname(&mut key)?;
        self.indexed(v, &mut key)
    }

    fn yindex(&mut self, v: &mut ExpDesc) -> R<()> {
        self.next()?;
        self.expr(v)?;
        self.exp2val(v)?;
        self.checknext(93)
    }

    fn recfield(&mut self, cc_t_info: i32, nh: &mut i32) -> R<()> {
        let reg = self.fs().freereg;
        let mut key = ExpDesc::new();
        let mut val = ExpDesc::new();
        if self.t.token == TK_NAME {
            self.checklimit(*nh, MAX_INT, "items in a constructor")?;
            self.checkname(&mut key)?;
        } else {
            self.yindex(&mut key)?;
        }
        *nh += 1;
        self.checknext(61)?;
        let rkkey = self.exp2rk(&mut key)?;
        self.expr(&mut val)?;
        let rkval = self.exp2rk(&mut val)?;
        self.code_abc(OP_SETTABLE, cc_t_info, rkkey, rkval);
        self.fs().freereg = reg;
        Ok(())
    }

    fn constructor(&mut self, t: &mut ExpDesc) -> R<()> {
        let line = self.linenumber;
        let pc = self.code_abc(OP_NEWTABLE, 0, 0, 0);
        let mut v = ExpDesc::new();
        let mut tostore = 0;
        let mut nh = 0;
        let mut na = 0;
        t.init(VRELOCABLE, pc);
        v.init(VVOID, 0);
        self.exp2nextreg(t)?;
        self.checknext(123)?;
        loop {
            if self.t.token == 125 {
                break;
            }
            // closelistfield
            if v.k != VVOID {
                self.exp2nextreg(&mut v)?;
                v.k = VVOID;
                if tostore == LFIELDS_PER_FLUSH {
                    self.setlist(t.info, na, tostore);
                    tostore = 0;
                }
            }
            match self.t.token {
                TK_NAME => {
                    self.lookahead_tok()?;
                    if self.lookahead.token != 61 {
                        self.listfield(&mut v, &mut na, &mut tostore)?;
                    } else {
                        self.recfield(t.info, &mut nh)?;
                    }
                }
                91 => self.recfield(t.info, &mut nh)?,
                _ => self.listfield(&mut v, &mut na, &mut tostore)?,
            }
            if !(self.testnext(44)? || self.testnext(59)?) {
                break;
            }
        }
        self.check_match(125, 123, line)?;
        // lastlistfield
        if tostore != 0 {
            if Self::hasmultret(v.k) {
                self.setreturns(&mut v, -1);
                self.setlist(t.info, na, -1);
                na -= 1;
            } else {
                if v.k != VVOID {
                    self.exp2nextreg(&mut v)?;
                }
                self.setlist(t.info, na, tostore);
            }
        }
        let code = &mut self.fs().f.code[pc as usize];
        setarg_b(code, int2fb(na));
        setarg_c(code, int2fb(nh));
        Ok(())
    }

    fn listfield(&mut self, v: &mut ExpDesc, na: &mut i32, tostore: &mut i32) -> R<()> {
        self.expr(v)?;
        self.checklimit(*na, MAX_INT, "items in a constructor")?;
        *na += 1;
        *tostore += 1;
        Ok(())
    }

    fn parlist(&mut self) -> R<()> {
        let mut nparams = 0;
        self.fs().is_vararg = 0;
        if self.t.token != 41 {
            loop {
                match self.t.token {
                    TK_NAME => {
                        let n = self.str_checkname()?;
                        self.new_localvar(n, nparams)?;
                        nparams += 1;
                    }
                    TK_DOTS => {
                        self.next()?;
                        self.fs().is_vararg |= 2;
                    }
                    _ => return self.syntaxerror("<name> or '...' expected"),
                }
                if !(self.fs().is_vararg == 0 && self.testnext(44)?) {
                    break;
                }
            }
        }
        self.adjustlocalvars(nparams);
        let fs = self.fs();
        fs.f.num_params = (fs.nactvar - (fs.is_vararg & 1)) as u8;
        let n = fs.nactvar;
        self.reserveregs(n)
    }

    fn body(&mut self, e: &mut ExpDesc, needself: bool, line: i32) -> R<()> {
        let name = self.t.ts.clone();
        let mut new_fs = FuncState::new(name);
        new_fs.linedefined = line;
        self.fs.push(new_fs);
        self.checknext(40)?;
        if needself {
            self.new_localvar(Rc::from("self"), 0)?;
            self.adjustlocalvars(1);
        }
        self.parlist()?;
        self.checknext(41)?;
        self.chunk()?;
        self.check_match(TK_END, TK_FUNCTION, line)?;
        // close_func, but keep the FuncState for pushclosure's upvalue info.
        self.fs().f.is_vararg = self.fs().is_vararg != 0;
        self.removevars(0);
        self.ret(0, 0);
        let mut fs = self.fs.pop().unwrap();
        let pc = fs.pc as usize;
        fs.f.code.truncate(pc);
        fs.f.lines.truncate(pc);
        fs.f.constants.truncate(fs.nk as usize);
        fs.f.prototypes.truncate(fs.np as usize);
        fs.locvars.truncate(fs.nlocvars as usize);
        fs.f.locvars = std::mem::take(&mut fs.locvars);
        self.pushclosure(fs, e);
        Ok(())
    }

    fn explist1(&mut self, v: &mut ExpDesc) -> R<i32> {
        let mut n = 1;
        self.expr(v)?;
        while self.testnext(44)? {
            self.exp2nextreg(v)?;
            self.expr(v)?;
            n += 1;
        }
        Ok(n)
    }

    fn funcargs(&mut self, f: &mut ExpDesc) -> R<()> {
        let mut args = ExpDesc::new();
        let line = self.linenumber;
        match self.t.token {
            40 => {
                if line != self.lastline {
                    return self.syntaxerror("ambiguous syntax (function call x new statement)");
                }
                self.next()?;
                if self.t.token == 41 {
                    args.k = VVOID;
                } else {
                    self.explist1(&mut args)?;
                    self.setreturns(&mut args, -1);
                }
                self.check_match(41, 40, line)?;
            }
            123 => self.constructor(&mut args)?,
            TK_STRING => {
                let s = self.t.ts.clone().unwrap_or_else(|| Rc::from(""));
                self.codestring(&mut args, s);
                self.next()?;
            }
            _ => return self.syntaxerror("function arguments expected"),
        }
        let base = f.info;
        let nparams = if Self::hasmultret(args.k) {
            -1
        } else {
            if args.k != VVOID {
                self.exp2nextreg(&mut args)?;
            }
            self.fs().freereg - (base + 1)
        };
        let pc = self.code_abc(OP_CALL, base, nparams + 1, 2);
        f.init(VCALL, pc);
        self.fixline(line);
        self.fs().freereg = base + 1;
        Ok(())
    }

    fn prefixexp(&mut self, v: &mut ExpDesc) -> R<()> {
        match self.t.token {
            40 => {
                let line = self.linenumber;
                self.next()?;
                self.expr(v)?;
                self.check_match(41, 40, line)?;
                self.dischargevars(v);
                Ok(())
            }
            TK_NAME => self.singlevar(v),
            _ => self.syntaxerror("unexpected symbol"),
        }
    }

    fn primaryexp(&mut self, v: &mut ExpDesc) -> R<()> {
        self.prefixexp(v)?;
        loop {
            match self.t.token {
                46 => self.field(v)?,
                91 => {
                    let mut key = ExpDesc::new();
                    self.exp2anyreg(v)?;
                    self.yindex(&mut key)?;
                    self.indexed(v, &mut key)?;
                }
                58 => {
                    let mut key = ExpDesc::new();
                    self.next()?;
                    self.checkname(&mut key)?;
                    self.self_(v, &mut key)?;
                    self.funcargs(v)?;
                }
                40 | 123 | TK_STRING => {
                    self.exp2nextreg(v)?;
                    self.funcargs(v)?;
                }
                _ => return Ok(()),
            }
        }
    }

    fn simpleexp(&mut self, v: &mut ExpDesc) -> R<()> {
        match self.t.token {
            TK_NUMBER => {
                v.init(VKNUM, 0);
                v.set_nval(self.t.r);
            }
            TK_STRING => {
                let s = self.t.ts.clone().unwrap_or_else(|| Rc::from(""));
                self.codestring(v, s);
            }
            TK_NIL => v.init(VNIL, 0),
            TK_TRUE => v.init(VTRUE, 0),
            TK_FALSE => v.init(VFALSE, 0),
            TK_DOTS => {
                let iv = self.fs().is_vararg;
                self.check_condition(iv != 0, "cannot use '...' outside a vararg function")?;
                self.fs().is_vararg &= !4;
                let pc = self.code_abc(OP_VARARG, 0, 1, 0);
                v.init(VVARARG, pc);
            }
            123 => return self.constructor(v),
            TK_FUNCTION => {
                self.next()?;
                let line = self.linenumber;
                return self.body(v, false, line);
            }
            _ => return self.primaryexp(v),
        }
        self.next()
    }

    fn getunopr(op: i32) -> i32 {
        match op {
            TK_NOT => OPR_NOT,
            45 => OPR_MINUS,
            35 => OPR_LEN,
            _ => OPR_NOUNOPR,
        }
    }

    fn getbinopr(op: i32) -> i32 {
        match op {
            43 => OPR_ADD,
            45 => OPR_SUB,
            42 => OPR_MUL,
            47 => OPR_DIV,
            37 => OPR_MOD,
            94 => OPR_POW,
            TK_CONCAT => OPR_CONCAT,
            TK_NE => OPR_NE,
            TK_EQ => OPR_EQ,
            60 => OPR_LT,
            TK_LE => OPR_LE,
            62 => OPR_GT,
            TK_GE => OPR_GE,
            TK_AND => OPR_AND,
            TK_OR => OPR_OR,
            _ => OPR_NOBINOPR,
        }
    }

    fn subexpr(&mut self, v: &mut ExpDesc, limit: i32) -> R<i32> {
        self.enterlevel()?;
        let uop = Self::getunopr(self.t.token);
        if uop != OPR_NOUNOPR {
            self.next()?;
            self.subexpr(v, UNARY_PRIORITY)?;
            self.prefix(uop, v)?;
        } else {
            self.simpleexp(v)?;
        }
        let mut op = Self::getbinopr(self.t.token);
        while op != OPR_NOBINOPR && PRIORITY_LEFT[op as usize] > limit {
            let mut v2 = ExpDesc::new();
            self.next()?;
            self.infix(op, v)?;
            let nextop = self.subexpr(&mut v2, PRIORITY_RIGHT[op as usize])?;
            self.posfix(op, v, &mut v2)?;
            op = nextop;
        }
        self.leavelevel();
        Ok(op)
    }

    fn expr(&mut self, v: &mut ExpDesc) -> R<()> {
        self.subexpr(v, 0).map(|_| ())
    }

    fn block_follow(token: i32) -> bool {
        matches!(token, TK_ELSE | TK_ELSEIF | TK_END | TK_UNTIL | TK_EOS)
    }

    fn block(&mut self) -> R<()> {
        self.enterblock(false);
        self.chunk()?;
        self.leaveblock();
        Ok(())
    }

    fn check_conflict(&mut self, lhs: &mut [ExpDesc], v: &ExpDesc) -> R<()> {
        let extra = self.fs().freereg;
        let mut conflict = false;
        for lh in lhs.iter_mut().rev() {
            if lh.k == VINDEXED {
                if lh.info == v.info {
                    conflict = true;
                    lh.info = extra;
                }
                if lh.aux == v.info {
                    conflict = true;
                    lh.aux = extra;
                }
            }
        }
        if conflict {
            let fr = self.fs().freereg;
            self.code_abc(OP_MOVE, fr, v.info, 0);
            self.reserveregs(1)?;
        }
        Ok(())
    }

    /// `assignment` with the LHS chain held in `lhs` (last = current).
    fn assignment(&mut self, lhs: &mut Vec<ExpDesc>, nvars: i32) -> R<()> {
        let mut e = ExpDesc::new();
        let k = lhs.last().unwrap().k;
        self.check_condition((VLOCAL..=VINDEXED).contains(&k), "syntax error")?;
        if self.testnext(44)? {
            let mut nv = ExpDesc::new();
            self.primaryexp(&mut nv)?;
            if nv.k == VLOCAL {
                self.check_conflict(lhs, &nv)?;
            }
            lhs.push(nv);
            self.assignment(lhs, nvars + 1)?;
            lhs.pop();
        } else {
            self.checknext(61)?;
            let nexps = self.explist1(&mut e)?;
            if nexps != nvars {
                self.adjust_assign(nvars, nexps, &mut e)?;
                if nexps > nvars {
                    self.fs().freereg -= nexps - nvars;
                }
            } else {
                self.setoneret(&mut e);
                let mut var = *lhs.last().unwrap();
                self.storevar(&mut var, &mut e)?;
                return Ok(());
            }
        }
        let fr = self.fs().freereg;
        e.init(VNONRELOC, fr - 1);
        let mut var = *lhs.last().unwrap();
        self.storevar(&mut var, &mut e)
    }

    fn cond(&mut self) -> R<i32> {
        let mut v = ExpDesc::new();
        self.expr(&mut v)?;
        if v.k == VNIL {
            v.k = VFALSE;
        }
        self.goiftrue(&mut v)?;
        Ok(v.f)
    }

    fn breakstat(&mut self) -> R<()> {
        let mut upval = false;
        let mut idx = None;
        for (i, bl) in self.fs().bl.iter().enumerate().rev() {
            if bl.isbreakable {
                idx = Some(i);
                break;
            }
            upval |= bl.upval;
        }
        let Some(i) = idx else { return self.syntaxerror("no loop to break") };
        if upval {
            let n = self.fs().bl[i].nactvar;
            self.code_abc(OP_CLOSE, n, 0, 0);
        }
        let j = self.jump();
        let bl = self.fs().bl[i].breaklist;
        let l = self.concat(bl, j)?;
        self.fs().bl[i].breaklist = l;
        Ok(())
    }

    fn whilestat(&mut self, line: i32) -> R<()> {
        self.next()?;
        let whileinit = self.getlabel();
        let condexit = self.cond()?;
        self.enterblock(true);
        self.checknext(TK_DO)?;
        self.block()?;
        let j = self.jump();
        self.patchlist(j, whileinit)?;
        self.check_match(TK_END, TK_WHILE, line)?;
        self.leaveblock();
        self.patchtohere(condexit)
    }

    fn repeatstat(&mut self, line: i32) -> R<()> {
        let repeat_init = self.getlabel();
        self.enterblock(true);
        self.enterblock(false);
        self.next()?;
        self.chunk()?;
        self.check_match(TK_UNTIL, TK_REPEAT, line)?;
        let condexit = self.cond()?;
        let bl2_upval = self.fs().bl.last().unwrap().upval;
        if !bl2_upval {
            self.leaveblock();
            self.patchlist(condexit, repeat_init)?;
        } else {
            self.breakstat()?;
            self.patchtohere(condexit)?;
            self.leaveblock();
            let j = self.jump();
            self.patchlist(j, repeat_init)?;
        }
        self.leaveblock();
        Ok(())
    }

    fn exp1(&mut self) -> R<i32> {
        let mut e = ExpDesc::new();
        self.expr(&mut e)?;
        let k = e.k;
        self.exp2nextreg(&mut e)?;
        Ok(k)
    }

    fn forbody(&mut self, base: i32, line: i32, nvars: i32, isnum: bool) -> R<()> {
        self.adjustlocalvars(3);
        self.checknext(TK_DO)?;
        let prep = if isnum { self.code_asbx(OP_FORPREP, base, NO_JUMP) } else { self.jump() };
        self.enterblock(false);
        self.adjustlocalvars(nvars);
        self.reserveregs(nvars)?;
        self.block()?;
        self.leaveblock();
        self.patchtohere(prep)?;
        let endfor = if isnum {
            self.code_asbx(OP_FORLOOP, base, NO_JUMP)
        } else {
            self.code_abc(OP_TFORLOOP, base, 0, nvars)
        };
        self.fixline(line);
        let l = if isnum { endfor } else { self.jump() };
        self.patchlist(l, prep + 1)
    }

    fn fornum(&mut self, varname: Rc<str>, line: i32) -> R<()> {
        let base = self.fs().freereg;
        self.new_localvar(Rc::from("(for index)"), 0)?;
        self.new_localvar(Rc::from("(for limit)"), 1)?;
        self.new_localvar(Rc::from("(for step)"), 2)?;
        self.new_localvar(varname, 3)?;
        self.checknext(61)?;
        self.exp1()?;
        self.checknext(44)?;
        self.exp1()?;
        if self.testnext(44)? {
            self.exp1()?;
        } else {
            let fr = self.fs().freereg;
            let k = self.number_k(1.0);
            self.code_abx(OP_LOADK, fr, k);
            self.reserveregs(1)?;
        }
        self.forbody(base, line, 1, true)
    }

    fn forlist(&mut self, indexname: Rc<str>) -> R<()> {
        let mut e = ExpDesc::new();
        let mut nvars = 0;
        let base = self.fs().freereg;
        self.new_localvar(Rc::from("(for generator)"), nvars)?;
        nvars += 1;
        self.new_localvar(Rc::from("(for state)"), nvars)?;
        nvars += 1;
        self.new_localvar(Rc::from("(for control)"), nvars)?;
        nvars += 1;
        self.new_localvar(indexname, nvars)?;
        nvars += 1;
        while self.testnext(44)? {
            let n = self.str_checkname()?;
            self.new_localvar(n, nvars)?;
            nvars += 1;
        }
        self.checknext(TK_IN)?;
        let line = self.linenumber;
        let n = self.explist1(&mut e)?;
        self.adjust_assign(3, n, &mut e)?;
        self.checkstack(3)?;
        self.forbody(base, line, nvars - 3, false)
    }

    fn forstat(&mut self, line: i32) -> R<()> {
        self.enterblock(true);
        self.next()?;
        let varname = self.str_checkname()?;
        match self.t.token {
            61 => self.fornum(varname, line)?,
            44 | TK_IN => self.forlist(varname)?,
            _ => return self.syntaxerror("'=' or 'in' expected"),
        }
        self.check_match(TK_END, TK_FOR, line)?;
        self.leaveblock();
        Ok(())
    }

    fn test_then_block(&mut self) -> R<i32> {
        self.next()?;
        let condexit = self.cond()?;
        self.checknext(TK_THEN)?;
        self.block()?;
        Ok(condexit)
    }

    fn ifstat(&mut self, line: i32) -> R<()> {
        let mut escapelist = NO_JUMP;
        let mut flist = self.test_then_block()?;
        while self.t.token == TK_ELSEIF {
            let j = self.jump();
            escapelist = self.concat(escapelist, j)?;
            self.patchtohere(flist)?;
            flist = self.test_then_block()?;
        }
        if self.t.token == TK_ELSE {
            let j = self.jump();
            escapelist = self.concat(escapelist, j)?;
            self.patchtohere(flist)?;
            self.next()?;
            self.block()?;
        } else {
            escapelist = self.concat(escapelist, flist)?;
        }
        self.patchtohere(escapelist)?;
        self.check_match(TK_END, TK_IF, line)
    }

    fn localfunc(&mut self) -> R<()> {
        let mut v = ExpDesc::new();
        let mut b = ExpDesc::new();
        let n = self.str_checkname()?;
        self.new_localvar(n, 0)?;
        let fr = self.fs().freereg;
        v.init(VLOCAL, fr);
        self.reserveregs(1)?;
        self.adjustlocalvars(1);
        let line = self.linenumber;
        self.body(&mut b, false, line)?;
        self.storevar(&mut v, &mut b)
    }

    fn localstat(&mut self) -> R<()> {
        let mut nvars = 0;
        let mut e = ExpDesc::new();
        loop {
            let n = self.str_checkname()?;
            self.new_localvar(n, nvars)?;
            nvars += 1;
            if !self.testnext(44)? {
                break;
            }
        }
        let nexps = if self.testnext(61)? {
            self.explist1(&mut e)?
        } else {
            e.k = VVOID;
            0
        };
        self.adjust_assign(nvars, nexps, &mut e)?;
        self.adjustlocalvars(nvars);
        Ok(())
    }

    fn funcname(&mut self, v: &mut ExpDesc) -> R<bool> {
        let mut needself = false;
        self.singlevar(v)?;
        while self.t.token == 46 {
            self.field(v)?;
        }
        if self.t.token == 58 {
            needself = true;
            self.field(v)?;
        }
        Ok(needself)
    }

    fn funcstat(&mut self, line: i32) -> R<()> {
        let mut v = ExpDesc::new();
        let mut b = ExpDesc::new();
        self.next()?;
        let needself = self.funcname(&mut v)?;
        self.body(&mut b, needself, line)?;
        self.storevar(&mut v, &mut b)?;
        self.fixline(line);
        Ok(())
    }

    fn exprstat(&mut self) -> R<()> {
        let mut v = ExpDesc::new();
        self.primaryexp(&mut v)?;
        if v.k == VCALL {
            let code = &mut self.fs().f.code[v.info as usize];
            setarg_c(code, 1);
            Ok(())
        } else {
            let mut lhs = vec![v];
            self.assignment(&mut lhs, 1)
        }
    }

    fn retstat(&mut self) -> R<()> {
        let mut e = ExpDesc::new();
        self.next()?;
        let (first, nret);
        if Self::block_follow(self.t.token) || self.t.token == 59 {
            nret = 0;
            first = 0;
        } else {
            let n = self.explist1(&mut e)?;
            if Self::hasmultret(e.k) {
                self.setreturns(&mut e, -1);
                if e.k == VCALL && n == 1 {
                    let code = &mut self.fs().f.code[e.info as usize];
                    set_opcode(code, OP_TAILCALL);
                }
                first = self.fs().nactvar;
                nret = -1;
            } else if n == 1 {
                first = self.exp2anyreg(&mut e)?;
                nret = 1;
            } else {
                self.exp2nextreg(&mut e)?;
                first = self.fs().nactvar;
                nret = n;
            }
        }
        self.ret(first, nret);
        Ok(())
    }

    fn statement(&mut self) -> R<bool> {
        let line = self.linenumber;
        match self.t.token {
            TK_IF => self.ifstat(line).map(|_| false),
            TK_WHILE => self.whilestat(line).map(|_| false),
            TK_DO => {
                self.next()?;
                self.block()?;
                self.check_match(TK_END, TK_DO, line)?;
                Ok(false)
            }
            TK_FOR => self.forstat(line).map(|_| false),
            TK_REPEAT => self.repeatstat(line).map(|_| false),
            TK_FUNCTION => self.funcstat(line).map(|_| false),
            TK_LOCAL => {
                self.next()?;
                if self.testnext(TK_FUNCTION)? {
                    self.localfunc()?;
                } else {
                    self.localstat()?;
                }
                Ok(false)
            }
            TK_RETURN => self.retstat().map(|_| true),
            TK_BREAK => {
                self.next()?;
                self.breakstat()?;
                Ok(true)
            }
            _ => self.exprstat().map(|_| false),
        }
    }

    fn chunk(&mut self) -> R<()> {
        let mut islast = false;
        self.enterlevel()?;
        while !islast && !Self::block_follow(self.t.token) {
            islast = self.statement()?;
            self.testnext(59)?;
            let fs = self.fs();
            fs.freereg = fs.nactvar;
        }
        self.leavelevel();
        Ok(())
    }

    // ---- FuncState code generation ----

    fn checklimit(&self, v: i32, l: i32, msg: &str) -> R<()> {
        if v > l {
            let fs = self.fs.last().unwrap();
            let m = if fs.linedefined == 0 {
                format!("main function has more than {l} {msg}")
            } else {
                format!("function at line {} has more than {l} {msg}", fs.linedefined)
            };
            return self.lexerror(&m, 0);
        }
        Ok(())
    }

    fn indexupvalue(&mut self, level: usize, name: &Rc<str>, v: &ExpDesc) -> R<i32> {
        {
            let fs = &self.fs[level];
            for i in 0..fs.f.num_upvalues as usize {
                if fs.upvalues_k[i] == v.k && fs.upvalues_info[i] == v.info {
                    return Ok(i as i32);
                }
            }
        }
        let n = self.fs[level].f.num_upvalues as i32;
        // checklimit against this function
        if n + 1 > LUAI_MAXUPVALUES {
            let fs = &self.fs[level];
            let m = if fs.linedefined == 0 {
                format!("main function has more than {LUAI_MAXUPVALUES} upvalues")
            } else {
                format!("function at line {} has more than {LUAI_MAXUPVALUES} upvalues", fs.linedefined)
            };
            return self.lexerror(&m, 0);
        }
        let fs = &mut self.fs[level];
        let i = n as usize;
        fs.upvalue_names.push(name.clone());
        fs.upvalues_k[i] = v.k;
        fs.upvalues_info[i] = v.info;
        fs.f.num_upvalues += 1;
        Ok(n)
    }

    fn searchvar(&self, level: usize, n: &str) -> i32 {
        let fs = &self.fs[level];
        for i in (0..fs.nactvar).rev() {
            if **fs.getlocvar(i) == *n {
                return i;
            }
        }
        -1
    }

    fn markupval(&mut self, level: usize, var: i32) {
        let fs = &mut self.fs[level];
        if let Some(bl) = fs.bl.iter_mut().rev().find(|bl| bl.nactvar <= var) {
            bl.upval = true;
        }
    }

    fn singlevaraux(&mut self, level: usize, n: &Rc<str>, var: &mut ExpDesc, base: i32) -> R<i32> {
        let v = self.searchvar(level, n);
        if v >= 0 {
            var.init(VLOCAL, v);
            if base == 0 {
                self.markupval(level, v);
            }
            return Ok(VLOCAL);
        }
        if level == 0 {
            var.init(VGLOBAL, NO_REG);
            return Ok(VGLOBAL);
        }
        if self.singlevaraux(level - 1, n, var, 0)? == VGLOBAL {
            return Ok(VGLOBAL);
        }
        var.info = self.indexupvalue(level, n, var)?;
        var.k = VUPVAL;
        Ok(VUPVAL)
    }

    fn enterblock(&mut self, isbreakable: bool) {
        let fs = self.fs();
        let nactvar = fs.nactvar;
        fs.bl.push(BlockCnt { breaklist: NO_JUMP, isbreakable, nactvar, upval: false });
    }

    fn leaveblock(&mut self) {
        let bl = self.fs().bl.pop().unwrap();
        self.removevars(bl.nactvar);
        if bl.upval {
            self.code_abc(OP_CLOSE, bl.nactvar, 0, 0);
        }
        let fs = self.fs();
        fs.freereg = fs.nactvar;
        // patchtohere cannot fail for valid lists.
        let _ = self.patchtohere(bl.breaklist);
    }

    fn nil(&mut self, from: i32, n: i32) {
        let fs = self.fs();
        if fs.pc > fs.lasttarget {
            if fs.pc == 0 {
                if from >= fs.nactvar {
                    return;
                }
            } else {
                let prev = fs.f.code[(fs.pc - 1) as usize];
                if get_opcode(prev) == OP_LOADNIL {
                    let pfrom = getarg_a(prev);
                    let pto = getarg_b(prev);
                    if pfrom <= from && from <= pto + 1 {
                        if from + n - 1 > pto {
                            let pc = (fs.pc - 1) as usize;
                            setarg_b(&mut fs.f.code[pc], from + n - 1);
                        }
                        return;
                    }
                }
            }
        }
        self.code_abc(OP_LOADNIL, from, from + n - 1, 0);
    }

    fn jump(&mut self) -> i32 {
        let jpc = self.fs().jpc;
        self.fs().jpc = NO_JUMP;
        let j = self.code_asbx(OP_JMP, 0, NO_JUMP);
        // concat cannot fail here: offsets are small.
        self.concat(j, jpc).unwrap_or(j)
    }

    fn ret(&mut self, first: i32, nret: i32) {
        self.code_abc(OP_RETURN, first, nret + 1, 0);
    }

    fn condjump(&mut self, op: u32, a: i32, b: i32, c: i32) -> i32 {
        self.code_abc(op, a, b, c);
        self.jump()
    }

    fn fixjump(&mut self, pc: i32, dest: i32) -> R<()> {
        let offset = dest - (pc + 1);
        if offset.abs() > MAXARG_SBX {
            return self.syntaxerror("control structure too long");
        }
        setarg_sbx(&mut self.fs().f.code[pc as usize], offset);
        Ok(())
    }

    fn getlabel(&mut self) -> i32 {
        let fs = self.fs();
        fs.lasttarget = fs.pc;
        fs.pc
    }

    fn getjump(&self, pc: i32) -> i32 {
        let offset = getarg_sbx(self.fs.last().unwrap().f.code[pc as usize]);
        if offset == NO_JUMP {
            NO_JUMP
        } else {
            pc + 1 + offset
        }
    }

    fn getjumpcontrol(&self, pc: i32) -> usize {
        let code = &self.fs.last().unwrap().f.code;
        if pc >= 1 && test_t_mode(get_opcode(code[(pc - 1) as usize])) {
            (pc - 1) as usize
        } else {
            pc as usize
        }
    }

    fn need_value(&self, mut list: i32) -> bool {
        while list != NO_JUMP {
            let i = self.fs.last().unwrap().f.code[self.getjumpcontrol(list)];
            if get_opcode(i) != OP_TESTSET {
                return true;
            }
            list = self.getjump(list);
        }
        false
    }

    fn patchtestreg(&mut self, node: i32, reg: i32) -> bool {
        let idx = self.getjumpcontrol(node);
        let code = &mut self.fs().f.code[idx];
        let i = *code;
        if get_opcode(i) != OP_TESTSET {
            return false;
        }
        if reg != NO_REG && reg != getarg_b(i) {
            setarg_a(code, reg);
        } else {
            *code = create_abc(OP_TEST, getarg_b(i), 0, getarg_c(i));
        }
        true
    }

    fn removevalues(&mut self, mut list: i32) {
        while list != NO_JUMP {
            self.patchtestreg(list, NO_REG);
            list = self.getjump(list);
        }
    }

    fn patchlistaux(&mut self, mut list: i32, vtarget: i32, reg: i32, dtarget: i32) -> R<()> {
        while list != NO_JUMP {
            let next = self.getjump(list);
            if self.patchtestreg(list, reg) {
                self.fixjump(list, vtarget)?;
            } else {
                self.fixjump(list, dtarget)?;
            }
            list = next;
        }
        Ok(())
    }

    fn dischargejpc(&mut self) -> R<()> {
        let (jpc, pc) = (self.fs().jpc, self.fs().pc);
        self.patchlistaux(jpc, pc, NO_REG, pc)?;
        self.fs().jpc = NO_JUMP;
        Ok(())
    }

    fn patchlist(&mut self, list: i32, target: i32) -> R<()> {
        if target == self.fs().pc {
            self.patchtohere(list)
        } else {
            self.patchlistaux(list, target, NO_REG, target)
        }
    }

    fn patchtohere(&mut self, list: i32) -> R<()> {
        self.getlabel();
        let jpc = self.fs().jpc;
        let l = self.concat(jpc, list)?;
        self.fs().jpc = l;
        Ok(())
    }

    fn concat(&mut self, l1: i32, l2: i32) -> R<i32> {
        if l2 == NO_JUMP {
            return Ok(l1);
        }
        if l1 == NO_JUMP {
            return Ok(l2);
        }
        let mut list = l1;
        loop {
            let next = self.getjump(list);
            if next == NO_JUMP {
                break;
            }
            list = next;
        }
        self.fixjump(list, l2)?;
        Ok(l1)
    }

    fn checkstack(&mut self, n: i32) -> R<()> {
        let newstack = self.fs().freereg + n;
        if newstack > self.fs().f.max_stack_size as i32 {
            if newstack >= MAXSTACK {
                return self.syntaxerror("function or expression too complex");
            }
            self.fs().f.max_stack_size = newstack as u8;
        }
        Ok(())
    }

    fn reserveregs(&mut self, n: i32) -> R<()> {
        self.checkstack(n)?;
        self.fs().freereg += n;
        Ok(())
    }

    fn freereg(&mut self, reg: i32) {
        let fs = self.fs();
        if !isk(reg) && reg >= fs.nactvar {
            fs.freereg -= 1;
        }
    }

    fn freeexp(&mut self, e: &ExpDesc) {
        if e.k == VNONRELOC {
            self.freereg(e.info);
        }
    }

    fn addk(&mut self, v: Value) -> i32 {
        let fs = self.fs();
        let key = Key(v.clone());
        if let Some(i) = fs.htable.get(&key) {
            return *i;
        }
        let idx = fs.nk;
        fs.htable.insert(key, idx);
        fs.f.constants.truncate(idx as usize);
        fs.f.constants.push(v);
        fs.nk += 1;
        idx
    }

    fn string_k(&mut self, s: Rc<str>) -> i32 {
        self.addk(Value::Str(s))
    }
    fn number_k(&mut self, r: f64) -> i32 {
        self.addk(Value::Num(r))
    }
    fn bool_k(&mut self, b: bool) -> i32 {
        self.addk(Value::Bool(b))
    }
    fn nil_k(&mut self) -> i32 {
        // The original keys nil with a private sentinel object.
        let fs = self.fs();
        let key = Key(Value::Native(nil_sentinel()));
        if let Some(i) = fs.htable.get(&key) {
            return *i;
        }
        let idx = fs.nk;
        fs.htable.insert(key, idx);
        fs.f.constants.truncate(idx as usize);
        fs.f.constants.push(Value::Nil);
        fs.nk += 1;
        idx
    }

    fn setreturns(&mut self, e: &mut ExpDesc, nresults: i32) {
        if e.k == VCALL {
            setarg_c(&mut self.fs().f.code[e.info as usize], nresults + 1);
        } else if e.k == VVARARG {
            let fr = self.fs().freereg;
            let code = &mut self.fs().f.code[e.info as usize];
            setarg_b(code, nresults + 1);
            setarg_a(code, fr);
            let _ = self.reserveregs(1);
        }
    }

    fn setoneret(&mut self, e: &mut ExpDesc) {
        if e.k == VCALL {
            e.k = VNONRELOC;
            e.info = getarg_a(self.fs().f.code[e.info as usize]);
        } else if e.k == VVARARG {
            setarg_b(&mut self.fs().f.code[e.info as usize], 2);
            e.k = VRELOCABLE;
        }
    }

    fn dischargevars(&mut self, e: &mut ExpDesc) {
        match e.k {
            VLOCAL => e.k = VNONRELOC,
            VUPVAL => {
                e.info = self.code_abc(OP_GETUPVAL, 0, e.info, 0);
                e.k = VRELOCABLE;
            }
            VGLOBAL => {
                e.info = self.code_abx(OP_GETGLOBAL, 0, e.info);
                e.k = VRELOCABLE;
            }
            VINDEXED => {
                self.freereg(e.aux);
                self.freereg(e.info);
                e.info = self.code_abc(OP_GETTABLE, 0, e.info, e.aux);
                e.k = VRELOCABLE;
            }
            VCALL | VVARARG => self.setoneret(e),
            _ => {}
        }
    }

    fn code_label(&mut self, a: i32, b: i32, jump: i32) -> i32 {
        self.getlabel();
        self.code_abc(OP_LOADBOOL, a, b, jump)
    }

    fn discharge2reg(&mut self, e: &mut ExpDesc, reg: i32) {
        self.dischargevars(e);
        match e.k {
            VNIL => self.nil(reg, 1),
            VTRUE | VFALSE => {
                self.code_abc(OP_LOADBOOL, reg, if e.k == VTRUE { 1 } else { 0 }, 0);
            }
            VK => {
                self.code_abx(OP_LOADK, reg, e.info);
            }
            VKNUM => {
                let k = self.number_k(e.nval());
                self.code_abx(OP_LOADK, reg, k);
            }
            VRELOCABLE => setarg_a(&mut self.fs().f.code[e.info as usize], reg),
            VNONRELOC => {
                if reg != e.info {
                    self.code_abc(OP_MOVE, reg, e.info, 0);
                }
            }
            _ => return,
        }
        e.info = reg;
        e.k = VNONRELOC;
    }

    fn discharge2anyreg(&mut self, e: &mut ExpDesc) -> R<()> {
        if e.k != VNONRELOC {
            self.reserveregs(1)?;
            let fr = self.fs().freereg;
            self.discharge2reg(e, fr - 1);
        }
        Ok(())
    }

    fn exp2reg(&mut self, e: &mut ExpDesc, reg: i32) -> R<()> {
        self.discharge2reg(e, reg);
        if e.k == VJMP {
            e.t = self.concat(e.t, e.info)?;
        }
        if e.hasjumps() {
            let mut p_f = NO_JUMP;
            let mut p_t = NO_JUMP;
            if self.need_value(e.t) || self.need_value(e.f) {
                let fj = if e.k == VJMP { NO_JUMP } else { self.jump() };
                p_f = self.code_label(reg, 0, 1);
                p_t = self.code_label(reg, 1, 0);
                self.patchtohere(fj)?;
            }
            let final_ = self.getlabel();
            self.patchlistaux(e.f, final_, reg, p_f)?;
            self.patchlistaux(e.t, final_, reg, p_t)?;
        }
        e.t = NO_JUMP;
        e.f = NO_JUMP;
        e.info = reg;
        e.k = VNONRELOC;
        Ok(())
    }

    fn exp2nextreg(&mut self, e: &mut ExpDesc) -> R<()> {
        self.dischargevars(e);
        self.freeexp(e);
        self.reserveregs(1)?;
        let fr = self.fs().freereg;
        self.exp2reg(e, fr - 1)
    }

    fn exp2anyreg(&mut self, e: &mut ExpDesc) -> R<i32> {
        self.dischargevars(e);
        if e.k == VNONRELOC {
            if !e.hasjumps() {
                return Ok(e.info);
            }
            if e.info >= self.fs().nactvar {
                let r = e.info;
                self.exp2reg(e, r)?;
                return Ok(e.info);
            }
        }
        self.exp2nextreg(e)?;
        Ok(e.info)
    }

    fn exp2val(&mut self, e: &mut ExpDesc) -> R<()> {
        if e.hasjumps() {
            self.exp2anyreg(e)?;
        } else {
            self.dischargevars(e);
        }
        Ok(())
    }

    fn exp2rk(&mut self, e: &mut ExpDesc) -> R<i32> {
        self.exp2val(e)?;
        match e.k {
            VNIL | VTRUE | VFALSE | VKNUM => {
                if self.fs().nk <= 255 {
                    e.info = if e.k == VNIL {
                        self.nil_k()
                    } else if e.k == VKNUM {
                        self.number_k(e.nval())
                    } else {
                        self.bool_k(e.k == VTRUE)
                    };
                    e.k = VK;
                    return Ok(rkask(e.info));
                }
            }
            VK => {
                if e.info <= 255 {
                    return Ok(rkask(e.info));
                }
            }
            _ => {}
        }
        self.exp2anyreg(e)
    }

    fn storevar(&mut self, var: &mut ExpDesc, ex: &mut ExpDesc) -> R<()> {
        match var.k {
            VLOCAL => {
                self.freeexp(ex);
                return self.exp2reg(ex, var.info);
            }
            VUPVAL => {
                let e = self.exp2anyreg(ex)?;
                self.code_abc(OP_SETUPVAL, e, var.info, 0);
            }
            VGLOBAL => {
                let e = self.exp2anyreg(ex)?;
                self.code_abx(OP_SETGLOBAL, e, var.info);
            }
            VINDEXED => {
                let e = self.exp2rk(ex)?;
                self.code_abc(OP_SETTABLE, var.info, var.aux, e);
            }
            _ => return Err(compile_error("compiler assert failed".into())),
        }
        self.freeexp(ex);
        Ok(())
    }

    fn self_(&mut self, e: &mut ExpDesc, key: &mut ExpDesc) -> R<()> {
        self.exp2anyreg(e)?;
        self.freeexp(e);
        let func = self.fs().freereg;
        self.reserveregs(2)?;
        let k = self.exp2rk(key)?;
        self.code_abc(OP_SELF, func, e.info, k);
        self.freeexp(key);
        e.info = func;
        e.k = VNONRELOC;
        Ok(())
    }

    fn invertjump(&mut self, e: &ExpDesc) {
        let idx = self.getjumpcontrol(e.info);
        let code = &mut self.fs().f.code[idx];
        let a = getarg_a(*code);
        setarg_a(code, if a != 0 { 0 } else { 1 });
    }

    fn jumponcond(&mut self, e: &mut ExpDesc, cond: i32) -> R<i32> {
        if e.k == VRELOCABLE {
            let ie = self.fs().f.code[e.info as usize];
            if get_opcode(ie) == OP_NOT {
                self.fs().pc -= 1;
                return Ok(self.condjump(OP_TEST, getarg_b(ie), 0, if cond != 0 { 0 } else { 1 }));
            }
        }
        self.discharge2anyreg(e)?;
        self.freeexp(e);
        Ok(self.condjump(OP_TESTSET, NO_REG, e.info, cond))
    }

    fn goiftrue(&mut self, e: &mut ExpDesc) -> R<()> {
        self.dischargevars(e);
        let pc = match e.k {
            VTRUE | VK | VKNUM => NO_JUMP,
            VFALSE => self.jump(),
            VJMP => {
                self.invertjump(e);
                e.info
            }
            _ => self.jumponcond(e, 0)?,
        };
        e.f = self.concat(e.f, pc)?;
        self.patchtohere(e.t)?;
        e.t = NO_JUMP;
        Ok(())
    }

    fn goiffalse(&mut self, e: &mut ExpDesc) -> R<()> {
        self.dischargevars(e);
        let pc = match e.k {
            VNIL | VFALSE => NO_JUMP,
            VTRUE => self.jump(),
            VJMP => e.info,
            _ => self.jumponcond(e, 1)?,
        };
        e.t = self.concat(e.t, pc)?;
        self.patchtohere(e.f)?;
        e.f = NO_JUMP;
        Ok(())
    }

    fn codenot(&mut self, e: &mut ExpDesc) -> R<()> {
        self.dischargevars(e);
        match e.k {
            VNIL | VFALSE => e.k = VTRUE,
            VTRUE | VK | VKNUM => e.k = VFALSE,
            VJMP => self.invertjump(e),
            VRELOCABLE | VNONRELOC => {
                self.discharge2anyreg(e)?;
                self.freeexp(e);
                e.info = self.code_abc(OP_NOT, 0, e.info, 0);
                e.k = VRELOCABLE;
            }
            _ => return Err(compile_error("compiler assert failed".into())),
        }
        std::mem::swap(&mut e.f, &mut e.t);
        self.removevalues(e.f);
        self.removevalues(e.t);
        Ok(())
    }

    fn indexed(&mut self, t: &mut ExpDesc, k: &mut ExpDesc) -> R<()> {
        t.aux = self.exp2rk(k)?;
        t.k = VINDEXED;
        Ok(())
    }

    fn constfolding(op: u32, e1: &mut ExpDesc, e2: &ExpDesc) -> bool {
        if !e1.isnumeral() || !e2.isnumeral() {
            return false;
        }
        let v1 = e1.nval();
        let v2 = e2.nval();
        let r = match op {
            OP_ADD => v1 + v2,
            OP_SUB => v1 - v2,
            OP_MUL => v1 * v2,
            OP_DIV => v1 / v2,
            OP_MOD => v1 % v2, // Java % on doubles: truncated remainder.
            OP_UNM => -v1,
            _ => return false,
        };
        if r.is_nan() || r.is_infinite() {
            return false;
        }
        e1.set_nval(r);
        true
    }

    fn codearith(&mut self, op: u32, e1: &mut ExpDesc, e2: &mut ExpDesc) -> R<()> {
        if Self::constfolding(op, e1, e2) {
            return Ok(());
        }
        let o2 = if op != OP_UNM && op != OP_LEN { self.exp2rk(e2)? } else { 0 };
        let o1 = self.exp2rk(e1)?;
        if o1 > o2 {
            self.freeexp(e1);
            self.freeexp(e2);
        } else {
            self.freeexp(e2);
            self.freeexp(e1);
        }
        e1.info = self.code_abc(op, 0, o1, o2);
        e1.k = VRELOCABLE;
        Ok(())
    }

    fn codecomp(&mut self, op: u32, mut cond: i32, e1: &mut ExpDesc, e2: &mut ExpDesc) -> R<()> {
        let mut o1 = self.exp2rk(e1)?;
        let mut o2 = self.exp2rk(e2)?;
        self.freeexp(e2);
        self.freeexp(e1);
        if cond == 0 && op != OP_EQ {
            std::mem::swap(&mut o1, &mut o2);
            cond = 1;
        }
        e1.info = self.condjump(op, cond, o1, o2);
        e1.k = VJMP;
        Ok(())
    }

    fn prefix(&mut self, op: i32, e: &mut ExpDesc) -> R<()> {
        let mut e2 = ExpDesc::new();
        e2.init(VKNUM, 0);
        match op {
            OPR_MINUS => {
                if e.k == VK {
                    self.exp2anyreg(e)?;
                }
                self.codearith(OP_UNM, e, &mut e2)
            }
            OPR_NOT => self.codenot(e),
            OPR_LEN => {
                self.exp2anyreg(e)?;
                self.codearith(OP_LEN, e, &mut e2)
            }
            _ => Err(compile_error("compiler assert failed".into())),
        }
    }

    fn infix(&mut self, op: i32, v: &mut ExpDesc) -> R<()> {
        match op {
            OPR_AND => self.goiftrue(v),
            OPR_OR => self.goiffalse(v),
            OPR_CONCAT => self.exp2nextreg(v),
            OPR_ADD | OPR_SUB | OPR_MUL | OPR_DIV | OPR_MOD | OPR_POW => {
                if !v.isnumeral() {
                    self.exp2rk(v)?;
                }
                Ok(())
            }
            _ => self.exp2rk(v).map(|_| ()),
        }
    }

    fn posfix(&mut self, op: i32, e1: &mut ExpDesc, e2: &mut ExpDesc) -> R<()> {
        match op {
            OPR_AND => {
                self.dischargevars(e2);
                e2.f = self.concat(e2.f, e1.f)?;
                *e1 = *e2;
                Ok(())
            }
            OPR_OR => {
                self.dischargevars(e2);
                e2.t = self.concat(e2.t, e1.t)?;
                *e1 = *e2;
                Ok(())
            }
            OPR_CONCAT => {
                self.exp2val(e2)?;
                if e2.k == VRELOCABLE && get_opcode(self.fs().f.code[e2.info as usize]) == OP_CONCAT {
                    self.freeexp(e1);
                    setarg_b(&mut self.fs().f.code[e2.info as usize], e1.info);
                    e1.k = VRELOCABLE;
                    e1.info = e2.info;
                    Ok(())
                } else {
                    self.exp2nextreg(e2)?;
                    self.codearith(OP_CONCAT, e1, e2)
                }
            }
            OPR_ADD => self.codearith(OP_ADD, e1, e2),
            OPR_SUB => self.codearith(OP_SUB, e1, e2),
            OPR_MUL => self.codearith(OP_MUL, e1, e2),
            OPR_DIV => self.codearith(OP_DIV, e1, e2),
            OPR_MOD => self.codearith(OP_MOD, e1, e2),
            OPR_POW => self.codearith(OP_POW, e1, e2),
            OPR_EQ => self.codecomp(OP_EQ, 1, e1, e2),
            OPR_NE => self.codecomp(OP_EQ, 0, e1, e2),
            OPR_LT => self.codecomp(OP_LT, 1, e1, e2),
            OPR_LE => self.codecomp(OP_LE, 1, e1, e2),
            OPR_GT => self.codecomp(OP_LT, 0, e1, e2),
            OPR_GE => self.codecomp(OP_LE, 0, e1, e2),
            _ => Err(compile_error("compiler assert failed".into())),
        }
    }

    fn fixline(&mut self, line: i32) {
        let fs = self.fs();
        let pc = (fs.pc - 1) as usize;
        fs.f.lines[pc] = line;
    }

    fn code(&mut self, instruction: u32, line: i32) -> i32 {
        let _ = self.dischargejpc();
        let file = self.file.clone();
        let filename = self.filename.clone();
        let fs = self.fs();
        let pc = fs.pc as usize;
        fs.f.code.truncate(pc);
        fs.f.code.push(instruction);
        fs.f.lines.truncate(pc);
        fs.f.lines.push(line);
        fs.f.file = file;
        fs.f.filename = filename;
        fs.pc += 1;
        fs.pc - 1
    }

    fn code_abc(&mut self, o: u32, a: i32, b: i32, c: i32) -> i32 {
        let line = self.lastline;
        self.code(create_abc(o, a, b, c), line)
    }

    fn code_abx(&mut self, o: u32, a: i32, bc: i32) -> i32 {
        let line = self.lastline;
        self.code(create_abx(o, a, bc), line)
    }

    fn code_asbx(&mut self, o: u32, a: i32, sbx: i32) -> i32 {
        self.code_abx(o, a, sbx + MAXARG_SBX)
    }

    fn setlist(&mut self, base: i32, nelems: i32, tostore: i32) {
        let c = (nelems - 1) / LFIELDS_PER_FLUSH + 1;
        let b = if tostore == -1 { 0 } else { tostore };
        if c <= 511 {
            self.code_abc(OP_SETLIST, base, b, c);
        } else {
            self.code_abc(OP_SETLIST, base, b, 0);
            let line = self.lastline;
            self.code(c as u32, line);
        }
        self.fs().freereg = base + 1;
    }
}

fn int2fb(mut x: i32) -> i32 {
    let mut e = 0;
    while x >= 16 {
        x = (x + 1) >> 1;
        e += 1;
    }
    if x < 8 {
        x
    } else {
        ((e + 1) << 3) | (x - 8)
    }
}

thread_local! {
    static NIL_SENTINEL: Rc<crate::vm::NativeFunction> =
        crate::vm::NativeFunction::new("<nil constant>", |_| Ok(0));
}

fn nil_sentinel() -> Rc<crate::vm::NativeFunction> {
    NIL_SENTINEL.with(|s| s.clone())
}
