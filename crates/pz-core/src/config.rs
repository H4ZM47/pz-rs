//! `zombie.config`: the `ConfigOption` family and `ConfigFile`, the
//! `name=value` reader/writer `Core` uses for `options.ini`.
//!
//! The Java class hierarchy (`ConfigOption` with `Boolean`, `Integer`,
//! `Double`, `String` and `Array` subclasses) is a single enum here. Parsing,
//! validation, clamping, logging and string formatting follow the original
//! exactly, including its quirks (`IntegerConfigOption.parse` goes through
//! `Double.parseDouble`, `ArrayConfigOption.setValueVarArgs` resizes and
//! re-fixes the array, `String.split` drops trailing empty fields, ...).
//!
//! On-change callbacks (`ConfigOptionOnChangeCallback`) are recorded as a
//! pending flag on the option (see [`ConfigOption::take_changed`]); the owner
//! (`Core`) turns them into the Lua events the Java callbacks trigger.

use pz_kahlua::number::{java_double_to_string, java_parse_double, java_parse_int};
use std::cell::RefCell;
use std::path::Path;

type LogHook = Option<Box<dyn Fn(&str)>>;

thread_local! {
    static LOG_HOOK: RefCell<LogHook> = const { RefCell::new(None) };
}

/// Install a sink for `DebugLog.log` messages emitted by this module and
/// `Core` (parse errors, range errors, ...). Without one they are dropped.
pub fn set_log_hook(hook: LogHook) {
    LOG_HOOK.with(|h| *h.borrow_mut() = hook);
}

/// `DebugLog.log`.
pub fn debug_log(msg: &str) {
    LOG_HOOK.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f(msg);
        }
    });
}

/// Java `String.split(sep)` for a single literal (non-regex-special)
/// separator: no match returns the whole string, trailing empty fields are
/// removed.
pub fn java_split<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    if s.is_empty() || !s.contains(sep) {
        return vec![s];
    }
    let mut v: Vec<&str> = s.split(sep).collect();
    while v.last().is_some_and(|x| x.is_empty()) {
        v.pop();
    }
    v
}

/// Java `String.trim()`: strips chars `<= ' '`.
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Java `String.valueOf(double)`.
pub fn double_to_string(d: f64) -> String {
    java_double_to_string(d)
}

/// Java `Float.toString(float)` (shortest digits that round-trip as f32).
pub fn float_to_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f < 0.0 {
            "-Infinity".into()
        } else {
            "Infinity".into()
        };
    }
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let sign = if f < 0.0 { "-" } else { "" };
    let abs = f.abs();
    if (1.0e-3..1.0e7).contains(&abs) {
        let point = exp + 1;
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        };
        format!("{sign}{s}")
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{}", &digits[..1], frac, exp)
    }
}

/// `String.CASE_INSENSITIVE_ORDER.compare`.
pub fn case_insensitive_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let a: Vec<u16> = a.encode_utf16().collect();
    let b: Vec<u16> = b.encode_utf16().collect();
    let fold = |c: u16| -> u16 {
        let ch = char::from_u32(c as u32).unwrap_or('\u{0}');
        let up = ch.to_uppercase().next().unwrap_or(ch);
        let lo = up.to_lowercase().next().unwrap_or(up);
        let mut buf = [0u16; 2];
        if lo.len_utf16() == 1 {
            lo.encode_utf16(&mut buf)[0]
        } else {
            c
        }
    };
    for i in 0..a.len().min(b.len()) {
        let (c1, c2) = (a[i], b[i]);
        if c1 != c2 {
            let (f1, f2) = (fold(c1), fold(c2));
            if f1 != f2 {
                return f1.cmp(&f2);
            }
        }
    }
    a.len().cmp(&b.len())
}

/// Truncate to at most `n` UTF-16 code units (`String.substring(0, n)`).
fn truncate_utf16(s: &str, n: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for ch in s.chars() {
        units += ch.len_utf16();
        if units > n {
            break;
        }
        out.push(ch);
    }
    out
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Which `Core` callback an option's `ConfigOptionOnChangeCallback` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnChange {
    /// `Core.onOptionControllerButtonStyleChanged`.
    ControllerButtonStyle,
    /// `Core.onOptionGamepadBindingPresetChanged`.
    GamepadBindingPreset,
}

