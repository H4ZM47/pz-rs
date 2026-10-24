//! `zombie.debug` logging: `DebugType`, `LogSeverity`, `DebugLog`,
//! `DebugLogStream`, `IDebugLogFormatter`, `DebugLogCfgFile` /
//! `DebugLogProfile`, the log-file side of `zombie.core.logger`
//! (`LoggerManager`, `ZLogger`) as far as `DebugLog` uses it, and the
//! `DebugOptions` option registry.
//!
//! # Output
//!
//! Console lines are byte-identical to the original:
//!
//! ```text
//! LOG  : General      f:0> text
//! WARN : Lua          f:12 at Lua(Vanilla).corpseStorageCheck.lua > text
//! ```
//!
//! (`<severity prefix><type left-justified to 12> f:<frame>[ t:<ms>][ st:<ms>][ at <caller left-justified to 36>]> <text>`),
//! and log-file lines are what `ZLogger.writeUnsafe` writes:
//! `[dd-MM-yy HH:mm:ss.SSS] <console line without newline>.`.
//!
//! Everything non-deterministic is an input: the frame number
//! ([`DebugLog::frame_no`], `IsoWorld.getFrameNo()`), wall clock, local
//! time-zone offset and server time ([`LogClock`]), and the caller stack
//! element used for the ` at ...` affix (passed by the caller as a
//! [`StackElement`], since there is no JVM stack to walk). Output goes to a
//! [`LogSink`]: [`DefaultSink`] prints to stdout/stderr and, once a cache
//! dir is configured, writes `<cache>/Logs/<stamp>_DebugLog.txt` like
//! `LoggerManager`; [`MemorySink`] records everything for tests.
//!
//! # State
//!
//! Java keeps the per-type severities in the enum constants and the rest in
//! the `DebugLog` singleton. Here one [`DebugLog`] value holds all of it.
//! [`with`] gives access to a thread-local instance (Lua and the game loop
//! run on one thread); tests can also build their own [`DebugLog`].
//!
//! # Formatting
//!
//! `debugln(format, params...)` etc. use `String.format`. [`java_format`]
//! implements `java.util.Formatter` for the general, character, integral
//! and floating-point conversions (`%s %S %b %h %c %d %o %x %e %f %g %a %%
//! %n`, flags, width, precision, argument indices) including its exceptions,
//! with the en-US locale. Lua numbers arrive as `java.lang.Double` (Kahlua
//! passes varargs unconverted), so `%d` of a Lua number fails with
//! `IllegalFormatConversionException: d != java.lang.Double` exactly as in
//! the game. Date/time conversions (`%t`) are not supported.

use crate::config::{ConfigFile, ConfigOption};
use pz_kahlua::Value;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// ===========================================================================
// LogSeverity
// ===========================================================================

/// `zombie.debug.LogSeverity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LogSeverity {
    Trace,
    Noise,
    Debug,
    General,
    Warning,
    Error,
    Off,
}

/// Constant names of `LogSeverity`, in ordinal order. This is the one place
/// the Lua exposure takes names from.
pub const LOG_SEVERITY_NAMES: &[&str] = &["Trace", "Noise", "Debug", "General", "Warning", "Error", "Off"];

impl LogSeverity {
    /// `LogSeverity.All`.
    pub const ALL: LogSeverity = LogSeverity::Trace;
    /// `LogSeverity.values()`.
    pub const VALUES: [LogSeverity; 7] = [
        LogSeverity::Trace,
        LogSeverity::Noise,
        LogSeverity::Debug,
        LogSeverity::General,
        LogSeverity::Warning,
        LogSeverity::Error,
        LogSeverity::Off,
    ];

    pub fn name(self) -> &'static str {
        LOG_SEVERITY_NAMES[self as usize]
    }
    pub fn ordinal(self) -> usize {
        self as usize
    }
    pub fn from_ordinal(i: usize) -> Option<Self> {
        Self::VALUES.get(i).copied()
    }
    /// The `logPrefix` field.
    pub fn log_prefix(self) -> &'static str {
        match self {
            LogSeverity::Trace => "TRACE: ",
            LogSeverity::Noise => "NOISE: ",
            LogSeverity::Debug => "DEBUG: ",
            LogSeverity::General => "LOG  : ",
            LogSeverity::Warning => "WARN : ",
            LogSeverity::Error => "ERROR: ",
            LogSeverity::Off => "!OFF!",
        }
    }
    /// `isLogEnabled(logSeverity)`: this level lets `s` through.
    pub fn is_log_enabled(self, s: LogSeverity) -> bool {
        self != LogSeverity::Off && s.ordinal() >= self.ordinal()
    }
    /// `LogSeverity.valueOf(name)` (exact, case-sensitive).
    pub fn value_of(name: &str) -> Option<Self> {
        Self::VALUES.iter().copied().find(|v| v.name() == name)
    }
    /// `isName(str)`: `equalsIgnoreCase`.
    pub fn is_name(self, s: &str) -> bool {
        java_equals_ignore_case(self.name(), s)
    }
}

impl fmt::Display for LogSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ===========================================================================
// DebugType
// ===========================================================================

macro_rules! debug_types {
    ($($name:ident),* $(,)?) => {
        /// `zombie.debug.DebugType`, constants in declaration order.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum DebugType { $($name),* }

        /// Constant names of `DebugType`, in ordinal order. This is the one
        /// place the Lua exposure takes names from.
        pub const DEBUG_TYPE_NAMES: &[&str] = &[$(stringify!($name)),*];

        impl DebugType {
            /// `DebugType.values()`.
            pub const VALUES: &'static [DebugType] = &[$(DebugType::$name),*];
        }
    };
}

debug_types!(
    General, Packet, NetworkFileDebug, Network, ZNet, DetailedInfo, Lua, LuaObject, GameOption, Mod, Sound, Zombie,
    Combat, Objects, Fireplace, Radio, MapLoading, Clothing, Animation, AnimationDetailed, AnimationLayers, Asset,
    Script, Shader, Sprite, Input, Recipe, ActionSystem, ActionSystemEvents, IsoRegion, FileIO, Multiplayer, Damage,
    Death, Discord, Statistic, Vehicle, VehicleHit, Voice, Checksum, Animal, ItemPicker, CraftLogic, Action, Entity,
    Lightning, Grapple, ExitDebug, BodyDamage, Xml, Physics, Ballistics, Ragdoll, PZBullet, ModelManager,
    LoadAnimation, Zone, WorldGen, Foraging, Saving, Fluid, Energy, Translation, Moveable, Basement, FallDamage, ImGui,
    CharacterTrait, ISUI, ISUIStackTrace, FaceLocationFix, Context, ClientCommand, VehicleClientCommand, UnitTest,
    AnimationRecorder,
);

impl DebugType {
    /// `DebugType.Default`.
    pub const DEFAULT: DebugType = DebugType::General;
    pub const COUNT: usize = DEBUG_TYPE_NAMES.len();

    pub fn name(self) -> &'static str {
        DEBUG_TYPE_NAMES[self as usize]
    }
    pub fn ordinal(self) -> usize {
        self as usize
    }
    pub fn from_ordinal(i: usize) -> Option<Self> {
        Self::VALUES.get(i).copied()
    }
    /// `DebugType.valueOf(name)`.
    pub fn value_of(name: &str) -> Option<Self> {
        DEBUG_TYPE_NAMES.iter().position(|n| *n == name).map(|i| Self::VALUES[i])
    }
    /// `valueOf` with the Java exception.
    pub fn value_of_or_throw(name: &str) -> Result<Self, JavaError> {
        Self::value_of(name).ok_or_else(|| no_enum_constant("zombie.debug.DebugType", name))
    }
    /// `isName(rhs)`.
    pub fn is_name(self, s: &str) -> bool {
        java_equals_ignore_case(self.name(), s)
    }
    /// The constructor's `alsoActivate` list.
    pub fn also_activate(self) -> &'static [DebugType] {
        use DebugType::*;
        match self {
            AnimationRecorder => {
                &[General, Animation, ActionSystem, ActionSystemEvents, Death, Vehicle, VehicleHit, Ragdoll, Context]
            }
            _ => &[],
        }
    }
    /// The private `orWith` field: the constructor of each constant sets
    /// `toBind.orWith = this` for its `alsoActivate` list, so the last
    /// constant listing this one wins.
    pub fn or_with(self) -> Option<DebugType> {
        Self::VALUES.iter().rev().copied().find(|t| t.also_activate().contains(&self))
    }

    // ---- convenience wrappers over the thread-local DebugLog ----

    /// `println(String)` on the thread-local log.
    pub fn println(self, s: &str) {
        with(|d| d.println(self, s));
    }
    /// `warn(Object)` on the thread-local log; `caller` is the ` at` affix.
    pub fn warn(self, caller: Option<&StackElement>, s: &str) {
        let _ = with(|d| d.warn(self, caller, s, &[]));
    }
    /// `error(Object)` on the thread-local log.
    pub fn error(self, caller: Option<&StackElement>, s: &str) {
        let _ = with(|d| d.error(self, caller, s, &[]));
    }
    /// `debugln(Object)` on the thread-local log.
    pub fn debugln(self, caller: Option<&StackElement>, s: &str) {
        let _ = with(|d| d.debugln(self, caller, s, &[]));
    }
    /// `isEnabled()` on the thread-local log.
    pub fn is_enabled(self) -> bool {
        with(|d| d.is_enabled(self))
    }
}

impl fmt::Display for DebugType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ===========================================================================
// Java exceptions
// ===========================================================================

/// A Java exception: class name and `getMessage()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaError {
    pub class: String,
    pub message: Option<String>,
}

impl JavaError {
    pub fn new(class: &str, message: impl Into<String>) -> Self {
        JavaError { class: class.to_string(), message: Some(message.into()) }
    }
    pub fn npe(message: &str) -> Self {
        JavaError::new("java.lang.NullPointerException", message)
    }
}

/// `Throwable.toString()`.
impl fmt::Display for JavaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(m) => write!(f, "{}: {}", self.class, m),
            None => f.write_str(&self.class),
        }
    }
}

fn no_enum_constant(class: &str, name: &str) -> JavaError {
    JavaError::new("java.lang.IllegalArgumentException", format!("No enum constant {class}.{name}"))
}

fn index_oob(i: usize, len: usize) -> JavaError {
    JavaError::new("java.lang.ArrayIndexOutOfBoundsException", format!("Index {i} out of bounds for length {len}"))
}

fn io_error(path: &str, e: &std::io::Error) -> JavaError {
    let reason = match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
        _ => e.to_string(),
    };
    JavaError::new("java.io.FileNotFoundException", format!("{path} ({reason})"))
}

// ===========================================================================
// Stack elements and throwables
// ===========================================================================

/// A `java.lang.StackTraceElement`. For Lua frames Kahlua uses the source
/// name `Lua(<mod>)` as class (`Lua(Vanilla)`, `Lua((MOD:Name))`), the
/// function name as method and the `.lua` file name as file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackElement {
    pub class_name: String,
    pub method_name: String,
    pub file_name: Option<String>,
    /// `-1` unknown, `-2` native.
    pub line_number: i32,
}

impl StackElement {
    pub fn new(class_name: &str, method_name: &str) -> Self {
        StackElement { class_name: class_name.into(), method_name: method_name.into(), file_name: None, line_number: -1 }
    }
    pub fn with_location(mut self, file: &str, line: i32) -> Self {
        self.file_name = Some(file.into());
        self.line_number = line;
        self
    }
    /// A Lua frame as `LuaClosure.toStackTraceElement` builds it.
    pub fn lua(mod_name: &str, function_name: &str, file: Option<&str>, line: i32) -> Self {
        StackElement {
            class_name: format!("Lua({mod_name})"),
            method_name: function_name.into(),
            file_name: file.map(str::to_string),
            line_number: line,
        }
    }
    pub fn is_native(&self) -> bool {
        self.line_number == -2
    }
    /// `StackTraceElement.toString()` (without module/class-loader parts).
    pub fn java_to_string(&self) -> String {
        let loc = if self.is_native() {
            "Native Method".to_string()
        } else {
            match (&self.file_name, self.line_number) {
                (None, _) => "Unknown Source".to_string(),
                (Some(f), l) if l >= 0 => format!("{f}:{l}"),
                (Some(f), _) => f.clone(),
            }
        };
        format!("{}.{}({})", self.class_name, self.method_name, loc)
    }
}

/// `LuaClosure.DebugInfo.generateFunctionName`: the prototype name, or for
/// a file's main chunk (no name, or name == file) the file name without
/// directory and extension. `file_name` uses `/` separators.
pub fn lua_function_name(prototype_name: Option<&str>, file_name: &str) -> String {
    let name = prototype_name.unwrap_or(file_name);
    if name != file_name {
        return name.to_string();
    }
    let mut n = name;
    if let Some(i) = n.rfind(std::path::MAIN_SEPARATOR) {
        n = &n[i + 1..];
    }
    if let Some(i) = n.rfind('.') {
        n = &n[..i];
    }
    n.to_string()
}

/// `DebugLogStream.getStackTraceElementString(elements, idx, includeLoc)`.
pub fn stack_trace_element_string(el: Option<&StackElement>, include_location: bool) -> String {
    let Some(e) = el else { return "(UnknownStack)".into() };
    let cls = unqualified_class_name(&e.class_name);
    let comment = if e.is_native() {
        " (Native Method)".to_string()
    } else if include_location {
        format!("({}:{})", e.file_name.as_deref().unwrap_or("null"), e.line_number)
    } else {
        String::new()
    };
    format!("{cls}.{}{comment}", e.method_name)
}

fn unqualified_class_name(c: &str) -> &str {
    match c.rfind('.') {
        Some(i) if i < c.len() - 1 => &c[i + 1..],
        _ => c,
    }
}

/// A `Throwable` passed to `printException`: its `toString()` and stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogThrowable {
    pub description: String,
    pub stack: Vec<StackElement>,
}

impl LogThrowable {
    pub fn new(description: impl Into<String>) -> Self {
        LogThrowable { description: description.into(), stack: Vec::new() }
    }
    /// `DebugLogStream.getTopStackTraceString(ex)`.
    pub fn top_stack_trace_string(&self) -> String {
        match self.stack.first() {
            None => "No Stack Trace Available".into(),
            Some(e) => stack_trace_element_string(Some(e), true),
        }
    }
}

impl From<&JavaError> for LogThrowable {
    fn from(e: &JavaError) -> Self {
        LogThrowable::new(e.to_string())
    }
}

/// `StackTraceContainer.getStackTraceString(elements, indent, start, count)`.
pub fn stack_trace_string(elements: &[StackElement], indent: &str, depth_start: i32, depth_count: i32) -> String {
    let start = depth_start.max(0) as usize;
    let count = if depth_count <= 0 { elements.len() } else { depth_count as usize };
    let mut out = String::new();
    let mut n = 0;
    let mut d = start;
    while n < count && d < elements.len() {
        let s = elements[d].java_to_string();
        d += 1;
        if n > 0 && s.starts_with("zombie.core.profiling.PerformanceProbes$") {
            continue;
        }
        if n > 0 {
            out.push('\n');
        }
        out.push_str(indent);
        out.push_str(&s);
        n += 1;
    }
    out
}

// ===========================================================================
// Java string helpers
// ===========================================================================

fn java_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `StringUtils.leftJustify(text, length)`.
pub fn left_justify(text: &str, length: usize) -> String {
    let n = java_len(text);
    if n >= length {
        return text.to_string();
    }
    format!("{text}{}", " ".repeat(length - n))
}

fn java_equals_ignore_case(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    a.len() == b.len()
        && a.iter().zip(&b).all(|(x, y)| {
            if x == y {
                return true;
            }
            let ux = x.to_uppercase().next().unwrap_or(*x);
            let uy = y.to_uppercase().next().unwrap_or(*y);
            ux == uy || ux.to_lowercase().next() == uy.to_lowercase().next()
        })
}

/// Java regex `\s`.
fn is_java_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `StringUtils.containsWhitespace`: `s.matches("(.*?)\\s(.*?)")`.
fn contains_whitespace(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    chars.iter().enumerate().any(|(i, c)| {
        is_java_ws(*c)
            && !chars[..i].iter().any(|c| is_line_terminator(*c))
            && !chars[i + 1..].iter().any(|c| is_line_terminator(*c))
    })
}

/// `StringUtils.isNullOrWhitespace`.
fn is_null_or_whitespace(s: Option<&str>) -> bool {
    match s {
        None => true,
        Some(s) => s.chars().all(char::is_whitespace),
    }
}

/// `String.trim()`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `String.split(regex)` for a regex matching runs of `is_sep` chars
/// (`plus` = the regex has `+`) or single chars: trailing empty strings
/// removed, a leading empty string kept when the input starts with a match
/// of non-zero width.
fn java_regex_split(s: &str, is_sep: impl Fn(char) -> bool, plus: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_sep = false;
    for c in s.chars() {
        if is_sep(c) {
            if !(plus && prev_sep) {
                out.push(std::mem::take(&mut cur));
            }
            prev_sep = true;
        } else {
            cur.push(c);
            prev_sep = false;
        }
    }
    out.push(cur);
    while out.len() > 1 && out.last().is_some_and(|x| x.is_empty()) {
        out.pop();
    }
    if out.len() == 1 && out[0].is_empty() && !s.is_empty() {
        out.clear();
    }
    out
}