/// A Java object passed to `setValueFromObject` / `setValueVarArgs`.
#[derive(Clone, Debug)]
pub enum ConfigObject {
    Null,
    Bool(bool),
    /// `java.lang.Double` (also used for widened `Float` arguments).
    Double(f64),
    Str(String),
    /// Any other object, by its `toString()`.
    Other(String),
}

#[derive(Clone, Debug)]
pub struct BooleanConfigOption {
    pub value: bool,
    pub default_value: bool,
}

#[derive(Clone, Debug)]
pub struct IntegerConfigOption {
    pub value: i32,
    pub default_value: i32,
    pub min: i32,
    pub max: i32,
}

#[derive(Clone, Debug)]
pub struct DoubleConfigOption {
    pub value: f64,
    pub default_value: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug)]
pub struct StringConfigOption {
    pub value: String,
    pub default_value: String,
    pub max_length: i32,
    pub values: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct ArrayConfigOption {
    pub value: Vec<ConfigOption>,
    pub default_value: Vec<ConfigOption>,
    pub separator: String,
    pub element_handler: Box<ConfigOption>,
    pub fixed_size: i32,
    pub multi_line: bool,
}

#[derive(Clone, Debug)]
pub enum OptionKind {
    Boolean(BooleanConfigOption),
    Integer(IntegerConfigOption),
    Double(DoubleConfigOption),
    String(StringConfigOption),
    Array(ArrayConfigOption),
}

/// `zombie.config.ConfigOption` and its subclasses.
#[derive(Clone, Debug)]
pub struct ConfigOption {
    name: String,
    pub kind: OptionKind,
    on_change: Option<OnChange>,
    /// Set when the on-change callback would have fired.
    changed: bool,
}

impl ConfigOption {
    fn new(name: &str, kind: OptionKind) -> Self {
        // `ConfigOption(String)` throws IllegalArgumentException for these.
        assert!(
            !name.is_empty() && !name.contains('='),
            "invalid ConfigOption name {name:?}"
        );
        ConfigOption {
            name: name.to_string(),
            kind,
            on_change: None,
            changed: false,
        }
    }

    /// `new BooleanConfigOption(name, defaultValue)`.
    pub fn boolean(name: &str, default_value: bool) -> Self {
        Self::new(
            name,
            OptionKind::Boolean(BooleanConfigOption {
                value: default_value,
                default_value,
            }),
        )
    }

    /// `new IntegerConfigOption(name, min, max, defaultValue)`.
    pub fn integer(name: &str, min: i32, max: i32, default_value: i32) -> Self {
        assert!(
            default_value >= min && default_value <= max,
            "IntegerConfigOption {name}: default out of range"
        );
        Self::new(
            name,
            OptionKind::Integer(IntegerConfigOption {
                value: default_value,
                default_value,
                min,
                max,
            }),
        )
    }

    /// `new DoubleConfigOption(name, min, max, defaultValue)`.
    pub fn double(name: &str, min: f64, max: f64, default_value: f64) -> Self {
        assert!(
            default_value >= min && default_value <= max,
            "DoubleConfigOption {name}: default out of range"
        );
        Self::new(
            name,
            OptionKind::Double(DoubleConfigOption {
                value: default_value,
                default_value,
                min,
                max,
            }),
        )
    }

    /// `new StringConfigOption(name, defaultValue, maxLength)` (also the
    /// on-change constructor, which leaves `maxLength` at 0).
    pub fn string_max(name: &str, default_value: &str, max_length: i32) -> Self {
        Self::new(
            name,
            OptionKind::String(StringConfigOption {
                value: default_value.to_string(),
                default_value: default_value.to_string(),
                max_length,
                values: None,
            }),
        )
    }

    /// `new StringConfigOption(name, defaultValue, String[] values)`: an
    /// unknown default falls back to `values[0]`.
    pub fn string_values(name: &str, default_value: &str, values: &[&str]) -> Self {
        let default_value = if values.contains(&default_value) {
            default_value
        } else {
            values[0]
        };
        Self::new(
            name,
            OptionKind::String(StringConfigOption {
                value: default_value.to_string(),
                default_value: default_value.to_string(),
                max_length: 0,
                values: Some(values.iter().map(|s| s.to_string()).collect()),
            }),
        )
    }