fn split_ws(s: &str) -> Vec<String> {
    java_regex_split(s, is_java_ws, true)
}

/// `String.CASE_INSENSITIVE_ORDER`.
fn case_insensitive_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |c: char| {
        let u = c.to_uppercase().next().unwrap_or(c);
        u.to_lowercase().next().unwrap_or(u)
    };
    a.chars().map(fold).cmp(b.chars().map(fold))
}

// ===========================================================================
// java.util.Formatter
// ===========================================================================

/// An argument to `String.format`, typed like the Java object it stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum FmtArg {
    Null,
    Bool(bool),
    Char(char),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(String),
    /// Any other object: its class name, `toString()` and `hashCode()`.
    Object { class: String, text: String, hash: i32 },
}

impl From<&str> for FmtArg {
    fn from(s: &str) -> Self {
        FmtArg::Str(s.to_string())
    }
}
impl From<String> for FmtArg {
    fn from(s: String) -> Self {
        FmtArg::Str(s)
    }
}
impl From<i32> for FmtArg {
    fn from(v: i32) -> Self {
        FmtArg::Int(v)
    }
}
impl From<i64> for FmtArg {
    fn from(v: i64) -> Self {
        FmtArg::Long(v)
    }
}
impl From<f64> for FmtArg {
    fn from(v: f64) -> Self {
        FmtArg::Double(v)
    }
}
impl From<f32> for FmtArg {
    fn from(v: f32) -> Self {
        FmtArg::Float(v)
    }
}
impl From<bool> for FmtArg {
    fn from(v: bool) -> Self {
        FmtArg::Bool(v)
    }
}

impl FmtArg {
    /// The Java object Kahlua passes for a Lua value in an `Object...`
    /// vararg (no conversion: numbers stay `Double`).
    pub fn from_lua(v: &Value) -> FmtArg {
        match v {
            Value::Nil => FmtArg::Null,
            Value::Bool(b) => FmtArg::Bool(*b),
            Value::Num(n) => FmtArg::Double(*n),
            Value::Str(s) => FmtArg::Str(s.to_string()),
            other => FmtArg::Object {
                class: pz_kahlua::stdlib::java_class_name(other),
                text: pz_kahlua::exec::java_value_of(other),
                hash: pz_kahlua::exec::identity_hash(other) as i32,
            },
        }
    }

    /// `getClass().getName()`.
    pub fn java_class(&self) -> &str {
        match self {
            FmtArg::Null => "null",
            FmtArg::Bool(_) => "java.lang.Boolean",
            FmtArg::Char(_) => "java.lang.Character",
            FmtArg::Int(_) => "java.lang.Integer",
            FmtArg::Long(_) => "java.lang.Long",
            FmtArg::Float(_) => "java.lang.Float",
            FmtArg::Double(_) => "java.lang.Double",
            FmtArg::Str(_) => "java.lang.String",
            FmtArg::Object { class, .. } => class,
        }
    }

    /// `String.valueOf(o)`.
    pub fn to_java_string(&self) -> String {
        match self {
            FmtArg::Null => "null".into(),
            FmtArg::Bool(b) => b.to_string(),
            FmtArg::Char(c) => c.to_string(),
            FmtArg::Int(i) => i.to_string(),
            FmtArg::Long(i) => i.to_string(),
            FmtArg::Float(f) => java_float_to_string(*f),
            FmtArg::Double(d) => pz_kahlua::number::java_double_to_string(*d),
            FmtArg::Str(s) => s.clone(),
            FmtArg::Object { text, .. } => text.clone(),
        }
    }

    fn hash_code(&self) -> Option<i32> {
        Some(match self {
            FmtArg::Null => return None,
            FmtArg::Bool(b) => {
                if *b {
                    1231
                } else {
                    1237
                }
            }
            FmtArg::Char(c) => *c as i32,
            FmtArg::Int(i) => *i,
            FmtArg::Long(l) => (*l ^ ((*l as u64) >> 32) as i64) as i32,
            FmtArg::Float(f) => f.to_bits() as i32,
            FmtArg::Double(d) => {
                let b = if d.is_nan() { 0x7ff8000000000000u64 } else { d.to_bits() };
                (b ^ (b >> 32)) as i32
            }
            FmtArg::Str(s) => s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32)),
            FmtArg::Object { hash, .. } => *hash,
        })
    }
}

/// Shortest round-trip digits: `(digits, decExp)` with value
/// `0.DIGITS × 10^decExp` (FloatingDecimal convention); zero is `("0", 0)`.
fn decimal_digits(v: f64, single: bool) -> (Vec<u8>, i32) {
    if v == 0.0 {
        return (vec![b'0'], 0);
    }
    let s = if single { format!("{:e}", (v.abs()) as f32) } else { format!("{:e}", v.abs()) };
    let (m, e) = s.split_once('e').unwrap();
    let digits: Vec<u8> = m.bytes().filter(|b| *b != b'.').collect();
    (digits, e.parse::<i32>().unwrap() + 1)
}

/// `Float.toString(f)`.
pub fn java_float_to_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f < 0.0 { "-Infinity".into() } else { "Infinity".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let (digits, dexp) = decimal_digits(f as f64, true);
    let digits = String::from_utf8(digits).unwrap();
    let sign = if f < 0.0 { "-" } else { "" };
    let abs = f.abs();
    if (1.0e-3..1.0e7).contains(&abs) {
        let point = dexp;
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
        format!("{sign}{}.{}E{}", &digits[..1], frac, dexp - 1)
    }
}

/// `FormattedFloatingDecimal.applyPrecision`.
fn apply_precision(dec_exp: i32, digits: &mut [u8], prec: i32) -> i32 {
    let n = digits.len() as i32;
    if prec >= n || prec < 0 {
        return dec_exp;
    }
    if prec == 0 {
        if digits[0] >= b'5' {
            digits[0] = b'1';
            for d in &mut digits[1..] {
                *d = b'0';
            }
            return dec_exp + 1;
        }
        for d in digits.iter_mut() {
            *d = b'0';
        }
        return dec_exp;
    }
    let p = prec as usize;
    if digits[p] >= b'5' {
        let mut i = p;
        i -= 1;
        let mut q = digits[i];
        if q == b'9' {
            while q == b'9' && i > 0 {
                i -= 1;
                q = digits[i];
            }
            if q == b'9' {
                digits[0] = b'1';
                for d in &mut digits[1..] {
                    *d = b'0';
                }
                return dec_exp + 1;
            }
        }
        digits[i] = q + 1;
        for d in &mut digits[i + 1..] {
            *d = b'0';
        }
    } else {
        for d in &mut digits[p..] {
            *d = b'0';
        }
    }
    dec_exp
}

/// `FormattedFloatingDecimal.fillDecimal`.
fn fill_decimal(precision: i32, digits: &[u8], exp: i32) -> String {
    let n = digits.len() as i32;
    let ds = |a: i32, b: i32| String::from_utf8(digits[a as usize..b as usize].to_vec()).unwrap();
    if exp > 0 {
        if n < exp {
            format!("{}{}", ds(0, n), "0".repeat((exp - n) as usize))
        } else {
            let t = (n - exp).min(precision);
            if t > 0 {
                format!("{}.{}", ds(0, exp), ds(exp, exp + t))
            } else {
                ds(0, exp)
            }
        }
    } else {
        let zeros = 0.max((-exp).min(precision));
        let t = 0.max(n.min(precision + exp));
        if zeros > 0 {
            format!("0.{}{}", "0".repeat(zeros as usize), ds(0, t))
        } else if t > 0 {
            format!("0.{}", ds(0, t))
        } else {
            "0".into()
        }
    }
}

/// `FormattedFloatingDecimal.fillScientific`: mantissa and exponent
/// (`+05`, `-123`).
fn fill_scientific(precision: i32, digits: &[u8], exp: i32) -> (String, String) {
    let n = digits.len() as i32;
    let t = 0.max((n - 1).min(precision));
    let mut m = String::new();
    m.push(digits[0] as char);
    if t > 0 {
        m.push('.');
        m.push_str(std::str::from_utf8(&digits[1..=t as usize]).unwrap());
    }
    let (sign, e) = if exp <= 0 { ('-', -exp + 1) } else { ('+', exp - 1) };
    let e = if e <= 9 { format!("0{e}") } else { e.to_string() };
    (m, format!("{sign}{e}"))
}

/// `Formatter.addZeros`.
fn add_zeros(m: &mut String, prec: i32) {
    let i = m.find('.');
    let need_dot = i.is_none();
    let out_prec = match i {
        Some(i) => (m.len() - i - 1) as i32,
        None => 0,
    };
    if out_prec >= prec {
        return;
    }
    if need_dot {
        m.push('.');
    }
    m.push_str(&"0".repeat((prec - out_prec) as usize));
}