    /// `new ArrayConfigOption(name, elementHandler, separator, defaultValue)`.
    pub fn array(
        name: &str,
        element_handler: ConfigOption,
        separator: &str,
        default_value: &str,
    ) -> Self {
        let mut o = Self::new(
            name,
            OptionKind::Array(ArrayConfigOption {
                value: Vec::new(),
                default_value: Vec::new(),
                separator: separator.to_string(),
                element_handler: Box::new(element_handler),
                fixed_size: -1,
                multi_line: false,
            }),
        );
        o.parse(default_value);
        o.set_default_to_current_value();
        o
    }

    /// `ArrayConfigOption.setFixedSize`.
    pub fn fixed_size(mut self, size: i32) -> Self {
        if let OptionKind::Array(a) = &mut self.kind {
            if size > 0 {
                a.resize_value(size);
                a.resize_default(size);
            } else {
                a.fixed_size = -1;
            }
        }
        self
    }

    /// `ArrayConfigOption.setMultiLine`.
    pub fn multi_line(mut self, b: bool) -> Self {
        if let OptionKind::Array(a) = &mut self.kind {
            a.multi_line = b;
        }
        self
    }

    /// `setOnChangeCallback`.
    pub fn on_change(mut self, cb: OnChange) -> Self {
        self.on_change = Some(cb);
        self
    }

    pub fn on_change_callback(&self) -> Option<OnChange> {
        self.on_change
    }

    /// Returns (and clears) whether the on-change callback fired since the
    /// last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn invoke_on_change_event(&mut self) {
        if self.on_change.is_some() {
            self.changed = true;
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `getType()`.
    pub fn get_type(&self) -> &'static str {
        match &self.kind {
            OptionKind::Boolean(_) => "boolean",
            OptionKind::Integer(_) => "integer",
            OptionKind::Double(_) => "double",
            OptionKind::String(_) => "string",
            OptionKind::Array(_) => "array",
        }
    }

    /// `resetToDefault()`.
    pub fn reset_to_default(&mut self) {
        match &mut self.kind {
            OptionKind::Boolean(b) => b.value = b.default_value,
            OptionKind::Integer(i) => {
                let d = i.default_value;
                self.set_int(d);
            }
            OptionKind::Double(d) => {
                let v = d.default_value;
                self.set_double(v);
            }
            OptionKind::String(s) => {
                let d = s.default_value.clone();
                self.set_string_internal(d);
            }
            OptionKind::Array(a) => a.value = a.default_value.clone(),
        }
    }

    /// `setDefaultToCurrentValue()`.
    pub fn set_default_to_current_value(&mut self) {
        match &mut self.kind {
            OptionKind::Boolean(b) => b.default_value = b.value,
            OptionKind::Integer(i) => i.default_value = i.value,
            OptionKind::Double(d) => d.default_value = d.value,
            OptionKind::String(s) => s.default_value = s.value.clone(),
            OptionKind::Array(a) => a.default_value = a.value.clone(),
        }
    }

    /// `parse(String)`.
    pub fn parse(&mut self, s: &str) {
        match &self.kind {
            OptionKind::Boolean(_) => {
                if self.is_valid_string(s) {
                    self.set_bool(s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("1"));
                } else {
                    debug_log(&format!(
                        "ERROR BooleanConfigOption.parse() \"{}\" string={}\"",
                        self.name, s
                    ));
                }
            }
            OptionKind::Integer(_) => match java_parse_double(s) {
                Some(v) => self.set_int(v as i32),
                None => debug_log(&format!(
                    "ERROR IntegerConfigOption.parse() \"{}\" string=\"{}\"",
                    self.name, s
                )),
            },
            OptionKind::Double(_) => match java_parse_double(s) {
                Some(v) => self.set_double(v),
                None => debug_log(&format!(
                    "ERROR DoubleConfigOption.parse() \"{}\" string=\"{}\"",
                    self.name, s
                )),
            },
            OptionKind::String(_) | OptionKind::Array(_) => {
                self.set_value_from_object(&ConfigObject::Str(s.to_string()))
            }
        }
    }