fn add_dot(m: &mut String) {
    if !m.contains('.') {
        m.push('.');
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Flags {
    minus: bool,
    hash: bool,
    plus: bool,
    space: bool,
    zero: bool,
    group: bool,
    paren: bool,
    prev: bool,
}

impl Flags {
    fn java_string(&self) -> String {
        let mut s = String::new();
        for (on, c) in [
            (self.minus, '-'),
            (self.hash, '#'),
            (self.plus, '+'),
            (self.space, ' '),
            (self.zero, '0'),
            (self.group, ','),
            (self.paren, '('),
            (self.prev, '<'),
        ] {
            if on {
                s.push(c);
            }
        }
        s
    }
}

#[derive(Clone, Debug)]
struct Spec {
    index: Option<usize>,
    flags: Flags,
    width: Option<usize>,
    precision: Option<usize>,
    dt: bool,
    /// Lower-case conversion.
    conv: char,
    upper: bool,
}

impl Spec {
    /// `FormatSpecifier.toString()`.
    fn java_string(&self) -> String {
        let mut s = String::from("%");
        s.push_str(&self.flags.java_string());
        if let Some(i) = self.index {
            s.push_str(&format!("{i}$"));
        }
        if let Some(w) = self.width {
            s.push_str(&w.to_string());
        }
        if let Some(p) = self.precision {
            s.push_str(&format!(".{p}"));
        }
        if self.dt {
            s.push(if self.upper { 'T' } else { 't' });
        }
        s.push(if self.upper { self.conv.to_ascii_uppercase() } else { self.conv });
        s
    }
}

enum Piece {
    Text(String),
    Spec(Spec),
}

fn fmt_err(class: &str, msg: String) -> JavaError {
    JavaError::new(&format!("java.util.{class}"), msg)
}

fn mismatch(flag: char, conv: char) -> JavaError {
    fmt_err("FormatFlagsConversionMismatchException", format!("Conversion = {conv}, Flags = {flag}"))
}

fn check_bad_flags(s: &Spec, bad: &[char]) -> Result<(), JavaError> {
    for &b in bad {
        let on = match b {
            '-' => s.flags.minus,
            '#' => s.flags.hash,
            '+' => s.flags.plus,
            ' ' => s.flags.space,
            '0' => s.flags.zero,
            ',' => s.flags.group,
            '(' => s.flags.paren,
            _ => false,
        };
        if on {
            return Err(mismatch(b, s.conv));
        }
    }
    Ok(())
}

fn check_spec(s: &Spec) -> Result<(), JavaError> {
    let missing_width = || fmt_err("MissingFormatWidthException", s.java_string());
    let bad_precision = |p: usize| fmt_err("IllegalFormatPrecisionException", p.to_string());
    let bad_flags = || fmt_err("IllegalFormatFlagsException", format!("Flags = '{}'", s.flags.java_string()));
    if s.dt {
        if let Some(p) = s.precision {
            return Err(bad_precision(p));
        }
        check_bad_flags(s, &['#', '+', ' ', '0', ',', '('])?;
        if s.width.is_none() && s.flags.minus {
            return Err(missing_width());
        }
        return Ok(());
    }
    match s.conv {
        'b' | 'h' | 's' => {
            if s.conv != 's' && s.flags.hash {
                return Err(mismatch('#', s.conv));
            }
            if s.width.is_none() && s.flags.minus {
                return Err(missing_width());
            }
            check_bad_flags(s, &['+', ' ', '0', ',', '('])
        }
        'c' => {
            if let Some(p) = s.precision {
                return Err(bad_precision(p));
            }
            check_bad_flags(s, &['#', '+', ' ', '0', ',', '('])?;
            if s.width.is_none() && s.flags.minus {
                return Err(missing_width());
            }
            Ok(())
        }
        'd' | 'o' | 'x' | 'e' | 'f' | 'g' | 'a' => {
            let integral = matches!(s.conv, 'd' | 'o' | 'x');
            if integral {
                if let Some(p) = s.precision {
                    return Err(bad_precision(p));
                }
            }
            if s.width.is_none() && (s.flags.minus || s.flags.zero) {
                return Err(missing_width());
            }
            if (s.flags.plus && s.flags.space) || (s.flags.minus && s.flags.zero) {
                return Err(bad_flags());
            }
            match s.conv {
                'd' => check_bad_flags(s, &['#']),
                'o' | 'x' => check_bad_flags(s, &[',']),
                'e' => check_bad_flags(s, &[',']),
                'g' => check_bad_flags(s, &['#']),
                'a' => check_bad_flags(s, &['(', ',']),
                _ => Ok(()),
            }
        }
        '%' => {
            if let Some(p) = s.precision {
                return Err(bad_precision(p));
            }
            let f = s.flags;
            if f.hash || f.plus || f.space || f.zero || f.group || f.paren || f.prev {
                return Err(bad_flags());
            }
            if s.width.is_none() && f.minus {
                return Err(missing_width());
            }
            Ok(())
        }
        'n' => {
            if let Some(w) = s.width {
                return Err(fmt_err("IllegalFormatWidthException", w.to_string()));
            }
            if let Some(p) = s.precision {
                return Err(bad_precision(p));
            }
            if s.flags.java_string().is_empty() {
                Ok(())
            } else {
                Err(bad_flags())
            }
        }
        _ => Ok(()),
    }
}

fn parse_format(fmt: &str) -> Result<Vec<Piece>, JavaError> {
    let unknown = |c: &str| fmt_err("UnknownFormatConversionException", format!("Conversion = '{c}'"));
    let ch: Vec<char> = fmt.chars().collect();
    let n = ch.len();
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < n {
        if ch[i] != '%' {
            text.push(ch[i]);
            i += 1;
            continue;
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut text)));
        }
        i += 1;
        if i >= n {
            return Err(unknown("%"));
        }
        // argument index
        let mut j = i;
        while j < n && ch[j].is_ascii_digit() {
            j += 1;
        }
        let mut index = None;
        if j > i && j < n && ch[j] == '$' {
            index = ch[i..j].iter().collect::<String>().parse::<usize>().ok();
            i = j + 1;
        }
        // flags
        let mut flags = Flags::default();
        while i < n && "-#+ 0,(<".contains(ch[i]) {
            let slot = match ch[i] {
                '-' => &mut flags.minus,
                '#' => &mut flags.hash,
                '+' => &mut flags.plus,
                ' ' => &mut flags.space,
                '0' => &mut flags.zero,
                ',' => &mut flags.group,
                '(' => &mut flags.paren,
                _ => &mut flags.prev,
            };
            if *slot {
                return Err(fmt_err("DuplicateFormatFlagsException", format!("Flags = '{}'", ch[i])));
            }
            *slot = true;
            i += 1;
        }
        // width
        let mut j = i;
        while j < n && ch[j].is_ascii_digit() {
            j += 1;
        }
        let width = if j > i { ch[i..j].iter().collect::<String>().parse::<usize>().ok() } else { None };
        i = j;
        // precision
        let mut precision = None;
        if i < n && ch[i] == '.' {
            let mut j = i + 1;
            while j < n && ch[j].is_ascii_digit() {
                j += 1;
            }
            if j == i + 1 {
                return Err(unknown("."));
            }
            precision = ch[i + 1..j].iter().collect::<String>().parse::<usize>().ok();
            i = j;
        }
        let mut dt = false;
        let mut upper = false;
        if i < n && (ch[i] == 't' || ch[i] == 'T') {
            dt = true;
            upper = ch[i] == 'T';
            i += 1;
        }
        if i >= n {
            return Err(unknown("%"));
        }
        let c = ch[i];
        i += 1;
        let conv;
        if dt {
            if !"HIklMSLNpzZsQBbhAaCYyjmdeRTrDFc".contains(c) {
                return Err(unknown(&format!("{}{c}", if upper { 'T' } else { 't' })));
            }
            conv = c;
        } else {
            match c {
                'b' | 'h' | 's' | 'c' | 'd' | 'o' | 'x' | 'e' | 'f' | 'g' | 'a' | '%' | 'n' => conv = c,
                'B' | 'H' | 'S' | 'C' | 'X' | 'E' | 'G' | 'A' => {
                    conv = c.to_ascii_lowercase();
                    upper = true;
                }
                _ => return Err(unknown(&c.to_string())),
            }
        }
        let spec = Spec { index, flags, width, precision, dt, conv, upper };
        check_spec(&spec)?;
        pieces.push(Piece::Spec(spec));
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

fn justify(s: String, spec: &Spec) -> String {
    match spec.width {
        Some(w) if java_len(&s) < w => {
            let pad = " ".repeat(w - java_len(&s));
            if spec.flags.minus {
                s + &pad
            } else {
                pad + &s
            }
        }
        _ => s,
    }
}

fn conversion_error(spec: &Spec, arg: &FmtArg) -> JavaError {
    fmt_err("IllegalFormatConversionException", format!("{} != {}", spec.conv, arg.java_class()))
}

/// `Formatter.localizedMagnitude` (en-US): grouping and zero padding of
/// `value` (digits with an optional '.') appended to `sb`.
fn localized_magnitude(sb: &mut String, value: &str, flags: &Flags, width: Option<usize>) {
    let begin = sb.len();
    let dot = value.find('.').unwrap_or(value.len());
    let int = &value.as_bytes()[..dot];
    for (j, d) in int.iter().enumerate() {
        sb.push(*d as char);
        if flags.group && j != dot - 1 && (dot - j) % 3 == 1 {
            sb.push(',');
        }
    }
    if dot < value.len() {
        sb.push_str(&value[dot..]);
    }
    if let Some(w) = width {
        if flags.zero {
            while sb.len() < w {
                sb.insert(begin, '0');
            }
        }
    }
}

fn adjust_width(width: Option<usize>, flags: &Flags, neg: bool) -> Option<usize> {
    match width {
        Some(w) if neg && flags.paren => Some(w.saturating_sub(1)),
        w => w,
    }
}

fn leading_sign(sb: &mut String, neg: bool, flags: &Flags) {
    if !neg {
        if flags.plus {
            sb.push('+');
        } else if flags.space {
            sb.push(' ');
        }
    } else if flags.paren {
        sb.push('(');
    } else {
        sb.push('-');
    }
}

fn trailing_sign(sb: &mut String, neg: bool, flags: &Flags) {
    if neg && flags.paren {
        sb.push(')');
    }
}

fn print_integral(spec: &Spec, v: i64, bits: u32) -> Result<String, JavaError> {
    let f = &spec.flags;
    let mut sb = String::new();
    match spec.conv {
        'd' => {
            let neg = v < 0;
            let mag = (v as i128).abs().to_string();
            leading_sign(&mut sb, neg, f);
            localized_magnitude(&mut sb, &mag, f, adjust_width(spec.width, f, neg));
            trailing_sign(&mut sb, neg, f);
        }
        'o' | 'x' => {
            check_bad_flags(spec, &['(', ' ', '+'])?;
            let u = if bits == 32 { v as i32 as u32 as u64 } else { v as u64 };
            let digits = if spec.conv == 'o' { format!("{u:o}") } else { format!("{u:x}") };
            let prefix = if !f.hash {
                ""
            } else if spec.conv == 'o' {
                "0"
            } else {
                "0x"
            };
            sb.push_str(prefix);
            if f.zero {
                if let Some(w) = spec.width {
                    let len = digits.len() + prefix.len();
                    if w > len {
                        sb.push_str(&"0".repeat(w - len));
                    }
                }
            }
            sb.push_str(&digits);
            if spec.upper {
                sb = sb.to_uppercase();
            }
        }
        _ => unreachable!(),
    }
    Ok(justify(sb, spec))
}

/// `Double.toHexString` of a non-negative finite value.
fn double_to_hex(v: f64) -> String {
    if v == 0.0 {
        return "0x0.0p0".into();
    }
    let bits = v.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i64;
    let mant = bits & ((1u64 << 52) - 1);
    let mut m = format!("{mant:013x}");
    while m.len() > 1 && m.ends_with('0') {
        m.pop();
    }
    if exp == 0 {
        format!("0x0.{m}p-1022")
    } else {
        format!("0x1.{m}p{}", exp - 1023)
    }
}

fn print_float(spec: &Spec, value: f64, single: bool) -> String {
    let f = &spec.flags;
    let mut sb = String::new();
    if value.is_nan() {
        sb.push_str(if spec.upper { "NAN" } else { "NaN" });
        return justify(sb, spec);
    }
    let neg = value < 0.0 || (value == 0.0 && value.is_sign_negative());
    let v = value.abs();
    leading_sign(&mut sb, neg, f);
    if v.is_infinite() {
        sb.push_str(if spec.upper { "INFINITY" } else { "Infinity" });
        trailing_sign(&mut sb, neg, f);
        return justify(sb, spec);
    }
    match spec.conv {
        'f' => {
            let prec = spec.precision.map_or(6, |p| p as i32);
            let (mut digits, dexp) = decimal_digits(v, single);
            let exp = apply_precision(dexp, &mut digits, dexp + prec);
            let mut mant = fill_decimal(prec, &digits, exp);
            add_zeros(&mut mant, prec);
            if f.hash && prec == 0 {
                add_dot(&mut mant);
            }
            localized_magnitude(&mut sb, &mant, f, adjust_width(spec.width, f, neg));
        }
        'e' => {
            let prec = spec.precision.map_or(6, |p| p as i32);
            let (mut digits, dexp) = decimal_digits(v, single);
            let exp = apply_precision(dexp, &mut digits, prec + 1);
            let (mut mant, e) = fill_scientific(prec, &digits, exp);
            let e = if v == 0.0 { "+00".to_string() } else { e };
            add_zeros(&mut mant, prec);
            if f.hash && prec == 0 {
                add_dot(&mut mant);
            }
            let w = spec.width.map(|w| w.saturating_sub(e.len() + 1));
            localized_magnitude(&mut sb, &mant, f, adjust_width(w, f, neg));
            sb.push(if spec.upper { 'E' } else { 'e' });
            sb.push_str(&e);
        }
        'g' => {
            let mut prec = match spec.precision {
                None => 6,
                Some(0) => 1,
                Some(p) => p as i32,
            };
            let (mut mant, e, exp_rounded) = if v == 0.0 {
                ("0".to_string(), None, 0)
            } else {
                let (mut digits, dexp) = decimal_digits(v, single);
                let exp = apply_precision(dexp, &mut digits, prec);
                if exp - 1 < -4 || exp - 1 >= prec {
                    let (m, e) = fill_scientific(prec - 1, &digits, exp);
                    (m, Some(e), exp - 1)
                } else {
                    (fill_decimal(prec - exp, &digits, exp), None, exp - 1)
                }
            };
            if e.is_some() {
                prec -= 1;
            } else {
                prec -= exp_rounded + 1;
            }
            add_zeros(&mut mant, prec);
            if f.hash && prec == 0 {
                add_dot(&mut mant);
            }
            let w = match &e {
                Some(e) => spec.width.map(|w| w.saturating_sub(e.len() + 1)),
                None => spec.width,
            };
            localized_magnitude(&mut sb, &mant, f, adjust_width(w, f, neg));
            if let Some(e) = e {
                sb.push(if spec.upper { 'E' } else { 'e' });
                sb.push_str(&e);
            }
        }
        'a' => {
            let mut s = double_to_hex(if single { v as f32 as f64 } else { v });
            if spec.upper {
                s = s.to_uppercase();
            }
            if f.zero {
                if let Some(w) = spec.width {
                    let len = s.len() + sb.len();
                    if w > len {
                        s.insert_str(2, &"0".repeat(w - len));
                    }
                }
            }
            sb.push_str(&s);
        }
        _ => unreachable!(),
    }
    trailing_sign(&mut sb, neg, f);
    justify(sb, spec)
}

fn truncate_precision(s: String, p: Option<usize>) -> String {
    match p {
        Some(p) if p < java_len(&s) => {
            let mut out = String::new();
            let mut n = 0;
            for c in s.chars() {
                let l = c.len_utf16();
                if n + l > p {
                    break;
                }
                out.push(c);
                n += l;
            }
            out
        }
        _ => s,
    }
}

fn print_arg(spec: &Spec, arg: &FmtArg) -> Result<String, JavaError> {
    if spec.dt {
        return match arg {
            FmtArg::Null => Ok(justify("null".into(), spec)),
            FmtArg::Long(_) | FmtArg::Int(_) => Err(JavaError::new(
                "java.lang.UnsupportedOperationException",
                "date/time conversions are not supported by this port",
            )),
            _ => Err(fmt_err("IllegalFormatConversionException", format!("{} != {}", spec.conv, arg.java_class()))),
        };
    }
    let upper = |s: String| if spec.upper { s.to_uppercase() } else { s };
    match spec.conv {
        'b' => {
            let s = match arg {
                FmtArg::Null => "false".to_string(),
                FmtArg::Bool(b) => b.to_string(),
                _ => "true".to_string(),
            };
            Ok(justify(upper(truncate_precision(s, spec.precision)), spec))
        }
        'h' => {
            let s = match arg.hash_code() {
                None => "null".to_string(),
                Some(h) => format!("{:x}", h as u32),
            };
            Ok(justify(upper(truncate_precision(s, spec.precision)), spec))
        }
        's' => {
            if spec.flags.hash && !matches!(arg, FmtArg::Null) {
                return Err(mismatch('#', 's'));
            }
            Ok(justify(upper(truncate_precision(arg.to_java_string(), spec.precision)), spec))
        }
        'c' => {
            let s = match arg {
                FmtArg::Null => "null".to_string(),
                FmtArg::Char(c) => c.to_string(),
                FmtArg::Int(i) => match char::from_u32(*i as u32) {
                    Some(c) => c.to_string(),
                    None => {
                        return Err(fmt_err("IllegalFormatCodePointException", format!("Code point = {:#x}", i)));
                    }
                },
                _ => return Err(conversion_error(spec, arg)),
            };
            Ok(justify(upper(s), spec))
        }
        'd' | 'o' | 'x' => match arg {
            FmtArg::Null => Ok(justify("null".into(), spec)),
            FmtArg::Int(i) => print_integral(spec, *i as i64, 32),
            FmtArg::Long(l) => print_integral(spec, *l, 64),
            _ => Err(conversion_error(spec, arg)),
        },
        'e' | 'f' | 'g' | 'a' => match arg {
            FmtArg::Null => Ok(justify("null".into(), spec)),
            FmtArg::Double(d) => Ok(print_float(spec, *d, false)),
            FmtArg::Float(f) => Ok(print_float(spec, *f as f64, spec.conv == 'a')),
            _ => Err(conversion_error(spec, arg)),
        },
        _ => unreachable!(),
    }
}

/// `String.format(format, args)` (en-US locale).
pub fn java_format(format: &str, args: &[FmtArg]) -> Result<String, JavaError> {
    let pieces = parse_format(format)?;
    let mut out = String::new();
    let mut ordinary = 0usize;
    let mut last: Option<usize> = None;
    for p in &pieces {
        match p {
            Piece::Text(t) => out.push_str(t),
            Piece::Spec(s) => match s.conv {
                '%' if !s.dt => out.push_str(&justify("%".into(), s)),
                'n' if !s.dt => out.push('\n'),
                _ => {
                    let missing = || fmt_err("MissingFormatArgumentException", format!("Format specifier '{}'", s.java_string()));
                    let idx = if s.flags.prev {
                        last.ok_or_else(missing)?
                    } else if let Some(k) = s.index.filter(|k| *k > 0) {
                        k - 1
                    } else {
                        ordinary += 1;
                        ordinary - 1
                    };
                    let arg = args.get(idx).ok_or_else(missing)?;
                    last = Some(idx);
                    out.push_str(&print_arg(s, arg)?);
                }
            },
        }
    }
    Ok(out)
}

/// `DebugLogStream.getFormattedOutputStr(format, params)`.
pub fn formatted_output_str(format: &str, params: &[FmtArg]) -> Result<String, JavaError> {
    if params.is_empty() {
        Ok(format.to_string())
    } else {
        java_format(format, params)
    }
}

// ===========================================================================
// Clock and local time
// ===========================================================================

/// Time inputs of the logger.
pub trait LogClock {
    /// `System.currentTimeMillis()`.
    fn now_millis(&self) -> i64;
    /// Offset of the default time zone from UTC at `epoch_ms`.
    fn local_offset_millis(&self, epoch_ms: i64) -> i64;
    /// `GameTime.getServerTime()` in nanoseconds (for ` st:`).
    fn server_time_nanos(&self) -> i64 {
        0
    }
}

/// Wall clock and the JVM default zone (`TZ`, else `/etc/localtime`).
pub struct SystemClock {
    zone: Option<TzInfo>,
    /// `GameTime.getServerTime()` source, set by the engine.
    pub server_time: Option<Box<dyn Fn() -> i64>>,
}

impl SystemClock {
    pub fn new() -> Self {
        SystemClock { zone: load_local_zone(), server_time: None }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl LogClock for SystemClock {
    fn now_millis(&self) -> i64 {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_millis() as i64,
            Err(e) => -(e.duration().as_millis() as i64),
        }
    }
    fn local_offset_millis(&self, epoch_ms: i64) -> i64 {
        self.zone.as_ref().map_or(0, |z| z.offset_secs(epoch_ms.div_euclid(1000)) * 1000)
    }
    fn server_time_nanos(&self) -> i64 {
        self.server_time.as_ref().map_or(0, |f| f())
    }
}

/// A settable clock for deterministic output.
#[derive(Default)]
pub struct FixedClock {
    pub now_ms: Cell<i64>,
    pub offset_ms: Cell<i64>,
    pub server_nanos: Cell<i64>,
}

impl FixedClock {
    pub fn new(now_ms: i64, offset_ms: i64) -> Self {
        FixedClock { now_ms: Cell::new(now_ms), offset_ms: Cell::new(offset_ms), server_nanos: Cell::new(0) }
    }
}

impl LogClock for FixedClock {
    fn now_millis(&self) -> i64 {
        self.now_ms.get()
    }
    fn local_offset_millis(&self, _: i64) -> i64 {
        self.offset_ms.get()
    }
    fn server_time_nanos(&self) -> i64 {
        self.server_nanos.get()
    }
}

struct TzInfo {
    transitions: Vec<i64>,
    type_idx: Vec<u8>,
    offsets: Vec<i64>,
    isdst: Vec<bool>,
}

impl TzInfo {
    fn offset_secs(&self, t: i64) -> i64 {
        if self.offsets.is_empty() {
            return 0;
        }
        match self.transitions.partition_point(|&x| x <= t) {
            0 => {
                let i = self.isdst.iter().position(|d| !d).unwrap_or(0);
                self.offsets[i]
            }
            k => self.offsets.get(self.type_idx[k - 1] as usize).copied().unwrap_or(0),
        }
    }

    fn parse(data: &[u8]) -> Option<TzInfo> {
        fn be32(b: &[u8], i: usize) -> Option<i64> {
            Some(i32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?) as i64)
        }
        fn be64(b: &[u8], i: usize) -> Option<i64> {
            Some(i64::from_be_bytes(b.get(i..i + 8)?.try_into().ok()?))
        }
        fn counts(b: &[u8], at: usize) -> Option<[usize; 6]> {
            if b.get(at..at + 4)? != b"TZif" {
                return None;
            }
            let mut c = [0usize; 6];
            for (k, v) in c.iter_mut().enumerate() {
                *v = be32(b, at + 20 + 4 * k)? as usize;
            }
            Some(c)
        }
        let c = counts(data, 0)?;
        let [isut, isstd, leap, timecnt, typecnt, charcnt] = c;
        let v1_len = timecnt * 5 + typecnt * 6 + charcnt + leap * 8 + isstd + isut;
        let (base, tsize, c) = if data.get(4).is_some_and(|v| *v >= b'2') {
            let at = 44 + v1_len;
            (at + 44, 8, counts(data, at)?)
        } else {
            (44, 4, c)
        };
        let [_, _, _, timecnt, typecnt, _] = c;
        let mut transitions = Vec::with_capacity(timecnt);
        for k in 0..timecnt {
            let i = base + k * tsize;
            transitions.push(if tsize == 8 { be64(data, i)? } else { be32(data, i)? });
        }
        let ti = base + timecnt * tsize;
        let type_idx = data.get(ti..ti + timecnt)?.to_vec();
        let tt = ti + timecnt;
        let mut offsets = Vec::new();
        let mut isdst = Vec::new();
        for k in 0..typecnt {
            offsets.push(be32(data, tt + k * 6)?);
            isdst.push(*data.get(tt + k * 6 + 4)? != 0);
        }
        Some(TzInfo { transitions, type_idx, offsets, isdst })
    }
}

fn load_local_zone() -> Option<TzInfo> {
    let read = |p: &str| std::fs::read(p).ok().and_then(|d| TzInfo::parse(&d));
    match std::env::var("TZ") {
        Ok(tz) => {
            let tz = tz.strip_prefix(':').unwrap_or(&tz).to_string();
            if tz.is_empty() || tz == "UTC" || tz.contains("..") {
                return None;
            }
            if tz.starts_with('/') {
                read(&tz)
            } else {
                read(&format!("/usr/share/zoneinfo/{tz}"))
            }
        }
        Err(_) => read("/etc/localtime"),
    }
}

/// Local calendar fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub milli: u32,
}

impl LocalTime {
    pub fn from_epoch(epoch_ms: i64, offset_ms: i64) -> Self {
        let t = epoch_ms + offset_ms;
        let days = t.div_euclid(86_400_000);
        let ms = t.rem_euclid(86_400_000);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = yoe + era * 400 + i64::from(month <= 2);
        LocalTime {
            year,
            month,
            day,
            hour: (ms / 3_600_000) as u32,
            minute: (ms / 60_000 % 60) as u32,
            second: (ms / 1000 % 60) as u32,
            milli: (ms % 1000) as u32,
        }
    }
    /// `SimpleDateFormat("dd-MM-yy HH:mm:ss.SSS")` (ZLogger).
    pub fn zlogger_stamp(&self) -> String {
        format!(
            "{:02}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            self.day,
            self.month,
            self.year.rem_euclid(100),
            self.hour,
            self.minute,
            self.second,
            self.milli
        )
    }
    /// `SimpleDateFormat("yyyy-MM-dd_HH-mm")` (log file names).
    pub fn date_time_stamp(&self) -> String {
        format!("{:04}-{:02}-{:02}_{:02}-{:02}", self.year, self.month, self.day, self.hour, self.minute)
    }
}

fn local_now(clock: &dyn LogClock) -> LocalTime {
    let now = clock.now_millis();
    LocalTime::from_epoch(now, clock.local_offset_millis(now))
}

/// `NumberFormat.getNumberInstance().format(long)` (en-US).
pub fn group_number(v: i64) -> String {
    let digits = (v as i128).abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if v < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// `ZLogger.writeUnsafe` line (without the newline `println` adds).
pub fn zlogger_line(stamp: &str, logs: &str, prefix: Option<&str>, append: bool) -> Result<String, JavaError> {
    let mut sb = String::new();
    if !append {
        sb.push('[');
        sb.push_str(stamp);
        sb.push(']');
    }
    if let Some(p) = prefix.filter(|p| !p.is_empty()) {
        sb.push('[');
        sb.push_str(p);
        sb.push(']');
    }
    // `logs.lastIndexOf('\n') == logs.length() - 1` is also true for "".
    let body = if logs.is_empty() {
        return Err(JavaError::new("java.lang.StringIndexOutOfBoundsException", "start 0, end -1, length 0"));
    } else if let Some(b) = logs.strip_suffix('\n') {
        b
    } else {
        logs
    };
    if !append {
        sb.push(' ');
        sb.push_str(body);
        sb.push('.');
    } else {
        sb.push_str(body);
    }
    Ok(sb)
}

/// `FileName.sanitize(name)`.
pub fn sanitize_file_name(name: &str) -> String {
    let mut s: String =
        name.chars().map(|c| if "\\/:*?\"<>|".contains(c) || (c as u32) < 0x20 { '_' } else { c }).collect();
    while s.ends_with(|c: char| c == '.' || is_java_ws(c)) {
        s.pop();
    }
    if s.is_empty() {
        s = "_".into();
    }
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = s.split('.').next().unwrap_or("").to_uppercase();
    if RESERVED.contains(&base.as_str()) {
        s = format!("_{s}");
    }
    s
}

// ===========================================================================
// Sinks and LoggerManager
// ===========================================================================

/// The two process streams the original writes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdStream {
    Out,
    Err,
}

/// Where log output goes.
pub trait LogSink {
    /// Text printed on a console stream (`print` or `println`, so it
    /// includes the newline where Java prints one).
    fn console(&mut self, stream: StdStream, text: &str);
    /// One line for the log file of `logger` (`ZLogger.writeUnsafe`
    /// output, without the trailing newline).
    fn file(&mut self, logger: &str, line: &str);
    /// One line of the animation recording (`recordingOut.println`).
    fn recording(&mut self, _line: &str) {}
}

struct ZLogFile {
    file: Option<std::fs::File>,
    written: u64,
}

/// `zombie.core.logger.LoggerManager`: `<cache>/Logs/<stamp>_<name>.txt`
/// files, old files moved to `logs_<stamp>` on first use, 10 000 KB
/// truncation per file.
pub struct LoggerManager {
    pub logs_dir: PathBuf,
    pub use_time_stamped_log_file_names: bool,
    debug_log_file_prefix: String,
    startup_time_stamp: String,
    initialized: bool,
    loggers: HashMap<String, ZLogFile>,
    clock: Rc<dyn LogClock>,
}

impl LoggerManager {
    pub fn new(cache_dir: &Path, clock: Rc<dyn LogClock>) -> Self {
        let startup_time_stamp = local_now(&*clock).date_time_stamp();
        LoggerManager {
            logs_dir: cache_dir.join("Logs"),
            use_time_stamped_log_file_names: true,
            debug_log_file_prefix: String::new(),
            startup_time_stamp,
            initialized: false,
            loggers: HashMap::new(),
            clock,
        }
    }

    /// `ZomboidFileSystem.getStartupTimeStamp()`.
    pub fn startup_time_stamp(&self) -> &str {
        &self.startup_time_stamp
    }

    /// `getLogFilePath(loggerName)`.
    pub fn log_file_path(&self, logger: &str) -> PathBuf {
        let mut name = logger.to_string();
        if !self.debug_log_file_prefix.is_empty() {
            name = format!("{}_{name}", self.debug_log_file_prefix);
        }
        if self.use_time_stamped_log_file_names {
            name = format!("{}_{name}", self.startup_time_stamp);
        }
        self.logs_dir.join(format!("{name}.txt"))
    }

    /// `LoggerManager.init()`: move the previous session's files into
    /// `Logs/logs_<last modified of the first file>`.
    fn init(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        let _ = std::fs::create_dir_all(&self.logs_dir);
        let Ok(rd) = std::fs::read_dir(&self.logs_dir) else { return };
        let entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        let Some(first) = entries.first() else { return };
        let mtime = std::fs::metadata(first)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64);
        let stamp = LocalTime::from_epoch(mtime, self.clock.local_offset_millis(mtime)).date_time_stamp();
        let backup = self.logs_dir.join(format!("logs_{stamp}"));
        let _ = std::fs::create_dir_all(&backup);
        for p in entries.iter().filter(|p| p.is_file()) {
            if let Some(n) = p.file_name() {
                let _ = std::fs::rename(p, backup.join(n));
            }
        }
    }

    fn open(&mut self, logger: &str) -> ZLogFile {
        self.init();
        let _ = std::fs::create_dir_all(&self.logs_dir);
        ZLogFile { file: std::fs::File::create(self.log_file_path(logger)).ok(), written: 0 }
    }

    /// `ZLogger.OutputStreams.println` + `checkSize`.
    pub fn write_line(&mut self, logger: &str, line: &str) {
        if !self.loggers.contains_key(logger) {
            let f = self.open(logger);
            self.loggers.insert(logger.to_string(), f);
        }
        let mut reopen = false;
        if let Some(z) = self.loggers.get_mut(logger) {
            if let Some(f) = z.file.as_mut() {
                let bytes = format!("{line}\n");
                if f.write_all(bytes.as_bytes()).is_ok() {
                    let _ = f.flush();
                    z.written += bytes.len() as u64;
                }
            }
            reopen = z.written / 1024 > 10000;
        }
        if reopen {
            let f = self.open(logger);
            self.loggers.insert(logger.to_string(), f);
        }
    }

    /// `setLogFilePrefixRaw`: recreates every open log file.
    pub fn set_log_file_prefix_raw(&mut self, raw: Option<&str>) {
        let prefix = match raw.map(java_trim).filter(|s| !s.is_empty()) {
            Some(s) => sanitize_file_name(s),
            None => String::new(),
        };
        if prefix == self.debug_log_file_prefix {
            return;
        }
        self.debug_log_file_prefix = prefix;
        self.recreate_all();
    }

    /// `setUseTimeStampedLogFileNames`.
    pub fn set_use_time_stamped_log_file_names(&mut self, b: bool) {
        if self.use_time_stamped_log_file_names == b {
            return;
        }
        self.use_time_stamped_log_file_names = b;
        self.recreate_all();
    }

    fn recreate_all(&mut self) {
        let names: Vec<String> = self.loggers.keys().cloned().collect();
        for n in names {
            let f = self.open(&n);
            self.loggers.insert(n, f);
        }
    }
}

/// stdout/stderr plus, when a cache dir is configured, the log files.
pub struct DefaultSink {
    pub loggers: Option<LoggerManager>,
}

impl LogSink for DefaultSink {
    fn console(&mut self, stream: StdStream, text: &str) {
        match stream {
            StdStream::Out => {
                let mut o = std::io::stdout().lock();
                let _ = o.write_all(text.as_bytes());
                let _ = o.flush();
            }
            StdStream::Err => {
                let _ = std::io::stderr().lock().write_all(text.as_bytes());
            }
        }
    }
    fn file(&mut self, logger: &str, line: &str) {
        if let Some(l) = self.loggers.as_mut() {
            l.write_line(logger, line);
        }
    }
}

/// Everything a [`MemorySink`] received.
#[derive(Default, Debug)]
pub struct MemoryLog {
    pub console: Vec<(StdStream, String)>,
    pub files: Vec<(String, String)>,
    pub recording: Vec<String>,
}

impl MemoryLog {
    pub fn stdout(&self) -> String {
        self.console.iter().filter(|(s, _)| *s == StdStream::Out).map(|(_, t)| t.as_str()).collect()
    }
    pub fn stderr(&self) -> String {
        self.console.iter().filter(|(s, _)| *s == StdStream::Err).map(|(_, t)| t.as_str()).collect()
    }
    /// All console output in order.
    pub fn all_console(&self) -> String {
        self.console.iter().map(|(_, t)| t.as_str()).collect()
    }
    pub fn file_lines(&self, logger: &str) -> Vec<String> {
        self.files.iter().filter(|(n, _)| n == logger).map(|(_, l)| l.clone()).collect()
    }
}

/// In-memory sink; clones share the record.
#[derive(Clone, Default)]
pub struct MemorySink(pub Rc<RefCell<MemoryLog>>);

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn log(&self) -> std::cell::Ref<'_, MemoryLog> {
        self.0.borrow()
    }
    pub fn clear(&self) {
        *self.0.borrow_mut() = MemoryLog::default();
    }
}

impl LogSink for MemorySink {
    fn console(&mut self, stream: StdStream, text: &str) {
        self.0.borrow_mut().console.push((stream, text.to_string()));
    }
    fn file(&mut self, logger: &str, line: &str) {
        self.0.borrow_mut().files.push((logger.to_string(), line.to_string()));
    }
    fn recording(&mut self, line: &str) {
        self.0.borrow_mut().recording.push(line.to_string());
    }
}

// ===========================================================================
// DebugLogProfile / DebugLogCfgFile
// ===========================================================================

/// `zombie.debug.DebugLogProfile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugLogProfile {
    pub name: String,
    pub commands: Vec<String>,
}

impl DebugLogProfile {
    pub fn new(name: &str) -> Self {
        DebugLogProfile { name: name.into(), commands: Vec::new() }
    }

    pub fn add_command(&mut self, c: &str) -> &mut Self {
        self.commands.push(c.into());
        self
    }

    /// `getCommandArgument0`.
    pub fn command_argument0(command: &str) -> String {
        let t = java_trim(command);
        if contains_whitespace(t) {
            split_ws(t).into_iter().next().unwrap_or_default()
        } else {
            t.to_string()
        }
    }

    pub fn remove_commands_with_argument0(&mut self, arg: &str) {
        self.commands.retain(|c| Self::command_argument0(c) != arg);
    }

    pub fn remove_debug_type_commands(&mut self) {
        let mut set: HashSet<String> = ["+all".to_string(), "-all".to_string()].into_iter().collect();
        for n in DEBUG_TYPE_NAMES {
            set.insert(format!("+{n}"));
            set.insert(format!("-{n}"));
        }
        self.commands.retain(|c| !set.contains(&Self::command_argument0(c)));
    }

    /// `updateAll(logSeverity)` (`None` = Java `null`).
    pub fn update_all(&mut self, sev: Option<LogSeverity>) {
        self.remove_debug_type_commands();
        match sev {
            Some(LogSeverity::Off) => {
                self.add_command("-all");
            }
            Some(s) => {
                self.add_command(&format!("+all {s}"));
            }
            None => {}
        }
    }

    /// `update(debugType, logSeverity)`.
    pub fn update(&mut self, t: DebugType, sev: Option<LogSeverity>) {
        let en = format!("+{t}");
        let dis = format!("-{t}");
        self.remove_commands_with_argument0(&en);
        self.remove_commands_with_argument0(&dis);
        match sev {
            Some(LogSeverity::Off) => {
                self.add_command(&dis);
            }
            Some(s) => {
                self.add_command(&format!("+{t} {s}"));
            }
            None => {}
        }
    }

    /// `getLogSeverity(debugType)`.
    pub fn log_severity(&self, t: DebugType) -> Option<LogSeverity> {
        let en = format!("+{t}");
        let dis = format!("-{t}");
        let mut sev = None;
        for c in &self.commands {
            let a0 = Self::command_argument0(c);
            if a0.eq_ignore_ascii_case("-all") || a0 == dis {
                sev = Some(LogSeverity::Off);
                continue;
            }
            if a0 != "+all" && a0 != en {
                continue;
            }
            let cmd = java_trim(c);
            let s = if contains_whitespace(cmd) { split_ws(cmd).get(1).map(|s| java_trim(s).to_string()) } else { None };
            sev = Some(s.and_then(|s| LogSeverity::value_of(&s)).unwrap_or(LogSeverity::Debug));
        }
        sev
    }

    /// `write(bw)`.
    pub fn write_to(&self, out: &mut String) {
        out.push_str(&self.name);
        out.push_str("\n{\n");
        for c in &self.commands {
            out.push_str("    ");
            out.push_str(c);
            out.push('\n');
        }
        out.push_str("}\n");
    }
}

/// `zombie.debug.DebugLogCfgFile`.
#[derive(Clone, Debug, Default)]
pub struct DebugLogCfgFile {
    pub profiles: Vec<DebugLogProfile>,
    pub aliases: Vec<String>,
    pub alias_map: HashMap<String, String>,
    pub selected_profile: Option<String>,
}

impl DebugLogCfgFile {
    pub fn clear(&mut self) {
        *self = DebugLogCfgFile::default();
    }

    /// `createProfile` (a later profile of the same name shadows earlier
    /// ones in `getProfile`, like the `HashMap.put`).
    pub fn create_profile(&mut self, name: &str) -> &mut DebugLogProfile {
        self.profiles.push(DebugLogProfile::new(name));
        self.profiles.last_mut().unwrap()
    }

    pub fn profile(&self, name: &str) -> Option<&DebugLogProfile> {
        self.profiles.iter().rev().find(|p| p.name == name)
    }

    pub fn profile_mut(&mut self, name: &str) -> Option<&mut DebugLogProfile> {
        self.profiles.iter_mut().rev().find(|p| p.name == name)
    }

    /// `setDefault()` from the current severities.
    pub fn set_default(&mut self, severities: &[LogSeverity]) {
        self.clear();
        let p = self.create_profile("default");
        p.add_command("-all");
        for t in DebugType::VALUES {
            let s = severities[t.ordinal()];
            if s != LogSeverity::Off {
                p.add_command(&format!("+{} {}", t.name(), s.name()));
            }
        }
        self.selected_profile = Some("default".into());
    }

    /// `readInternal` over the file text. Bad alias lines produce the
    /// exceptions the original prints (returned here).
    pub fn read_text(&mut self, text: &str) -> Vec<JavaError> {
        self.clear();
        let mut errors = Vec::new();
        let mut opened = false;
        let mut line: Option<String> = None;
        let mut current: Option<usize> = None;
        for l in crate::config::java_lines(text) {
            let last_line = line.take();
            let ln = java_trim(l).to_string();
            line = Some(ln.clone());
            if ln.starts_with("//") || ln.starts_with('#') || is_null_or_whitespace(Some(&ln)) {
                continue;
            }
            if let Some(rest) = ln.strip_prefix('=') {
                self.selected_profile = Some(java_trim(rest).to_string());
                continue;
            }
            if let Some(rest) = ln.strip_prefix('$') {
                let s = java_trim(rest);
                match s.find('=') {
                    Some(i) => {
                        let alias = java_trim(&s[..i]).to_string();
                        let command = java_trim(&s[i + 1..]).to_string();
                        self.aliases.push(alias.clone());
                        self.alias_map.insert(alias, command);
                    }
                    None => errors.push(JavaError::new(
                        "java.lang.StringIndexOutOfBoundsException",
                        format!("begin 0, end -1, length {}", java_len(s)),
                    )),
                }
                continue;
            }
            if !opened && ln.starts_with('{') {
                if let Some(name) = last_line {
                    opened = true;
                    self.create_profile(&name);
                    current = Some(self.profiles.len() - 1);
                    continue;
                }
            }
            if !opened {
                continue;
            }
            if ln.starts_with('}') {
                opened = false;
                continue;
            }
            if let Some(i) = current {
                self.profiles[i].add_command(&ln);
            }
        }
        errors
    }