    /// `getValueAsString()`.
    pub fn get_value_as_string(&self) -> String {
        match &self.kind {
            OptionKind::Boolean(b) => b.value.to_string(),
            OptionKind::Integer(i) => i.value.to_string(),
            OptionKind::Double(d) => double_to_string(d.value),
            OptionKind::String(s) => s.value.clone(),
            OptionKind::Array(a) => a.value_as_string(&a.value),
        }
    }

    /// `getValueAsLuaString()`.
    pub fn get_value_as_lua_string(&self) -> String {
        match &self.kind {
            OptionKind::String(s) => {
                format!("\"{}\"", s.value.replace('\\', "\\\\").replace('"', "\\\""))
            }
            _ => self.get_value_as_string(),
        }
    }

    /// `getTooltip()`.
    pub fn get_tooltip(&self) -> String {
        self.get_value_as_string()
    }

    /// `setValueFromObject(Object)`.
    pub fn set_value_from_object(&mut self, o: &ConfigObject) {
        match &mut self.kind {
            OptionKind::Boolean(_) => match o {
                ConfigObject::Bool(b) => self.set_bool(*b),
                ConfigObject::Double(d) => self.set_bool(*d != 0.0),
                ConfigObject::Str(s) => {
                    let s = s.clone();
                    self.parse(&s)
                }
                _ => {}
            },
            OptionKind::Integer(_) => match o {
                ConfigObject::Double(d) => self.set_int(*d as i32),
                ConfigObject::Str(s) => {
                    let s = s.clone();
                    self.parse(&s)
                }
                _ => {}
            },
            OptionKind::Double(_) => match o {
                ConfigObject::Double(d) => self.set_double(*d),
                ConfigObject::Str(s) => {
                    let s = s.clone();
                    self.parse(&s)
                }
                _ => {}
            },
            OptionKind::String(s) => {
                if let Some(values) = &s.values {
                    let found = match o {
                        ConfigObject::Str(x) => values.iter().find(|v| *v == x).cloned(),
                        _ => None,
                    };
                    match found {
                        Some(v) => self.set_string_internal(v),
                        None => debug_log(&format!(
                            "ERROR: StringConfigOption.setValueFromObject() \"{}\" value \"{}\" is unknown",
                            self.name,
                            object_to_string(o)
                        )),
                    }
                    return;
                }
                let v = match o {
                    ConfigObject::Null => String::new(),
                    ConfigObject::Str(x) => x.clone(),
                    other => object_to_string(other),
                };
                self.set_string_internal(v);
            }
            OptionKind::Array(_) => {
                let ConfigObject::Str(s) = o else {
                    debug_log(&format!(
                        "ERROR ArrayConfigOption.setValueFromObject() \"{}\" string={}\"",
                        self.name,
                        object_to_string(o)
                    ));
                    return;
                };
                if !self.is_valid_string(s) {
                    debug_log(&format!(
                        "ERROR ArrayConfigOption.setValueFromObject() \"{}\" string={}\"",
                        self.name, s
                    ));
                    return;
                }
                let OptionKind::Array(a) = &mut self.kind else {
                    unreachable!()
                };
                if !a.multi_line {
                    a.value.clear();
                }
                if !java_trim(s).is_empty() {
                    for part in java_split(s, &a.separator) {
                        a.element_handler.reset_to_default();
                        a.element_handler.parse(part);
                        let copy = a.element_handler.make_copy();
                        a.value.push(copy);
                    }
                }
                if a.fixed_size > 0 {
                    let n = a.fixed_size;
                    a.resize_value(n);
                }
            }
        }
    }

    /// `getValueAsObject()` as a [`ConfigObject`].
    pub fn get_value_as_object(&self) -> ConfigObject {
        match &self.kind {
            OptionKind::Boolean(b) => ConfigObject::Bool(b.value),
            OptionKind::Integer(i) => ConfigObject::Double(i.value as f64),
            OptionKind::Double(d) => ConfigObject::Double(d.value),
            OptionKind::String(s) => ConfigObject::Str(s.value.clone()),
            OptionKind::Array(_) => ConfigObject::Str(self.get_value_as_string()),
        }
    }