    /// `write(filePath)` text.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for p in &self.profiles {
            p.write_to(&mut out);
            out.push('\n');
        }
        for a in &self.aliases {
            out.push_str(&format!("${a} = {}\n", self.alias_map.get(a).map_or("null", |s| s.as_str())));
        }
        if let Some(s) = &self.selected_profile {
            if !self.aliases.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("={s}\n"));
        }
        out
    }

    /// `resolveAliasProfiles(alias)` (alias includes the leading `$`):
    /// indices of the named profiles that exist.
    pub fn resolve_alias_profiles(&self, alias: &str) -> Result<Vec<String>, JavaError> {
        let key: String = alias.chars().skip(1).collect();
        let cmd = self
            .alias_map
            .get(&key)
            .ok_or_else(|| JavaError::npe("Cannot invoke \"String.split(String)\" because \"aliasCommand\" is null"))?;
        Ok(java_regex_split(cmd, |c| c == '+', false)
            .into_iter()
            .map(|e| java_trim(&e).to_string())
            .filter(|n| self.profile(n).is_some())
            .collect())
    }
}

// ===========================================================================
// DebugOptions
// ===========================================================================

/// One `BooleanDebugOption`.
#[derive(Clone, Debug)]
pub struct DebugOptionDef {
    pub name: String,
    pub default_value: bool,
    pub debug_only: bool,
}

/// `zombie.debug.DebugOptions`: the flat list of boolean options in tree
/// order, persisted in `debug-options.ini`.
///
/// The option tree is game data, not engine logic, so it is not compiled
/// in: install it with [`DebugOptions::set_definitions`] (e.g. from the
/// game classes) or [`DebugOptions::load_options_list_xml`] (the
/// `messaging/DebugOptions_list.xml` the game writes from
/// `initMessaging`, which lists every option in order with its value).
#[derive(Default)]
pub struct DebugOptions {
    pub options: Vec<(DebugOptionDef, ConfigOption)>,
    pub cache_dir: Option<PathBuf>,
    /// `Core.debug` (debug-only options read their default otherwise).
    pub core_debug: bool,
}

impl DebugOptions {
    pub fn set_definitions(&mut self, defs: Vec<DebugOptionDef>) {
        self.options = defs
            .into_iter()
            .map(|d| {
                let o = ConfigOption::boolean(&d.name, d.default_value);
                (d, o)
            })
            .collect();
    }

    /// Definitions from `DebugOptions_list.xml` text: names in order, the
    /// listed value as default. The file does not say which options are
    /// debug-only, so none are.
    pub fn load_options_list_xml(&mut self, xml: &str) {
        let mut defs = Vec::new();
        let mut rest = xml;
        while let Some(i) = rest.find("<options>") {
            rest = &rest[i + 9..];
            let end = rest.find("</options>").unwrap_or(rest.len());
            let block = &rest[..end];
            let tag = |t: &str| {
                let open = format!("<{t}>");
                let close = format!("</{t}>");
                let a = block.find(&open)? + open.len();
                let b = block[a..].find(&close)? + a;
                Some(xml_unescape(&block[a..b]))
            };
            if let Some(name) = tag("name") {
                if !name.is_empty() && !name.contains('=') {
                    defs.push(DebugOptionDef {
                        name,
                        default_value: tag("value").as_deref() == Some("true"),
                        debug_only: false,
                    });
                }
            }
            rest = &rest[end..];
        }
        self.set_definitions(defs);
    }

    /// The `DebugOptions_list.xml` text `initMessaging` writes.
    pub fn options_list_xml(&self) -> String {
        let mut s = String::from("<debugOptionsXml>\n");
        for i in 0..self.options.len() {
            s.push_str(&format!(
                "    <options>\n        <name>{}</name>\n        <value>{}</value>\n    </options>\n",
                xml_escape(&self.options[i].0.name),
                self.get_value(i)
            ));
        }
        s.push_str("</debugOptionsXml>\n");
        s
    }

    pub fn option_count(&self) -> usize {
        self.options.len()
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.options.iter().position(|(d, _)| d.name == name)
    }

    /// `BooleanDebugOption.getValue()`.
    pub fn get_value(&self, i: usize) -> bool {
        let (d, o) = &self.options[i];
        if self.core_debug || !d.debug_only {
            o.get_bool()
        } else {
            d.default_value
        }
    }

    pub fn set_value(&mut self, i: usize, v: bool) {
        self.options[i].1.set_bool(v);
    }

    /// `getBoolean(name)`.
    pub fn get_boolean(&self, name: &str) -> bool {
        self.index_of(name).is_some_and(|i| self.get_value(i))
    }

    /// `setBoolean(name, value)`.
    pub fn set_boolean(&mut self, name: &str, v: bool) {
        if let Some(i) = self.index_of(name) {
            self.set_value(i, v);
        }
    }

    fn ini_path(&self) -> Option<PathBuf> {
        self.cache_dir.as_ref().map(|d| d.join("debug-options.ini"))
    }

    /// `load()`.
    pub fn load(&mut self) {
        let Some(path) = self.ini_path() else { return };
        let mut cf = ConfigFile::new();
        if cf.read(&path) {
            for o in cf.get_options() {
                if let Some(i) = self.index_of(o.name()) {
                    self.options[i].1.parse(&o.get_value_as_string());
                }
            }
        }
    }

    /// `save()`.
    pub fn save(&self) {
        let Some(path) = self.ini_path() else { return };
        let opts: Vec<ConfigOption> = (0..self.options.len())
            .map(|i| {
                let mut o = self.options[i].1.clone();
                o.set_bool(self.get_value(i));
                o
            })
            .collect();
        let refs: Vec<&ConfigOption> = opts.iter().collect();
        ConfigFile::new().write(&path, 1, &refs);
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn xml_unescape(s: &str) -> String {
    s.trim().replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// ===========================================================================
// DebugLog
// ===========================================================================

fn caller_of(class: &str, method: &str) -> StackElement {
    StackElement::new(class, method)
}

/// `zombie.debug.DebugLog` plus the per-type `DebugType`/`DebugLogStream`
/// state.
pub struct DebugLog {
    severities: [LogSeverity; DebugType::COUNT],
    /// `DebugLog.GeneralErr` (the `System.err` replacement) severity.
    pub general_err_severity: LogSeverity,
    /// `Core.debug`.
    pub core_debug: bool,
    /// `GameServer.server`.
    pub is_server: bool,
    /// `GameClient.client`.
    pub is_client: bool,
    /// `IsoWorld.instance.getFrameNo()`.
    pub frame_no: i64,
    initialized: bool,
    log_trace_file_location_enabled: bool,
    log_time_ms_enabled: bool,
    log_server_time_ms_enabled: bool,
    pub cfg: DebugLogCfgFile,
    debug_once: HashMap<Option<DebugType>, HashSet<String>>,
    clock: Rc<dyn LogClock>,
    sink: Box<dyn LogSink>,
    default_sink: bool,
    cache_dir: Option<PathBuf>,
    debug_cfg_file_watcher_path: Option<String>,
    /// `recordingOut != null`.
    pub recording: bool,
    /// `ScriptManager.EnableDebug(scriptType, enable)` requests from
    /// `+Type.ScriptType` config commands.
    pub script_debug_requests: Vec<(String, bool)>,
    /// `ExceptionLogger.exceptionCount` (the number on the error popup).
    pub exception_count: i32,
}

impl Default for DebugLog {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugLog {
    /// A logger before `init()`: every type `Off`, stdout/stderr output.
    pub fn new() -> Self {
        DebugLog {
            severities: [LogSeverity::Off; DebugType::COUNT],
            general_err_severity: LogSeverity::ALL,
            core_debug: false,
            is_server: false,
            is_client: false,
            frame_no: 0,
            initialized: false,
            log_trace_file_location_enabled: false,
            log_time_ms_enabled: false,
            log_server_time_ms_enabled: false,
            cfg: DebugLogCfgFile::default(),
            debug_once: HashMap::new(),
            clock: Rc::new(SystemClock::new()),
            sink: Box::new(DefaultSink { loggers: None }),
            default_sink: true,
            cache_dir: None,
            debug_cfg_file_watcher_path: None,
            recording: false,
            script_debug_requests: Vec::new(),
            exception_count: 0,
        }
    }

    /// A logger with a custom sink and clock (tests).
    pub fn with_sink(sink: impl LogSink + 'static, clock: Rc<dyn LogClock>) -> Self {
        let mut d = DebugLog::new();
        d.clock = clock;
        d.set_sink(sink);
        d
    }

    pub fn set_sink(&mut self, sink: impl LogSink + 'static) {
        self.sink = Box::new(sink);
        self.default_sink = false;
    }

    pub fn set_clock(&mut self, clock: Rc<dyn LogClock>) {
        self.clock = clock;
        if self.default_sink {
            self.rebuild_default_sink();
        }
    }

    pub fn clock(&self) -> Rc<dyn LogClock> {
        self.clock.clone()
    }

    /// `ZomboidFileSystem.getCacheDir()`: enables the log files of the
    /// default sink and locates `debuglog.cfg`.
    pub fn set_cache_dir(&mut self, dir: Option<PathBuf>) {
        self.cache_dir = dir;
        if self.default_sink {
            self.rebuild_default_sink();
        }
    }

    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }

    fn rebuild_default_sink(&mut self) {
        let loggers = self.cache_dir.as_ref().map(|d| LoggerManager::new(d, self.clock.clone()));
        self.sink = Box::new(DefaultSink { loggers });
        self.default_sink = true;
    }

    // ---- severities ----

    /// `getLogSeverity()`.
    pub fn severity(&self, t: DebugType) -> LogSeverity {
        self.severities[t.ordinal()]
    }
    /// `setLogSeverity(newSeverity)`.
    pub fn set_severity(&mut self, t: DebugType, s: LogSeverity) {
        self.severities[t.ordinal()] = s;
    }
    pub fn severities(&self) -> &[LogSeverity] {
        &self.severities
    }
    /// `DebugType.isEnabled()` (including `orWith`).
    pub fn is_enabled(&self, t: DebugType) -> bool {
        self.severity(t) != LogSeverity::Off || t.or_with().is_some_and(|o| self.is_enabled(o))
    }
    /// `DebugType.isEnabled(logSeverity)` / `DebugLog.isLogEnabled`.
    pub fn is_enabled_for(&self, t: DebugType, s: LogSeverity) -> bool {
        self.severity(t).is_log_enabled(s) || t.or_with().is_some_and(|o| self.is_enabled_for(o, s))
    }
    /// `DebugLogStream.isLogEnabled` (the stream's own level only).
    fn stream_enabled(&self, t: Option<DebugType>, s: LogSeverity) -> bool {
        match t {
            Some(t) => self.severity(t).is_log_enabled(s),
            None => self.general_err_severity.is_log_enabled(s),
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    pub fn is_log_trace_file_location_enabled(&self) -> bool {
        self.log_trace_file_location_enabled
    }
    pub fn should_log_include_time_ms(&self) -> bool {
        self.log_time_ms_enabled
    }
    pub fn is_log_server_time_ms_enabled(&self) -> bool {
        self.log_server_time_ms_enabled
    }
    pub fn set_log_server_time_ms_enabled(&mut self, b: bool) {
        self.log_server_time_ms_enabled = b;
    }
    pub fn should_log_include_server_time(&self) -> bool {
        self.is_server || self.is_client || self.log_server_time_ms_enabled
    }

    // ---- formatting ----

    /// `formatLogStringForConsole` (identical to `formatLogStringForLogFile`).
    pub fn format_console_line(&self, t: Option<DebugType>, sev: LogSeverity, affix: &str, text: &str) -> String {
        let mut s = String::new();
        s.push_str(sev.log_prefix());
        s.push_str(&left_justify(t.unwrap_or(DebugType::General).name(), 12));
        s.push_str(&format!(" f:{}", self.frame_no));
        if self.should_log_include_time_ms() {
            s.push_str(&format!(" t:{}", self.clock.now_millis()));
        }
        if self.should_log_include_server_time() {
            s.push_str(&format!(" st:{}", group_number(self.clock.server_time_nanos().div_euclid(1_000_000))));
        }
        if !affix.is_empty() {
            s.push_str(" at ");
            s.push_str(affix);
        }
        s.push_str("> ");
        s.push_str(text);
        s
    }

    /// `formatLogStringForLogFile`.
    pub fn format_log_file_line(&self, t: Option<DebugType>, sev: LogSeverity, affix: &str, text: &str) -> String {
        self.format_console_line(t, sev, affix, text)
    }

    /// `formatLogStringAnimationRecordingFile`.
    pub fn format_recording_line(&self, t: Option<DebugType>, sev: LogSeverity, affix: &str, text: &str) -> String {
        let t = t.unwrap_or(DebugType::General);
        format!(
            "{},{},{},\"{}{}{}> {}\"",
            self.frame_no,
            t.name(),
            sev.name(),
            sev.log_prefix(),
            left_justify(t.name(), 12),
            if affix.is_empty() { String::new() } else { format!(" at {affix}") },
            text.replace("\r\n", "\n").replace('\n', "%0A").replace('"', "%22")
        )
    }

    fn zlogger_stamp(&self) -> String {
        local_now(&*self.clock).zlogger_stamp()
    }

    fn debug_log_file_name(&self) -> &'static str {
        if self.is_server {
            "DebugLog-server"
        } else {
            "DebugLog"
        }
    }

    /// `echoToLogFiles`.
    pub fn echo_to_log_files(&mut self, t: Option<DebugType>, sev: LogSeverity, affix: &str, raw: &str) {
        let line = self.format_log_file_line(t, sev, affix, raw);
        if let Ok(z) = zlogger_line(&self.zlogger_stamp(), &line, None, false) {
            let name = self.debug_log_file_name();
            self.sink.file(name, &z);
        }
        if self.recording {
            let r = self.format_recording_line(t, sev, affix, raw);
            self.sink.recording(&r);
        }
    }

    // ---- DebugLogStream writes ----

    /// `write(out, severity, text)`: `print` without newline.
    fn stream_write(&mut self, t: Option<DebugType>, stream: StdStream, sev: LogSeverity, text: &str) {
        if !self.stream_enabled(t, sev) {
            return;
        }
        let line = self.format_console_line(t, sev, "", text);
        self.sink.console(stream, &line);
        self.echo_to_log_files(t, sev, "", text);
    }

    /// `writeln(out, severity, format, params)`.
    fn stream_writeln(
        &mut self,
        t: Option<DebugType>,
        stream: StdStream,
        sev: LogSeverity,
        format: &str,
        params: &[FmtArg],
    ) -> Result<(), JavaError> {
        if !self.stream_enabled(t, sev) {
            return Ok(());
        }
        let s = formatted_output_str(format, params)?;
        let line = self.format_console_line(t, sev, "", &s);
        self.sink.console(stream, &format!("{line}\n"));
        self.echo_to_log_files(t, sev, "", &s);
        Ok(())
    }

    /// `writeWithCallerPrefixln`.
    #[allow(clippy::too_many_arguments)]
    fn write_with_caller(
        &mut self,
        t: Option<DebugType>,
        stream: StdStream,
        sev: LogSeverity,
        caller: Option<&StackElement>,
        allow_repeat: bool,
        format: &str,
        params: &[FmtArg],
    ) -> Result<(), JavaError> {
        if !self.stream_enabled(t, sev) {
            return Ok(());
        }
        let affix = left_justify(&stack_trace_element_string(caller, self.log_trace_file_location_enabled), 36);
        if !allow_repeat && !self.debug_once.entry(t).or_default().insert(affix.clone()) {
            return Ok(());
        }
        let s = formatted_output_str(format, params)?;
        let line = self.format_console_line(t, sev, &affix, &s);
        self.sink.console(stream, &format!("{line}\n"));
        self.echo_to_log_files(t, sev, &affix, &s);
        Ok(())
    }

    /// `print(x)` (any overload; `text` is `String.valueOf(x)`).
    pub fn print(&mut self, t: DebugType, text: &str) {
        self.stream_write(Some(t), StdStream::Out, LogSeverity::General, text);
    }
    /// `printf(format, args)`: always formats.
    pub fn printf(&mut self, t: DebugType, format: &str, args: &[FmtArg]) -> Result<(), JavaError> {
        let s = java_format(format, args)?;
        self.stream_write(Some(t), StdStream::Out, LogSeverity::General, &s);
        Ok(())
    }
    /// `println(String)`: no formatting.
    pub fn println(&mut self, t: DebugType, text: &str) {
        let _ = self.stream_writeln(Some(t), StdStream::Out, LogSeverity::General, text, &[]);
    }
    /// `println(format, params...)`.
    pub fn println_fmt(&mut self, t: DebugType, format: &str, params: &[FmtArg]) -> Result<(), JavaError> {
        self.stream_writeln(Some(t), StdStream::Out, LogSeverity::General, format, params)
    }
    /// `System.err.println(text)` (the `GeneralErr` stream).
    pub fn general_err_println(&mut self, text: &str) {
        let _ = self.stream_writeln(None, StdStream::Err, LogSeverity::General, text, &[]);
    }
    /// `System.out.println(text)` (`DebugType.General`'s stream).
    pub fn stdout_println(&mut self, text: &str) {
        self.println(DebugType::General, text);
    }

    /// `trace(format, params...)`.
    pub fn trace(&mut self, t: DebugType, caller: Option<&StackElement>, f: &str, p: &[FmtArg]) -> Result<(), JavaError> {
        self.write_with_caller(Some(t), StdStream::Out, LogSeverity::Trace, caller, true, f, p)
    }
    /// `debugln(format, params...)` (only with `Core.debug`).
    pub fn debugln(&mut self, t: DebugType, caller: Option<&StackElement>, f: &str, p: &[FmtArg]) -> Result<(), JavaError> {
        if !self.core_debug {
            return Ok(());
        }
        self.write_with_caller(Some(t), StdStream::Out, LogSeverity::Debug, caller, true, f, p)
    }
    /// `debugOnceln(format, params...)`: once per caller affix.
    pub fn debug_once(&mut self, t: DebugType, caller: Option<&StackElement>, f: &str, p: &[FmtArg]) -> Result<(), JavaError> {
        if !self.core_debug {
            return Ok(());
        }
        self.write_with_caller(Some(t), StdStream::Out, LogSeverity::Debug, caller, false, f, p)
    }
    /// `noise(format, params...)` (only with `Core.debug`).
    pub fn noise(&mut self, t: DebugType, caller: Option<&StackElement>, f: &str, p: &[FmtArg]) -> Result<(), JavaError> {
        if !self.core_debug {
            return Ok(());
        }
        self.write_with_caller(Some(t), StdStream::Out, LogSeverity::Noise, caller, true, f, p)
    }
    /// `warn(format, params...)` (the warn stream is stdout).
    pub fn warn(&mut self, t: DebugType, caller: Option<&StackElement>, f: &str, p: &[FmtArg]) -> Result<(), JavaError> {
        self.write_with_caller(Some(t), StdStream::Out, LogSeverity::Warning, caller, true, f, p)
    }
    /// `warnOnce(format, params...)`.
    pub fn warn_once(&mut self, t: DebugType, caller: Option<&StackElement>, f: &str, p: &[FmtArg]) -> Result<(), JavaError> {
        self.write_with_caller(Some(t), StdStream::Out, LogSeverity::Warning, caller, false, f, p)
    }
    /// `error(format, params...)` (stderr).
    pub fn error(&mut self, t: DebugType, caller: Option<&StackElement>, f: &str, p: &[FmtArg]) -> Result<(), JavaError> {
        self.write_with_caller(Some(t), StdStream::Err, LogSeverity::Error, caller, true, f, p)
    }

    /// `write(logSeverity, logText)` / `routedWrite`: `text` is not
    /// formatted; `General` has no caller affix; `Off` writes nothing.
    pub fn routed_write(&mut self, t: DebugType, sev: LogSeverity, caller: Option<&StackElement>, text: &str) {
        let _ = match sev {
            LogSeverity::Trace => self.trace(t, caller, text, &[]),
            LogSeverity::Noise => self.noise(t, caller, text, &[]),
            LogSeverity::Debug => self.debugln(t, caller, text, &[]),
            LogSeverity::General => {
                self.println(t, text);
                Ok(())
            }
            LogSeverity::Warning => self.warn(t, caller, text, &[]),
            LogSeverity::Error => self.error(t, caller, text, &[]),
            LogSeverity::Off => Ok(()),
        };
    }

    /// `getPrintStream(severity)`.
    fn print_stream(&mut self, t: DebugType, sev: LogSeverity) -> StdStream {
        match sev {
            LogSeverity::Trace | LogSeverity::Noise | LogSeverity::Debug | LogSeverity::General | LogSeverity::Warning => {
                StdStream::Out
            }
            LogSeverity::Error => StdStream::Err,
            LogSeverity::Off => {
                let caller = caller_of("zombie.debug.DebugLogStream", "getPrintStream");
                let _ = self.error(t, Some(&caller), "Unhandled LogSeverity: %s. Defaulted to Error.", &[FmtArg::Str(sev.name().into())]);
                StdStream::Err
            }
        }
    }

    /// `DebugLogStream.printException(ex, errorMessage, callerPrefix, severity)`.
    /// `caller_prefix` is the unpadded caller element string
    /// ([`stack_trace_element_string`]).
    pub fn print_exception(
        &mut self,
        t: DebugType,
        ex: Option<&LogThrowable>,
        message: Option<&str>,
        caller_prefix: &str,
        sev: LogSeverity,
    ) {
        let Some(ex) = ex else {
            let caller = caller_of("zombie.debug.DebugLogStream", "printException");
            let _ = self.warn(t, Some(&caller), "Null exception passed.", &[]);
            return;
        };
        if !self.stream_enabled(Some(t), sev) {
            return;
        }
        let stream = self.print_stream(t, sev);
        let include_stack = !matches!(sev, LogSeverity::Trace | LogSeverity::Noise | LogSeverity::General | LogSeverity::Warning);
        let top = ex.top_stack_trace_string();
        if include_stack {
            let mut sb = match message {
                Some(m) => format!("{caller_prefix}> Exception thrown\n\t{} at {top}. Message: {m}", ex.description),
                None => format!("{caller_prefix}> Exception thrown\n\t{} at {top}.", ex.description),
            };
            sb.push('\n');
            sb.push_str("\tStack trace:\n");
            let mut n = 0;
            for e in &ex.stack {
                let s = e.java_to_string();
                if n > 0 && s.starts_with("zombie.core.profiling.PerformanceProbes$") {
                    continue;
                }
                sb.push_str("\t\t");
                sb.push_str(&s);
                sb.push('\n');
                n += 1;
            }
            self.stream_write(Some(t), stream, sev, &sb);
        } else {
            let m = match message {
                Some(m) => format!("{caller_prefix}> Exception thrown {} at {top}. Message: {m}", ex.description),
                None => format!("{caller_prefix}> Exception thrown {} at {top}.", ex.description),
            };
            let _ = self.stream_writeln(Some(t), stream, sev, &m, &[]);
        }
    }

    /// `DebugLogStream.printException(ex, severity, callerPrefix, format, params)`.
    pub fn print_exception_fmt(
        &mut self,
        t: DebugType,
        ex: Option<&LogThrowable>,
        sev: LogSeverity,
        caller_prefix: &str,
        format: Option<&str>,
        params: &[FmtArg],
    ) -> Result<(), JavaError> {
        if !self.stream_enabled(Some(t), sev) {
            return Ok(());
        }
        let msg = match format {
            None => None,
            Some(f) if !params.is_empty() => Some(java_format(f, params)?),
            Some(f) => Some(java_format("%s", &[FmtArg::Str(f.into())])?),
        };
        self.print_exception(t, ex, msg.as_deref(), caller_prefix, sev);
        Ok(())
    }

    /// `DebugType.printException(ex, severity, format, params...)`: the
    /// original passes `new Object[]{logSeverity}` as the parameters.
    pub fn print_exception_type_fmt(
        &mut self,
        t: DebugType,
        ex: Option<&LogThrowable>,
        sev: LogSeverity,
        caller_prefix: &str,
        format: Option<&str>,
    ) -> Result<(), JavaError> {
        self.print_exception_fmt(t, ex, sev, caller_prefix, format, &[FmtArg::Object {
            class: "zombie.debug.LogSeverity".into(),
            text: sev.name().into(),
            hash: 0,
        }])
    }

    /// `DebugLogStream.printStackTrace(severity, depthStart, depthCount, format, params)`.
    /// `frames` is the stack from the caller of `DebugType.printStackTrace`
    /// outward; `depth_start` is the value `DebugType` passes (1).
    #[allow(clippy::too_many_arguments)]
    pub fn print_stack_trace(
        &mut self,
        t: DebugType,
        sev: LogSeverity,
        depth_start: i32,
        depth_count: i32,
        format: Option<&str>,
        params: &[FmtArg],
        frames: &[StackElement],
    ) -> Result<(), JavaError> {
        if !self.stream_enabled(Some(t), sev) {
            return Ok(());
        }
        let stream = self.print_stream(t, sev);
        if let Some(f) = format {
            let m = if params.is_empty() { f.to_string() } else { java_format(f, params)? };
            self.sink.console(stream, &format!("{m}\n"));
            self.echo_to_log_files(Some(t), sev, "", &m);
        }
        // Java index 0/1 are DebugLogStream/DebugType.printStackTrace and
        // the start is depthStart + 1.
        let st = stack_trace_string(frames, "\t", depth_start + 1 - 2, depth_count);
        self.sink.console(stream, &format!("{st}\n"));
        self.echo_to_log_files(Some(t), sev, "", &st);
        Ok(())
    }

    // ---- DebugLog statics ----

    /// `getDefaultLogSeverity()`.
    pub fn default_log_severity(&self) -> LogSeverity {
        if self.core_debug {
            LogSeverity::General
        } else if self.is_server {
            LogSeverity::Warning
        } else {
            LogSeverity::Off
        }
    }

    /// `setDefaultLogSeverity()`.
    pub fn set_default_log_severity(&mut self) {
        let def = self.default_log_severity();
        for &t in DebugType::VALUES {
            if (t == DebugType::DetailedInfo && !self.core_debug) || def.ordinal() >= self.severity(t).ordinal() {
                continue;
            }
            self.set_severity(t, def);
        }
    }

    /// `getConfigFileName()`.
    pub fn config_file_name(&self) -> &'static str {
        if self.is_server {
            "debuglog-server.cfg"
        } else {
            "debuglog.cfg"
        }
    }

    /// `getConfigFilePath()` (`None` without a cache dir).
    pub fn config_file_path(&self) -> Option<String> {
        self.cache_dir.as_ref().map(|d| format!("{}{}{}", d.display(), std::path::MAIN_SEPARATOR, self.config_file_name()))
    }

    /// `printLogLevels()`.
    pub fn print_log_levels(&mut self) {
        let name = self.config_file_name();
        self.println(DebugType::General, &format!("You can setup the log levels in the {name} file"));
        let path = self.config_file_path().unwrap_or_else(|| format!("null{}{name}", std::path::MAIN_SEPARATOR));
        self.println(DebugType::DetailedInfo, &format!("log config path: {path}"));
        self.println(DebugType::General, "Logs configuration:");
        for sev in LogSeverity::VALUES {
            if sev == LogSeverity::Off {
                continue;
            }
            for &t in DebugType::VALUES {
                if self.severity(t) == sev {
                    let _ = self.println_fmt(DebugType::General, "%12s: %s", &[t.name().into(), sev.name().into()]);
                }
            }
        }
        let def = self.default_log_severity();
        let _ = self.println_fmt(DebugType::General, "%12s: %s", &["Default".into(), def.name().into()]);
    }

    /// `DebugLog.log(type, str)` / `log(str)`.
    pub fn log(&mut self, t: DebugType, s: &str) {
        self.println(t, s);
    }

    /// `setLogEnabled(type, enabled)`.
    pub fn set_log_enabled(&mut self, t: DebugType, enabled: bool) {
        if self.is_enabled(t) != enabled {
            let s = if enabled { self.default_log_severity() } else { LogSeverity::Off };
            self.set_severity(t, s);
        }
    }

    /// `setLogEnabledFromCommandLine(token)` (`-debuglog=` entries).
    pub fn set_log_enabled_from_command_line(&mut self, token: &str) {
        let Some(first) = token.chars().next() else { return };
        let enabled = first != '-';
        let name = if first == '+' || first == '-' { &token[1..] } else { token };
        let Some(t) = DebugType::value_of(name) else { return };
        if t == DebugType::General && !enabled {
            return;
        }
        self.set_severity(t, if enabled { LogSeverity::General } else { LogSeverity::Off });
    }

    /// `getDebugTypes()`: sorted case-insensitively by name.
    pub fn debug_types_sorted() -> Vec<DebugType> {
        let mut v = DebugType::VALUES.to_vec();
        v.sort_by(|a, b| case_insensitive_cmp(a.name(), b.name()));
        v
    }

    /// `getSelectedProfileName()`.
    pub fn selected_profile_name(&self) -> Option<String> {
        self.cfg.selected_profile.clone()
    }

    /// `getProfileNames()`.
    pub fn profile_names(&self) -> Vec<String> {
        self.cfg.profiles.iter().map(|p| p.name.clone()).collect()
    }

    /// `getProfileAliases()`.
    pub fn profile_aliases(&self) -> Vec<String> {
        let mut v = self.cfg.aliases.clone();
        v.sort_by(|a, b| case_insensitive_cmp(a, b));
        v
    }

    /// `getLogSeverityForSelectedProfile(debugType)`.
    pub fn log_severity_for_selected_profile(&self, t: DebugType) -> Result<Option<LogSeverity>, JavaError> {
        let sel = self
            .cfg
            .selected_profile
            .as_deref()
            .ok_or_else(|| JavaError::npe("Cannot invoke \"String.startsWith(String)\" because \"selectedProfile\" is null"))?;
        if sel.starts_with('$') {
            let mut sev = None;
            for name in self.cfg.resolve_alias_profiles(sel)? {
                if let Some(s) = self.cfg.profile(&name).and_then(|p| p.log_severity(t)) {
                    sev = Some(s);
                }
            }
            return Ok(sev);
        }
        let p = self.cfg.profile(sel).ok_or_else(|| {
            JavaError::npe("Cannot invoke \"zombie.debug.DebugLogProfile.getLogSeverity(zombie.debug.DebugType)\" because \"profile\" is null")
        })?;
        Ok(p.log_severity(t))
    }

    /// `DebugLogProfile.invoke()`.
    fn invoke_commands(&mut self, commands: Vec<String>) {
        for s in commands {
            if let Some(r) = s.strip_prefix('+') {
                self.read_config_command(r, true);
            } else if let Some(r) = s.strip_prefix('-') {
                self.read_config_command(r, false);
            } else {
                self.log(DebugType::General, &format!("unknown command: '{s}'"));
            }
        }
    }

    /// `invokeProfile(profileOrAlias)`.
    pub fn invoke_profile(&mut self, name: &str) -> Result<(), JavaError> {
        self.cfg.selected_profile = Some(name.to_string());
        if name.starts_with('$') {
            self.log(DebugType::General, &format!("Selected debug alias = '{name}'"));
            for p in self.cfg.resolve_alias_profiles(name)? {
                let cmds = self.cfg.profile(&p).map(|p| p.commands.clone()).unwrap_or_default();
                self.invoke_commands(cmds);
            }
            return Ok(());
        }
        self.log(DebugType::General, &format!("Selected debug profile = '{name}'"));
        let cmds = self.cfg.profile(name).map(|p| p.commands.clone()).ok_or_else(|| {
            JavaError::npe("Cannot invoke \"zombie.debug.DebugLogProfile.invoke()\" because the return value of \"zombie.debug.DebugLogCfgFile.getProfile(String)\" is null")
        })?;
        self.invoke_commands(cmds);
        Ok(())
    }

    /// `invokeSelectedProfile()`.
    pub fn invoke_selected_profile(&mut self) -> Result<(), JavaError> {
        match self.selected_profile_name() {
            Some(n) => self.invoke_profile(&n),
            None => Err(JavaError::npe("Cannot invoke \"String.startsWith(String)\" because \"profileOrAlias\" is null")),
        }
    }

    fn selected_profile_mut(&mut self, method: &str) -> Result<&mut DebugLogProfile, JavaError> {
        let sel = self.cfg.selected_profile.clone();
        sel.and_then(move |s| self.cfg.profile_mut(&s))
            .ok_or_else(|| JavaError::npe(&format!("Cannot invoke \"zombie.debug.DebugLogProfile.{method}\" because \"profile\" is null")))
    }

    /// `updateSelectedProfileAll(logSeverity)`.
    pub fn update_selected_profile_all(&mut self, sev: Option<LogSeverity>) -> Result<(), JavaError> {
        self.selected_profile_mut("updateAll(zombie.debug.LogSeverity)")?.update_all(sev);
        Ok(())
    }

    /// `updateSelectedProfile(debugType, logSeverity)`.
    pub fn update_selected_profile(&mut self, t: DebugType, sev: Option<LogSeverity>) -> Result<(), JavaError> {
        self.selected_profile_mut("update(zombie.debug.DebugType, zombie.debug.LogSeverity)")?.update(t, sev);
        Ok(())
    }

    /// `writeConfigFile()`.
    pub fn write_config_file(&mut self) -> Result<(), JavaError> {
        if self.debug_cfg_file_watcher_path.is_none() {
            self.debug_cfg_file_watcher_path = self.config_file_path();
        }
        let Some(path) = self.debug_cfg_file_watcher_path.clone() else { return Ok(()) };
        let cache = self.cache_dir.as_ref().map(|d| d.display().to_string());
        let Some(cache) = cache else { return Ok(()) };
        if !path.ends_with(".cfg") || path.contains("..") || !path.starts_with(&cache) {
            return Ok(());
        }
        std::fs::write(&path, self.cfg.to_text()).map_err(|e| io_error(&path, &e))
    }

    /// `nativeLog(logType, logSeverity, logTxt)`.
    pub fn native_log(&mut self, log_type: &str, log_severity: &str, text: &str, caller: Option<&StackElement>) {
        let t = DebugType::value_of(log_type).unwrap_or(DebugType::General);
        let s = LogSeverity::value_of(log_severity).unwrap_or(LogSeverity::General);
        self.routed_write(t, s, caller, text);
    }

    /// `init()`: the six types enabled at `General` in every build.
    pub fn init(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        for t in [
            DebugType::ClientCommand,
            DebugType::General,
            DebugType::Lua,
            DebugType::Mod,
            DebugType::Multiplayer,
            DebugType::Network,
        ] {
            self.set_severity(t, LogSeverity::General);
        }
    }

    /// `loadDebugConfig(filepath)`.
    pub fn load_debug_config(&mut self, filepath: Option<&str>) {
        let sev = self.severities;
        self.cfg.set_default(&sev);
        let caller = stack_trace_element_string(Some(&caller_of("zombie.debug.DebugLog", "loadDebugConfig")), self.log_trace_file_location_enabled);
        let path = match filepath {
            Some(p) => p.to_string(),
            None => match self.config_file_path() {
                Some(p) if Path::new(&p).is_file() => p,
                _ => return,
            },
        };
        self.log(DebugType::General, "Attempting to read debug config...");
        let mut file = PathBuf::from(&path);
        if !file.is_file() {
            self.log(DebugType::General, "Attempting relative path...");
            let parent = std::env::current_dir().ok().and_then(|d| d.parent().map(|p| p.display().to_string()));
            file = PathBuf::from(format!("{}{}{path}", parent.as_deref().unwrap_or("null"), std::path::MAIN_SEPARATOR));
        }
        let abs = if file.is_absolute() {
            file.clone()
        } else {
            std::env::current_dir().map(|d| d.join(&file)).unwrap_or_else(|_| file.clone())
        };
        self.println(DebugType::DetailedInfo, &format!("file = {}", abs.display()));
        if !file.is_file() {
            self.log(DebugType::General, "Could not find debug config.");
            return;
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                let errs = self.cfg.read_text(&String::from_utf8_lossy(&bytes));
                let read_caller = stack_trace_element_string(
                    Some(&caller_of("zombie.debug.DebugLogCfgFile", "readInternal")),
                    self.log_trace_file_location_enabled,
                );
                for e in errs {
                    self.print_exception(DebugType::General, Some(&(&e).into()), None, &read_caller, LogSeverity::Error);
                }
            }
            Err(e) => {
                self.cfg.clear();
                let e = io_error(&path, &e);
                self.print_exception(DebugType::General, Some(&(&e).into()), None, &caller, LogSeverity::Error);
            }
        }
        if let Some(sel) = self.cfg.selected_profile.clone() {
            if let Err(e) = self.invoke_profile(&sel) {
                self.print_exception(DebugType::General, Some(&(&e).into()), None, &caller, LogSeverity::Error);
                return;
            }
        }
        self.debug_cfg_file_watcher_path = Some(file.display().to_string());
    }

    /// `readConfigCommand(s, enable)`.
    pub fn read_config_command(&mut self, s: &str, enable: bool) {
        if let Err(e) = self.read_config_command_inner(s, enable) {
            let caller = stack_trace_element_string(
                Some(&caller_of("zombie.debug.DebugLog", "readConfigCommand")),
                self.log_trace_file_location_enabled,
            );
            self.print_exception(
                DebugType::General,
                Some(&(&e).into()),
                Some("Exception thrown in readConfigCommand"),
                &caller,
                LogSeverity::Error,
            );
        }
    }

    fn read_config_command_inner(&mut self, s: &str, enable: bool) -> Result<(), JavaError> {
        let mut type_str = s.to_string();
        let mut sev_str: Option<String> = None;
        if contains_whitespace(s) {
            let split = split_ws(s);
            type_str = java_trim(split.first().ok_or_else(|| index_oob(0, 0))?).to_string();
            sev_str = Some(java_trim(split.get(1).ok_or_else(|| index_oob(1, split.len()))?).to_string());
        }
        let mut sev = LogSeverity::Debug;
        if !is_null_or_whitespace(sev_str.as_deref()) {
            let n = sev_str.unwrap();
            sev = LogSeverity::value_of(&n).ok_or_else(|| no_enum_constant("zombie.debug.LogSeverity", &n))?;
        }
        if java_equals_ignore_case(&type_str, "LogTraceFileLocation") {
            self.log_trace_file_location_enabled = enable;
            return Ok(());
        }
        if java_equals_ignore_case(&type_str, "LogTimeMsEnabled") {
            self.log_time_ms_enabled = enable;
            return Ok(());
        }
        if java_equals_ignore_case(&type_str, "LogServerTimeMsEnabled") {
            self.log_server_time_ms_enabled = enable;
            return Ok(());
        }
        if java_equals_ignore_case(&type_str, "all") {
            for &t in DebugType::VALUES {
                if t == DebugType::General && !enable {
                    continue;
                }
                self.set_severity(t, sev);
                self.set_log_enabled(t, enable);
            }
            return Ok(());
        }
        let t = if type_str.contains('.') {
            let split = java_regex_split(&type_str, |c| c == '.', false);
            let t = DebugType::value_of_or_throw(split.first().map_or("", |s| s.as_str()))?;
            let script = split.get(1).ok_or_else(|| index_oob(1, split.len()))?;
            self.script_debug_requests.push((script.clone(), enable));
            t
        } else {
            DebugType::value_of_or_throw(&type_str)?
        };
        self.set_severity(t, sev);
        self.set_log_enabled(t, enable);
        Ok(())
    }

    // ---- LuaManager.GlobalObject.writeLog ----

    /// `writeLog(loggerName, logs)`: `LoggerManager.getLogger(FileName.sanitize(name)).write(logs)`.
    pub fn write_log(&mut self, logger_name: &str, logs: &str) {
        let name = sanitize_file_name(logger_name);
        match zlogger_line(&self.zlogger_stamp(), logs, None, false) {
            Ok(line) => self.sink.file(&name, &line),
            Err(e) => {
                let caller = stack_trace_element_string(Some(&caller_of("zombie.core.logger.ZLogger", "write")), self.log_trace_file_location_enabled);
                self.print_exception(DebugType::General, Some(&(&e).into()), None, &caller, LogSeverity::Error);
            }
        }
    }

    // ---- startup sequences ----

    /// `MainScreenState.main` logging setup: `init()`, `-debuglog=`
    /// tokens, `loadDebugConfig(debugcfg)` in debug builds, then
    /// `printLogLevels()`.
    pub fn client_startup(&mut self, debuglog_tokens: &[&str], debug_cfg: Option<&str>) {
        self.init();
        for t in debuglog_tokens {
            self.set_log_enabled_from_command_line(t);
        }
        if self.core_debug {
            self.load_debug_config(debug_cfg);
        }
        self.print_log_levels();
    }

    /// `GameServer.main` logging setup: `init()`, `setDefaultLogSeverity()`,
    /// `loadDebugConfig(null)` in debug builds, `-debuglog=` tokens.
    pub fn server_startup(&mut self, debuglog_tokens: &[&str]) {
        self.init();
        self.set_default_log_severity();
        if self.core_debug {
            self.load_debug_config(None);
        }
        for t in debuglog_tokens {
            self.set_log_enabled_from_command_line(t);
        }
    }
}

// ===========================================================================
// ExceptionLogger and java.util.logging
// ===========================================================================

/// A `java.util.logging.Level` (only its name and value are used).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JulLevel {
    Severe,
    Warning,
    Info,
    Config,
    Fine,
    Finer,
    Finest,
}

impl JulLevel {
    /// `Level.getName()` (the default formatter prints the localized name,
    /// which is the same in English).
    pub fn name(self) -> &'static str {
        match self {
            JulLevel::Severe => "SEVERE",
            JulLevel::Warning => "WARNING",
            JulLevel::Info => "INFO",
            JulLevel::Config => "CONFIG",
            JulLevel::Fine => "FINE",
            JulLevel::Finer => "FINER",
            JulLevel::Finest => "FINEST",
        }
    }
    /// `Level.intValue()`.
    pub fn value(self) -> i32 {
        match self {
            JulLevel::Severe => 1000,
            JulLevel::Warning => 900,
            JulLevel::Info => 800,
            JulLevel::Config => 700,
            JulLevel::Fine => 500,
            JulLevel::Finer => 400,
            JulLevel::Finest => 300,
        }
    }
}