    /// `isValidString(String)`.
    pub fn is_valid_string(&self, s: &str) -> bool {
        match &self.kind {
            OptionKind::Boolean(_) => {
                s.eq_ignore_ascii_case("true")
                    || s.eq_ignore_ascii_case("false")
                    || s.eq_ignore_ascii_case("1")
                    || s.eq_ignore_ascii_case("0")
            }
            OptionKind::Integer(i) => {
                java_parse_int(s, 10).is_some_and(|v| v >= i.min && v <= i.max)
            }
            OptionKind::Double(d) => java_parse_double(s).is_some_and(|v| v >= d.min && v <= d.max),
            OptionKind::String(st) => match &st.values {
                Some(values) => values.iter().any(|v| v == s),
                None => true,
            },
            OptionKind::Array(a) => {
                if java_trim(s).is_empty() {
                    return true;
                }
                java_split(s, &a.separator)
                    .iter()
                    .all(|p| a.element_handler.is_valid_string(p))
            }
        }
    }

    /// `makeCopy()`: same value and default, no on-change callback.
    pub fn make_copy(&self) -> ConfigOption {
        let mut c = self.clone();
        c.on_change = None;
        c.changed = false;
        if let OptionKind::Array(a) = &mut c.kind {
            // ArrayConfigOption.makeCopy re-runs the constructor with the
            // default string: fixedSize/multiLine are not carried over.
            a.fixed_size = -1;
            a.multi_line = false;
        }
        c
    }

    // ---- typed accessors -------------------------------------------------

    /// `BooleanConfigOption.getValue()`.
    pub fn get_bool(&self) -> bool {
        match &self.kind {
            OptionKind::Boolean(b) => b.value,
            _ => panic!("{} is not a BooleanConfigOption", self.name),
        }
    }

    /// `BooleanConfigOption.setValue(boolean)`.
    pub fn set_bool(&mut self, v: bool) {
        match &mut self.kind {
            OptionKind::Boolean(b) => b.value = v,
            _ => panic!("{} is not a BooleanConfigOption", self.name),
        }
    }

    /// `IntegerConfigOption.getValue()`.
    pub fn get_int(&self) -> i32 {
        match &self.kind {
            OptionKind::Integer(i) => i.value,
            _ => panic!("{} is not an IntegerConfigOption", self.name),
        }
    }

    /// `IntegerConfigOption.setValue(int)`: out-of-range values are logged
    /// and ignored.
    pub fn set_int(&mut self, v: i32) {
        let OptionKind::Integer(i) = &mut self.kind else {
            panic!("{} is not an IntegerConfigOption", self.name)
        };
        if v < i.min {
            debug_log(&format!(
                "ERROR: IntegerConfigOption.setValue() \"{}\" {} is less than min={}",
                self.name, v, i.min
            ));
            return;
        }
        if v > i.max {
            debug_log(&format!(
                "ERROR: IntegerConfigOption.setValue() \"{}\" {} is greater than max={}",
                self.name, v, i.max
            ));
            return;
        }
        if i.value != v {
            i.value = v;
            self.invoke_on_change_event();
        }
    }

    /// `DoubleConfigOption.getValue()`.
    pub fn get_double(&self) -> f64 {
        match &self.kind {
            OptionKind::Double(d) => d.value,
            _ => panic!("{} is not a DoubleConfigOption", self.name),
        }
    }

    /// `DoubleConfigOption.setValue(double)`.
    pub fn set_double(&mut self, v: f64) {
        let OptionKind::Double(d) = &mut self.kind else {
            panic!("{} is not a DoubleConfigOption", self.name)
        };
        if v < d.min {
            debug_log(&format!(
                "ERROR: DoubleConfigOption.setValue() \"{}\" {} is less than min={}",
                self.name,
                double_to_string(v),
                double_to_string(d.min)
            ));
            return;
        }
        if v > d.max {
            debug_log(&format!(
                "ERROR: DoubleConfigOption.setValue() \"{}\" {} is greater than max={}",
                self.name,
                double_to_string(v),
                double_to_string(d.max)
            ));
            return;
        }
        d.value = v;
    }