const MONTH_ABBR: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

impl LocalTime {
    /// `%1$tb %1$td, %1$tY %1$tl:%1$tM:%1$tS %1$Tp` (en-US), the date part of
    /// `SimpleFormatter`'s default format.
    pub fn jul_stamp(&self) -> String {
        let h12 = match self.hour % 12 {
            0 => 12,
            h => h,
        };
        let ampm = if self.hour < 12 { "AM" } else { "PM" };
        format!(
            "{} {:02}, {} {}:{:02}:{:02} {}",
            MONTH_ABBR[(self.month as usize).clamp(1, 12) - 1],
            self.day,
            self.year,
            h12,
            self.minute,
            self.second,
            ampm
        )
    }
}

impl DebugLog {
    /// `ExceptionLogger.logException(ex, callerPrefix, errorMessage, out, severity)`.
    /// `caller` is the frame that called `logException` (what
    /// `DebugLogStream.generateCallerPrefix()` finds). The on-screen popup is
    /// not drawn here; only its counter is kept.
    pub fn log_exception(&mut self, ex: &LogThrowable, caller: Option<&StackElement>, message: Option<&str>, t: DebugType, sev: LogSeverity) {
        let prefix = stack_trace_element_string(caller, self.log_trace_file_location_enabled);
        self.print_exception(t, Some(ex), message, &prefix, sev);
        self.exception_count += 1;
    }

    /// `Logger.getLogger(logger_name).log(level, msg[, thrown])` under the
    /// JDK's default logging configuration (the game installs none): the root
    /// `ConsoleHandler` at `INFO` with `SimpleFormatter`'s default format.
    /// `source` is the inferred caller frame; without it the logger name is
    /// printed. The handler writes bytes to `System.err`, which
    /// `DebugLogStream` passes through unformatted, so the text goes to
    /// stderr as is and not to the log files.
    pub fn jul_log(&mut self, logger_name: &str, level: JulLevel, source: Option<&StackElement>, msg: Option<&str>, thrown: Option<&LogThrowable>) {
        if level.value() < JulLevel::Info.value() {
            return;
        }
        let stamp = local_now(&*self.clock).jul_stamp();
        let source = match source {
            Some(e) => format!("{} {}", e.class_name, e.method_name),
            None => logger_name.to_string(),
        };
        let thrown = match thrown {
            None => String::new(),
            Some(ex) => {
                let mut s = format!("\n{}\n", ex.description);
                for e in &ex.stack {
                    s.push_str(&format!("\tat {}\n", e.java_to_string()));
                }
                s
            }
        };
        let text = format!("{stamp} {source}\n{}: {}{thrown}\n", level.name(), msg.unwrap_or("null"));
        self.sink.console(StdStream::Err, &text);
    }
}

/// `ExceptionLogger.logException(ex)` on this thread's log: `General`,
/// `Error`, no message.
pub fn log_exception(ex: &LogThrowable, caller: Option<&StackElement>) {
    with(|d| d.log_exception(ex, caller, None, DebugType::General, LogSeverity::Error));
}

/// `DebugLog.log(str)` on this thread's log (`DebugType.General.println`).
pub fn log(s: &str) {
    with(|d| d.log(DebugType::General, s));
}

/// `Logger.getLogger(GameWindow.class.getName()).log(level, msg[, thrown])`
/// on this thread's log (see [`DebugLog::jul_log`]).
pub fn game_window_logger(level: JulLevel, source: Option<&StackElement>, msg: Option<&str>, thrown: Option<&LogThrowable>) {
    with(|d| d.jul_log("zombie.GameWindow", level, source, msg, thrown));
}

// ===========================================================================
// Thread-local instances
// ===========================================================================

thread_local! {
    static GLOBAL: RefCell<DebugLog> = RefCell::new(DebugLog::new());
    static OPTIONS: Rc<RefCell<DebugOptions>> = Rc::new(RefCell::new(DebugOptions::default()));
}

/// Run `f` on this thread's `DebugLog` (`DebugLog.getInstance()` plus the
/// `DebugType` constants' state). Do not log from inside a sink.
pub fn with<R>(f: impl FnOnce(&mut DebugLog) -> R) -> R {
    GLOBAL.with(|g| f(&mut g.borrow_mut()))
}

/// This thread's `DebugOptions.instance`.
pub fn debug_options() -> Rc<RefCell<DebugOptions>> {
    OPTIONS.with(|o| o.clone())
}

/// Route `zombie.config`'s `DebugLog.log` calls ([`crate::config::debug_log`])
/// to `DebugType.General.println` on this thread's log.
pub fn route_config_log() {
    crate::config::set_log_hook(Some(Box::new(|m| {
        GLOBAL.with(|g| {
            if let Ok(mut d) = g.try_borrow_mut() {
                d.println(DebugType::General, m);
            }
        })
    })));
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_logger_and_jul() {
        let (mut d, sink, _) = logger();
        d.set_severity(DebugType::General, LogSeverity::General);
        let caller = StackElement::new("zombie.iso.IsoLot", "load");
        d.log_exception(&LogThrowable::new("java.io.IOException: bad"), Some(&caller), None, DebugType::General, LogSeverity::Error);
        assert_eq!(d.exception_count, 1);
        assert_eq!(
            sink.log().stderr(),
            "ERROR: General      f:0> IsoLot.load> Exception thrown\n\tjava.io.IOException: bad at No Stack Trace Available.\n\tStack trace:\n"
        );
        sink.clear();
        let src = StackElement::new("zombie.iso.IsoCell", "PlaceLot");
        d.jul_log("zombie.GameWindow", JulLevel::Severe, Some(&src), Some("Missing tile definition: x_0"), None);
        assert_eq!(sink.log().stderr(), "Oct 23, 2026 11:30:04 PM zombie.iso.IsoCell PlaceLot\nSEVERE: Missing tile definition: x_0\n");
        sink.clear();
        d.jul_log("zombie.GameWindow", JulLevel::Severe, None, None, Some(&LogThrowable::new("java.io.IOException: e")));
        assert_eq!(sink.log().stderr(), "Oct 23, 2026 11:30:04 PM zombie.GameWindow\nSEVERE: null\njava.io.IOException: e\n\n");
        sink.clear();
        d.jul_log("zombie.GameWindow", JulLevel::Fine, None, Some("x"), None);
        assert!(sink.log().console.is_empty());
    }

    fn logger() -> (DebugLog, MemorySink, Rc<FixedClock>) {
        let sink = MemorySink::new();
        // 2026-10-23 23:30:04.860 at UTC+2
        let clock = Rc::new(FixedClock::new(1_792_791_004_860, 2 * 3_600_000));
        let d = DebugLog::with_sink(sink.clone(), clock.clone());
        (d, sink, clock)
    }

    #[test]
    fn enum_tables() {
        assert_eq!(DebugType::COUNT, 76);
        assert_eq!(DebugType::VALUES[75], DebugType::AnimationRecorder);
        assert_eq!(DebugType::value_of("ISUIStackTrace"), Some(DebugType::ISUIStackTrace));
        assert_eq!(DebugType::General.or_with(), Some(DebugType::AnimationRecorder));
        assert_eq!(DebugType::Lua.or_with(), None);
        assert_eq!(LogSeverity::ALL, LogSeverity::Trace);
        assert!(LogSeverity::General.is_log_enabled(LogSeverity::Error));
        assert!(!LogSeverity::General.is_log_enabled(LogSeverity::Debug));
        assert!(!LogSeverity::Off.is_log_enabled(LogSeverity::Error));
        assert!(LogSeverity::Error.is_log_enabled(LogSeverity::Off));
        assert!(DebugType::Lua.is_name("lua"));
    }

    #[test]
    fn console_and_file_lines_match_game_output() {
        let (mut d, sink, _) = logger();
        d.init();
        d.println(DebugType::General, "23-10-2026 23:30:04");
        let caller = StackElement::new("oshi.util.platform.mac.SysctlUtilFFM", "sysctlbyname");
        d.warn(DebugType::General, Some(&caller), "Unable to parse version %s to a codename.", &["27.0".into()]).unwrap();
        let caller = StackElement::lua("Vanilla", "ISInventoryTransferUtil.lua", None, 3);
        d.warn(DebugType::Lua, Some(&caller), "require(\"TimedActions/ISInventoryTransferAction\") failed", &[]).unwrap();
        d.error(DebugType::General, Some(&StackElement::new("zombie.core.fonts.AngelCodeFont", "parseFnt")), "x", &[]).unwrap();
        let log = sink.log();
        assert_eq!(
            log.stdout(),
            "LOG  : General      f:0> 23-10-2026 23:30:04\n\
             WARN : General      f:0 at SysctlUtilFFM.sysctlbyname          > Unable to parse version 27.0 to a codename.\n\
             WARN : Lua          f:0 at Lua(Vanilla).ISInventoryTransferUtil.lua> require(\"TimedActions/ISInventoryTransferAction\") failed\n"
        );
        assert_eq!(log.stderr(), "ERROR: General      f:0 at AngelCodeFont.parseFnt              > x\n");
        let f = log.file_lines("DebugLog");
        assert_eq!(f[0], "[23-10-26 23:30:04.860] LOG  : General      f:0> 23-10-2026 23:30:04.");
        assert_eq!(
            f[1],
            "[23-10-26 23:30:04.860] WARN : General      f:0 at SysctlUtilFFM.sysctlbyname          > Unable to parse version 27.0 to a codename.."
        );
    }

    #[test]
    fn time_and_server_time_affixes() {
        let (mut d, sink, clock) = logger();
        d.init();
        d.read_config_command("LogTimeMsEnabled", true);
        d.is_client = true;
        clock.server_nanos.set(1_234_567_000_000);
        d.frame_no = 42;
        d.println(DebugType::Network, "hi");
        let now = clock.now_ms.get();
        assert_eq!(sink.log().stdout(), format!("LOG  : Network      f:42 t:{now} st:1,234,567> hi\n"));
    }

    #[test]
    fn print_log_levels_release_defaults() {
        let (mut d, sink, _) = logger();
        d.client_startup(&[], None);
        let out = sink.log().stdout();
        let expected = "LOG  : General      f:0> You can setup the log levels in the debuglog.cfg file\n\
LOG  : General      f:0> Logs configuration:\n\
LOG  : General      f:0>      General: General\n\
LOG  : General      f:0>      Network: General\n\
LOG  : General      f:0>          Lua: General\n\
LOG  : General      f:0>          Mod: General\n\
LOG  : General      f:0>  Multiplayer: General\n\
LOG  : General      f:0> ClientCommand: General\n\
LOG  : General      f:0>      Default: Off\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn server_defaults_and_debug_gating() {
        let (mut d, _, _) = logger();
        d.is_server = true;
        d.server_startup(&["+Sound", "-Lua", "-General", "Bogus"]);
        assert_eq!(d.severity(DebugType::Zombie), LogSeverity::Warning);
        assert_eq!(d.severity(DebugType::DetailedInfo), LogSeverity::Off);
        assert_eq!(d.severity(DebugType::Sound), LogSeverity::General);
        assert_eq!(d.severity(DebugType::Lua), LogSeverity::Off);
        assert_eq!(d.severity(DebugType::General), LogSeverity::General);

        let (mut d, sink, _) = logger();
        d.init();
        d.set_severity(DebugType::Lua, LogSeverity::Trace);
        d.debugln(DebugType::Lua, None, "hidden %d", &[FmtArg::Double(1.0)]).unwrap();
        d.noise(DebugType::Lua, None, "hidden", &[]).unwrap();
        d.trace(DebugType::Lua, None, "shown", &[]).unwrap();
        assert_eq!(sink.log().stdout(), "TRACE: Lua          f:0 at (UnknownStack)                      > shown\n");
        d.core_debug = true;
        assert!(d.debugln(DebugType::Lua, None, "%d", &[FmtArg::Double(1.0)]).is_err());
    }

    #[test]
    fn severity_filtering() {
        let (mut d, sink, _) = logger();
        d.set_severity(DebugType::Sound, LogSeverity::Warning);
        d.println(DebugType::Sound, "general hidden");
        d.trace(DebugType::Sound, None, "trace hidden", &[]).unwrap();
        d.warn(DebugType::Sound, None, "warn", &[]).unwrap();
        d.error(DebugType::Sound, None, "err", &[]).unwrap();
        let log = sink.log();
        assert_eq!(log.console.len(), 2);
        assert!(log.stdout().starts_with("WARN : Sound"));
        assert!(log.stderr().starts_with("ERROR: Sound"));
    }

    #[test]
    fn animation_recorder_also_activates() {
        let (mut d, sink, _) = logger();
        assert!(!d.is_enabled(DebugType::Death));
        d.set_severity(DebugType::AnimationRecorder, LogSeverity::Trace);
        for t in DebugType::AnimationRecorder.also_activate() {
            assert!(d.is_enabled(*t), "{t}");
            assert!(d.is_enabled_for(*t, LogSeverity::Trace));
        }
        assert!(!d.is_enabled(DebugType::Lua));
        // The stream itself only looks at its own level.
        d.warn(DebugType::Death, None, "not written", &[]).unwrap();
        assert!(sink.log().console.is_empty());
        // setLogEnabled sees orWith: Death counts as enabled already.
        d.set_log_enabled(DebugType::Death, true);
        assert_eq!(d.severity(DebugType::Death), LogSeverity::Off);
    }

    #[test]
    fn once_per_caller() {
        let (mut d, sink, _) = logger();
        d.set_severity(DebugType::Lua, LogSeverity::Trace);
        let a = StackElement::new("x.A", "f");
        let b = StackElement::new("x.B", "f");
        for _ in 0..3 {
            d.warn_once(DebugType::Lua, Some(&a), "a", &[]).unwrap();
            d.warn_once(DebugType::Lua, Some(&b), "b", &[]).unwrap();
        }
        assert_eq!(sink.log().console.len(), 2);
    }

    #[test]
    fn java_format_subset() {
        let f = |s: &str, a: &[FmtArg]| java_format(s, a).unwrap();
        assert_eq!(f("%s|%5s|%-5s|%.2s", &["ab".into(), "ab".into(), "ab".into(), "abc".into()]), "ab|   ab|ab   |ab");
        assert_eq!(f("%s %s %s", &[FmtArg::Double(5.0), FmtArg::Null, FmtArg::Bool(true)]), "5.0 null true");
        assert_eq!(f("%d|%5d|%-5d|%05d|%,d|%+d|%(d", &[1.into(), 42.into(), 42.into(), (-42).into(), 1234567.into(), 3.into(), (-3).into()]), "1|   42|42   |-0042|1,234,567|+3|(3)");
        assert_eq!(f("%x %X %#x %o", &[255.into(), (-1).into(), 255.into(), 8.into()]), "ff FFFFFFFF 0xff 10");
        assert_eq!(f("%f|%.2f|%.1f|%.0f|%8.3f|%,.2f", &[1.5.into(), 0.125.into(), 0.15.into(), 0.5.into(), (-3.14159).into(), 1234567.891.into()]), "1.500000|0.13|0.2|1|  -3.142|1,234,567.89");
        assert_eq!(f("%f %.3f", &[FmtArg::Float(0.1), 1e-5.into()]), "0.100000 0.000");
        assert_eq!(f("%e|%.2E|%e", &[12345.678.into(), 0.000123.into(), 0.0.into()]), "1.234568e+04|1.23E-04|0.000000e+00");
        assert_eq!(f("%g|%g|%g", &[0.0001.into(), 123456789.0.into(), 1.0.into()]), "0.000100000|1.23457e+08|1.00000");
        assert_eq!(f("%f %f", &[f64::NAN.into(), f64::NEG_INFINITY.into()]), "NaN -Infinity");
        assert_eq!(f("100%% %n%2$s %1$s %<s", &["a".into(), "b".into()]), "100% \nb a a");
        assert_eq!(f("%b %b %c", &[FmtArg::Null, "x".into(), 65.into()]), "false true A");
        assert_eq!(f("%12s: %s", &["Multiplayer".into(), "General".into()]), " Multiplayer: General");
    }

    #[test]
    fn java_format_errors() {
        let e = |s: &str, a: &[FmtArg]| java_format(s, a).unwrap_err().to_string();
        assert_eq!(e("%d", &[FmtArg::Double(1.0)]), "java.util.IllegalFormatConversionException: d != java.lang.Double");
        assert_eq!(e("%X", &[FmtArg::Double(1.0)]), "java.util.IllegalFormatConversionException: x != java.lang.Double");
        assert_eq!(e("%f", &[FmtArg::Int(1)]), "java.util.IllegalFormatConversionException: f != java.lang.Integer");
        assert_eq!(e("%s %s", &["a".into()]), "java.util.MissingFormatArgumentException: Format specifier '%s'");
        assert_eq!(e("%q", &[]), "java.util.UnknownFormatConversionException: Conversion = 'q'");
        assert_eq!(e("abc %", &[]), "java.util.UnknownFormatConversionException: Conversion = '%'");
        assert_eq!(e("%-d", &[1.into()]), "java.util.MissingFormatWidthException: %-d");
        assert_eq!(e("%.2d", &[1.into()]), "java.util.IllegalFormatPrecisionException: 2");
        assert_eq!(e("%+s", &["a".into()]), "java.util.FormatFlagsConversionMismatchException: Conversion = s, Flags = +");
        assert_eq!(e("%--5s", &["a".into()]), "java.util.DuplicateFormatFlagsException: Flags = '-'");
        // Parse errors win over missing arguments.
        assert_eq!(e("%s %q", &[]), "java.util.UnknownFormatConversionException: Conversion = 'q'");
    }

    #[test]
    fn print_exception_layout() {
        let (mut d, sink, _) = logger();
        d.init();
        let ex = LogThrowable {
            description: "java.lang.RuntimeException: attempted index of non-table at KahluaUtil.fail(KahluaUtil.java:100)".into(),
            stack: vec![
                StackElement::new("se.krka.kahlua.vm.KahluaUtil", "fail").with_location("KahluaUtil.java", 100),
                StackElement::lua("(MOD:Common Sense)", "BB_CS_Fireplace.lua", Some("BB_CS_Fireplace.lua"), 42),
            ],
        };
        d.print_exception(DebugType::General, Some(&ex), None, "Lua((MOD:Common Sense)).BB_CS_Fireplace.lua", LogSeverity::Error);
        let log = sink.log();
        assert_eq!(
            log.stderr(),
            "ERROR: General      f:0> Lua((MOD:Common Sense)).BB_CS_Fireplace.lua> Exception thrown\n\
\tjava.lang.RuntimeException: attempted index of non-table at KahluaUtil.fail(KahluaUtil.java:100) at KahluaUtil.fail(KahluaUtil.java:100).\n\
\tStack trace:\n\
\t\tse.krka.kahlua.vm.KahluaUtil.fail(KahluaUtil.java:100)\n\
\t\tLua((MOD:Common Sense)).BB_CS_Fireplace.lua(BB_CS_Fireplace.lua:42)\n"
        );
        assert!(log.file_lines("DebugLog")[0].ends_with("(BB_CS_Fireplace.lua:42)."));
    }

    #[test]
    fn stack_trace_print() {
        let (mut d, sink, _) = logger();
        d.set_severity(DebugType::LuaObject, LogSeverity::Warning);
        let frames = vec![StackElement::lua("Vanilla", "a", Some("a.lua"), 1), StackElement::lua("Vanilla", "b", Some("b.lua"), 2)];
        d.print_stack_trace(DebugType::LuaObject, LogSeverity::Warning, 1, 4, Some("No Type found: x"), &[], &frames).unwrap();
        let log = sink.log();
        assert_eq!(log.stdout(), "No Type found: x\n\tLua(Vanilla).a(a.lua:1)\n\tLua(Vanilla).b(b.lua:2)\n");
        assert_eq!(log.file_lines("DebugLog")[0], "[23-10-26 23:30:04.860] WARN : LuaObject    f:0> No Type found: x.");
    }

    #[test]
    fn config_file_profiles() {
        let (mut d, sink, _) = logger();
        d.init();
        let errs = d.cfg.read_text(
            "// comment\ndefault\n{\n    -all\n    +General\n    +Lua Trace\n    +Sound Warning\n}\nnet\n{\n  +Network Noise\n}\n$both = default + net\n$bad\n=$both\n",
        );
        assert_eq!(errs.len(), 1);
        assert_eq!(d.profile_names(), ["default", "net"]);
        assert_eq!(d.profile_aliases(), ["both"]);
        d.invoke_selected_profile().unwrap();
        assert_eq!(d.severity(DebugType::General), LogSeverity::Debug);
        assert_eq!(d.severity(DebugType::Lua), LogSeverity::Trace);
        assert_eq!(d.severity(DebugType::Sound), LogSeverity::Warning);
        assert_eq!(d.severity(DebugType::Network), LogSeverity::Noise);
        assert_eq!(d.severity(DebugType::Mod), LogSeverity::Off);
        assert_eq!(d.log_severity_for_selected_profile(DebugType::Network).unwrap(), Some(LogSeverity::Noise));
        assert!(sink.log().stdout().contains("Selected debug alias = '$both'"));

        d.cfg.selected_profile = Some("default".into());
        d.update_selected_profile(DebugType::Lua, Some(LogSeverity::Off)).unwrap();
        d.update_selected_profile_all(Some(LogSeverity::Error)).unwrap();
        assert_eq!(d.cfg.profile("default").unwrap().commands, ["+all Error"]);
        let text = d.cfg.to_text();
        assert!(text.starts_with("default\n{\n    +all Error\n}\n\nnet\n"));
        assert!(text.ends_with("$both = default + net\n\n=default\n"));

        d.read_config_command("NoSuchType", true);
        assert!(sink.log().stderr().contains(
            "DebugLog.readConfigCommand> Exception thrown\n\tjava.lang.IllegalArgumentException: No enum constant zombie.debug.DebugType.NoSuchType at No Stack Trace Available. Message: Exception thrown in readConfigCommand\n"
        ));
    }

    #[test]
    fn set_default_profile_from_current() {
        let (mut d, _, _) = logger();
        d.init();
        d.load_debug_config(None);
        let p = d.cfg.profile("default").unwrap();
        assert_eq!(p.commands[0], "-all");
        assert_eq!(p.commands[1], "+General General");
        assert_eq!(d.selected_profile_name().as_deref(), Some("default"));
    }

    #[test]
    fn zlogger_and_names() {
        assert_eq!(zlogger_line("s", "a\n", None, false).unwrap(), "[s] a.");
        assert_eq!(zlogger_line("s", "a", Some("p"), true).unwrap(), "[p]a");
        assert!(zlogger_line("s", "", None, false).is_err());
        assert_eq!(sanitize_file_name("a/b:c. "), "a_b_c");
        assert_eq!(sanitize_file_name("con.txt"), "_con.txt");
        assert_eq!(group_number(-1234567), "-1,234,567");
        let t = LocalTime::from_epoch(951_782_400_000, 0); // 2000-02-29
        assert_eq!(t.date_time_stamp(), "2000-02-29_00-00");
    }

    #[test]
    fn recording_line() {
        let (mut d, sink, _) = logger();
        d.recording = true;
        d.set_severity(DebugType::Death, LogSeverity::General);
        d.println(DebugType::Death, "a\"b\nc");
        assert_eq!(sink.log().recording[0], "0,Death,General,\"LOG  : Death       > a%22b%0Ac\"");
    }

    #[test]
    fn debug_options_registry() {
        let mut o = DebugOptions::default();
        o.load_options_list_xml(
            "<debugOptionsXml>\n    <options>\n        <name>Asset.SlowLoad</name>\n        <value>false</value>\n    </options>\n    <options>\n        <name>UI.DebugConsole.StartVisible</name>\n        <value>true</value>\n    </options>\n</debugOptionsXml>\n",
        );
        assert_eq!(o.option_count(), 2);
        assert!(o.get_boolean("UI.DebugConsole.StartVisible"));
        assert!(!o.get_boolean("Missing"));
        o.set_boolean("Asset.SlowLoad", true);
        assert!(o.get_boolean("Asset.SlowLoad"));
        assert!(o.options_list_xml().contains("<name>Asset.SlowLoad</name>\n        <value>true</value>"));
        let mut d = DebugOptions::default();
        d.set_definitions(vec![DebugOptionDef { name: "X.Y".into(), default_value: false, debug_only: true }]);
        d.set_boolean("X.Y", true);
        assert!(!d.get_boolean("X.Y"));
        d.core_debug = true;
        assert!(d.get_boolean("X.Y"));
    }
}