    /// `StringConfigOption.getValue()`.
    pub fn get_string(&self) -> &str {
        match &self.kind {
            OptionKind::String(s) => &s.value,
            _ => panic!("{} is not a StringConfigOption", self.name),
        }
    }

    /// `StringConfigOption.setValue(String)` (`None` is Java `null`).
    pub fn set_string(&mut self, v: Option<&str>) {
        let OptionKind::String(s) = &self.kind else {
            panic!("{} is not a StringConfigOption", self.name)
        };
        if s.values.is_some() {
            match v {
                Some(v) if self.is_valid_string(v) => self.set_string_internal(v.to_string()),
                _ => debug_log(&format!(
                    "ERROR StringConfigOption.setValue() \"{}\" string=\"{}\"",
                    self.name,
                    v.unwrap_or("null")
                )),
            }
            return;
        }
        let mut v = v.unwrap_or("").to_string();
        if s.max_length > 0 && utf16_len(&v) > s.max_length as usize {
            v = truncate_utf16(&v, s.max_length as usize);
        }
        self.set_string_internal(v);
    }

    fn set_string_internal(&mut self, v: String) {
        let OptionKind::String(s) = &mut self.kind else {
            unreachable!()
        };
        if s.value != v {
            s.value = v;
            self.invoke_on_change_event();
        }
    }

    pub fn as_array(&self) -> &ArrayConfigOption {
        match &self.kind {
            OptionKind::Array(a) => a,
            _ => panic!("{} is not an ArrayConfigOption", self.name),
        }
    }

    pub fn as_array_mut(&mut self) -> &mut ArrayConfigOption {
        match &mut self.kind {
            OptionKind::Array(a) => a,
            _ => panic!("{} is not an ArrayConfigOption", self.name),
        }
    }

    pub fn is_multi_line(&self) -> bool {
        matches!(&self.kind, OptionKind::Array(a) if a.multi_line)
    }
}

fn object_to_string(o: &ConfigObject) -> String {
    match o {
        ConfigObject::Null => "null".into(),
        ConfigObject::Bool(b) => b.to_string(),
        ConfigObject::Double(d) => double_to_string(*d),
        ConfigObject::Str(s) | ConfigObject::Other(s) => s.clone(),
    }
}

impl ArrayConfigOption {
    fn value_as_string(&self, value: &[ConfigOption]) -> String {
        value
            .iter()
            .map(|e| e.get_value_as_string())
            .collect::<Vec<_>>()
            .join(&self.separator)
    }

    fn resized(&mut self, mut value: Vec<ConfigOption>, new_size: i32) -> Vec<ConfigOption> {
        self.fixed_size = new_size;
        self.element_handler.reset_to_default();
        let n = new_size.max(0) as usize;
        while value.len() < n {
            value.push(self.element_handler.make_copy());
        }
        value.truncate(n);
        value
    }

    fn resize_value(&mut self, n: i32) {
        let v = std::mem::take(&mut self.value);
        self.value = self.resized(v, n);
    }

    fn resize_default(&mut self, n: i32) {
        let v = std::mem::take(&mut self.default_value);
        self.default_value = self.resized(v, n);
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.value.clear();
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.value.len()
    }

    /// `getElement(index)`; panics like `ArrayList.get` when out of range.
    pub fn get_element(&self, index: usize) -> &ConfigOption {
        &self.value[index]
    }

    pub fn get_element_mut(&mut self, index: usize) -> &mut ConfigOption {
        &mut self.value[index]
    }

    /// `setValueVarArgs(Object...)`.
    pub fn set_value_var_args(&mut self, args: &[ConfigObject]) {
        let mut size = args.len();
        if self.fixed_size > 0 {
            size = size.min(self.fixed_size as usize);
        } else {
            self.resize_value(args.len() as i32);
        }
        for (i, a) in args.iter().enumerate().take(size) {
            self.value[i].set_value_from_object(a);
        }
    }

    /// `getElementAsDouble(index, defaultValue)`.
    pub fn get_element_as_double(&self, index: i32, default_value: f32) -> f64 {
        if index >= 0 && (index as usize) < self.value.len() {
            if let OptionKind::Double(_) = self.element_handler.kind {
                return self.value[index as usize].get_double();
            }
        }
        default_value as f64
    }

    /// `getValueAsColorInfo(colorInfo)`: the (r, g, b, a) it stores.
    pub fn get_value_as_color(&self) -> [f32; 4] {
        [
            self.get_element_as_double(0, 1.0) as f32,
            self.get_element_as_double(1, 1.0) as f32,
            self.get_element_as_double(2, 1.0) as f32,
            self.get_element_as_double(3, 1.0) as f32,
        ]
    }
}

/// `zombie.config.ConfigFile`.
pub struct ConfigFile {
    pub options: Vec<ConfigOption>,
    pub version: i32,
    pub version_string: String,
    pub write_tooltips: bool,
}

impl Default for ConfigFile {
    fn default() -> Self {
        ConfigFile {
            options: Vec::new(),
            version: 0,
            version_string: "Version".into(),
            write_tooltips: true,
        }
    }
}

impl ConfigFile {
    pub fn new() -> Self {
        Self::default()
    }

    fn file_error(file_name: &str, line_number: usize, message: &str) {
        debug_log(&format!("{file_name}:{line_number} {message}"));
    }

    /// `read(fileName)`: every `name=value` line becomes a
    /// `StringConfigOption`; the `versionString` line sets the version.
    /// Returns false if the file does not exist or reading aborted (the
    /// cases where Java throws inside the try block).
    pub fn read(&mut self, file_name: &Path) -> bool {
        self.options = Vec::new();
        self.version = 0;
        let display = file_name.to_string_lossy().into_owned();
        let bytes = match std::fs::read(file_name) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let text = String::from_utf8_lossy(&bytes);
        for (n, raw) in java_lines(&text).into_iter().enumerate() {
            let line_number = n + 1;
            let line = java_trim(raw);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if !line.contains('=') {
                Self::file_error(&display, line_number, line);
                continue;
            }
            let ss = java_split(line, "=");
            // `ss[0]` on an empty array, or an empty option name, throws in
            // Java and aborts the whole read.
            let Some(first) = ss.first() else {
                return false;
            };
            if self.version_string == *first {
                let Some(v) = ss.get(1) else { return false };
                match java_parse_int(v, 10) {
                    Some(v) => self.version = v,
                    None => Self::file_error(
                        &display,
                        line_number,
                        &format!("expected version number, got \"{v}\""),
                    ),
                }
                continue;
            }
            if first.is_empty() {
                return false;
            }
            let value = ss.get(1).copied().unwrap_or("");
            self.options
                .push(ConfigOption::string_max(first, value, -1));
        }
        true
    }

    /// `write(fileName, version, options)`.
    pub fn write(&self, file_name: &Path, version: i32, options: &[&ConfigOption]) -> bool {
        let sep = line_separator();
        let mut out = String::new();
        if version != 0 {
            out.push_str(&format!("{}={}{}", self.version_string, version, sep));
        }
        let line_sep = if self.write_tooltips {
            format!("{sep}{sep}")
        } else {
            sep.to_string()
        };
        let count = options.len();
        for (i, option) in options.iter().enumerate() {
            if self.write_tooltips {
                let tooltip = option
                    .get_tooltip()
                    .replace("\\n", " ")
                    .replace("\\\"", "\"");
                let tooltip = tooltip.replace('\n', &format!("{sep}# "));
                out.push_str(&format!("# {tooltip}{sep}"));
            }
            if option.is_multi_line() {
                let a = option.as_array();
                for j in 0..a.size() {
                    out.push_str(&format!(
                        "{}={}",
                        option.name(),
                        a.get_element(j).get_value_as_string()
                    ));
                    if j >= a.size() - 1 && i >= count - 1 {
                        continue;
                    }
                    out.push_str(&line_sep);
                }
                continue;
            }
            out.push_str(&format!(
                "{}={}{}",
                option.name(),
                option.get_value_as_string(),
                if i < count - 1 { line_sep.as_str() } else { "" }
            ));
        }
        std::fs::write(file_name, out).is_ok()
    }

    pub fn get_options(&self) -> &[ConfigOption] {
        &self.options
    }

    pub fn get_version(&self) -> i32 {
        self.version
    }

    pub fn set_version_string(&mut self, s: &str) {
        self.version_string = s.to_string();
    }

    pub fn set_write_tooltips(&mut self, b: bool) {
        self.write_tooltips = b;
    }
}

/// `System.lineSeparator()`.
pub fn line_separator() -> &'static str {
    if cfg!(windows) {
        "\r\n"
    } else {
        "\n"
    }
}

/// `BufferedReader.readLine` splitting: `\n`, `\r` or `\r\n`.
pub fn java_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&text[start..i]);
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
        lines.push(&text[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_like_java() {
        assert_eq!(java_split("a=b", "="), vec!["a", "b"]);
        assert_eq!(java_split("a=", "="), vec!["a"]);
        assert_eq!(java_split("=", "="), Vec::<&str>::new());
        assert_eq!(java_split("", ","), vec![""]);
        assert_eq!(java_split("a,,b,,", ","), vec!["a", "", "b"]);
        assert_eq!(java_split("a=b=c", "="), vec!["a", "b", "c"]);
    }

    #[test]
    fn float_strings() {
        assert_eq!(float_to_string(1.0), "1.0");
        assert_eq!(float_to_string(0.98), "0.98");
        assert_eq!(float_to_string(0.1), "0.1");
        assert_eq!(double_to_string(0.98f32 as f64), "0.9800000190734863");
        assert_eq!(double_to_string(0.5), "0.5");
    }

    #[test]
    fn array_options() {
        let mut o = ConfigOption::array(
            "c",
            ConfigOption::double("element", 0.0, 1.0, 1.0),
            ",",
            "0.5,1.0, 1.0",
        )
        .fixed_size(3);
        assert_eq!(o.get_value_as_string(), "0.5,1.0,1.0");
        o.parse("0.1,0.2");
        assert_eq!(o.get_value_as_string(), "0.1,0.2,1.0");
        o.parse("0.1,2.0,0.3"); // invalid: element out of range
        assert_eq!(o.get_value_as_string(), "0.1,0.2,1.0");
        let mut z =
            ConfigOption::array("z", ConfigOption::integer("element", 25, 250, 100), ";", "");
        assert_eq!(z.get_value_as_string(), "");
        z.parse("50;75");
        assert_eq!(z.get_value_as_string(), "50;75");
        let b = ConfigOption::array("b", ConfigOption::boolean("element", false), ",", "")
            .fixed_size(4);
        assert_eq!(b.get_value_as_string(), "false,false,false,false");
    }

    #[test]
    fn int_and_string_options() {
        let mut i = ConfigOption::integer("i", 1, 3, 1).on_change(OnChange::ControllerButtonStyle);
        i.parse("2.9");
        assert_eq!(i.get_int(), 2);
        assert!(i.take_changed());
        i.set_int(7);
        assert_eq!(i.get_int(), 2);
        assert!(!i.take_changed());
        let mut s = ConfigOption::string_values("s", "Huge", &["Small", "Medium"]);
        assert_eq!(s.get_string(), "Small");
        s.set_string(Some("Medium"));
        s.set_string(Some("Bad"));
        assert_eq!(s.get_string(), "Medium");
        let mut m = ConfigOption::string_max("m", "", 3);
        m.set_string(Some("abcdef"));
        assert_eq!(m.get_string(), "abc");
        m.parse("abcdef");
        assert_eq!(m.get_string(), "abcdef");
    }

    #[test]
    fn case_insensitive_order() {
        use std::cmp::Ordering::*;
        assert_eq!(
            case_insensitive_cmp("bPerfReflections", "borderless"),
            Greater
        );
        assert_eq!(case_insensitive_cmp("bPerfReflections", "celsius"), Less);
        assert_eq!(
            case_insensitive_cmp("3DGroundItem", "actionProgressBarSize"),
            Less
        );
        assert_eq!(case_insensitive_cmp("abc", "ABC"), Equal);
    }
}
