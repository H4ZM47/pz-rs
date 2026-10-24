//! The animation variable system at the level the animator needs:
//! `AnimationVariableHandle`/`HandlePool` (global, case-insensitive names
//! with stable indices), the slot value types and their conversions,
//! `IAnimationVariableSource` / `IAnimationVariableMap` as one trait, the
//! map-backed `AnimationVariableSource`, and `AnimationVariableReference`
//! (`$subSource.name` references).
//!
//! Slots are read as [`VariableValue`] snapshots carrying the slot's type,
//! so the conversions (`getValueFloat` of a string slot, `getValueString`
//! of a float slot, ...) behave like the original slot classes. Character
//! code implements [`AnimationVariableSource`] over its own state; the
//! callback slots of the original become computed values there.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

/// `AnimationVariableHandle`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AnimationVariableHandle {
    name: Arc<str>,
    index: i32,
}

impl AnimationVariableHandle {
    /// `AnimationVariableHandle.alloc` / `AnimationVariableHandlePool.getOrCreate`:
    /// `None` for blank names.
    pub fn alloc(name: &str) -> Option<AnimationVariableHandle> {
        if is_null_or_whitespace(name) {
            return None;
        }
        let key = java_trim(name);
        static POOL: OnceLock<Mutex<(HashMap<String, AnimationVariableHandle>, i32)>> = OnceLock::new();
        let mut pool = POOL.get_or_init(|| Mutex::new((HashMap::new(), 0))).lock().unwrap();
        let lc = key.to_lowercase();
        if let Some(h) = pool.0.get(&lc) {
            return Some(h.clone());
        }
        pool.1 += 1;
        let h = AnimationVariableHandle { name: Arc::from(key), index: pool.1 };
        pool.0.insert(lc, h.clone());
        Some(h)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn index(&self) -> i32 {
        self.index
    }
}

/// `AnimationVariableType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationVariableType {
    String,
    Float,
    Int,
    Boolean,
}

/// A slot's current value, tagged with the slot's type.
#[derive(Clone, Debug, PartialEq)]
pub enum VariableValue {
    String(String),
    Float(f32),
    Int(i32),
    Bool(bool),
}

impl VariableValue {
    pub fn kind(&self) -> AnimationVariableType {
        match self {
            VariableValue::String(_) => AnimationVariableType::String,
            VariableValue::Float(_) => AnimationVariableType::Float,
            VariableValue::Int(_) => AnimationVariableType::Int,
            VariableValue::Bool(_) => AnimationVariableType::Boolean,
        }
    }

    /// `getValueString`.
    pub fn as_string(&self) -> String {
        match self {
            VariableValue::String(s) => s.clone(),
            VariableValue::Float(f) => java_float_to_string(*f),
            VariableValue::Int(i) => i.to_string(),
            VariableValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        }
    }

    /// `getValueFloat`.
    pub fn as_float(&self) -> f32 {
        match self {
            VariableValue::String(s) => try_parse_float(s, 0.0),
            VariableValue::Float(f) => *f,
            VariableValue::Int(i) => *i as f32,
            VariableValue::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// `getValueBool`.
    pub fn as_bool(&self) -> bool {
        match self {
            VariableValue::String(s) => try_parse_boolean(s),
            VariableValue::Float(f) => *f != 0.0,
            VariableValue::Int(i) => *i != 0,
            VariableValue::Bool(b) => *b,
        }
    }

    /// `getValueInt`.
    pub fn as_int(&self) -> i32 {
        match self {
            VariableValue::String(s) => java_trim(s).parse().unwrap_or(0),
            VariableValue::Float(f) => *f as i32,
            VariableValue::Int(i) => *i,
            VariableValue::Bool(b) => *b as i32,
        }
    }

    /// The value a slot of type `kind` holds after `setValue(self)`.
    pub fn converted_to(&self, kind: AnimationVariableType) -> VariableValue {
        match kind {
            AnimationVariableType::String => VariableValue::String(match self {
                VariableValue::Float(f) => java_float_to_string(*f),
                other => other.as_string(),
            }),
            AnimationVariableType::Float => VariableValue::Float(self.as_float()),
            AnimationVariableType::Int => VariableValue::Int(match self {
                VariableValue::String(s) => java_trim(s).parse().unwrap_or(0),
                other => other.as_int(),
            }),
            AnimationVariableType::Boolean => VariableValue::Bool(match self {
                VariableValue::Float(f) => *f as f64 != 0.0,
                other => other.as_bool(),
            }),
        }
    }

    /// The value after `clear()`.
    pub fn cleared(&self) -> VariableValue {
        match self {
            VariableValue::String(_) => VariableValue::String(String::new()),
            VariableValue::Float(_) => VariableValue::Float(0.0),
            VariableValue::Int(_) => VariableValue::Int(0),
            VariableValue::Bool(_) => VariableValue::Bool(false),
        }
    }
}

impl From<&str> for VariableValue {
    fn from(s: &str) -> Self {
        VariableValue::String(s.to_string())
    }
}
impl From<f32> for VariableValue {
    fn from(f: f32) -> Self {
        VariableValue::Float(f)
    }
}
impl From<bool> for VariableValue {
    fn from(b: bool) -> Self {
        VariableValue::Bool(b)
    }
}

/// `IAnimationVariableSource` (+ the setters of `IAnimationVariableMap`).
pub trait AnimationVariableSource {
    /// `getVariable(AnimationVariableHandle)`: the slot's value, `None` if
    /// there is no such slot.
    fn get_variable(&self, handle: &AnimationVariableHandle) -> Option<VariableValue>;

    /// `slot.setValue(value)` on an existing slot (converting to the slot's
    /// type). Returns `false` when there is no slot.
    fn set_existing_variable(&mut self, _handle: &AnimationVariableHandle, _value: VariableValue) -> bool {
        false
    }

    /// `IAnimationVariableMap.setVariable(name, value)`: creates the slot
    /// with the value's type when missing. Returns `false` when the source
    /// is read-only (not an `IAnimationVariableMap`).
    fn set_variable(&mut self, _name: &str, _value: VariableValue) -> bool {
        false
    }

    /// `slot.clear()` / `IAnimationVariableMap.clearVariable`.
    fn clear_variable(&mut self, _name: &str) -> bool {
        false
    }

    /// `getSubVariableSource`.
    fn sub_variable_source(&self, _name: &str) -> Option<&dyn AnimationVariableSource> {
        None
    }

    fn sub_variable_source_mut(&mut self, _name: &str) -> Option<&mut dyn AnimationVariableSource> {
        None
    }

    /// `setVariable(String, String)`.
    fn set_variable_string(&mut self, name: &str, value: &str) -> bool {
        self.set_variable(name, VariableValue::String(value.to_string()))
    }

    /// `setVariable(String, float)`.
    fn set_variable_float(&mut self, name: &str, value: f32) -> bool {
        self.set_variable(name, VariableValue::Float(value))
    }

    /// `setVariable(String, boolean)`.
    fn set_variable_bool(&mut self, name: &str, value: bool) -> bool {
        self.set_variable(name, VariableValue::Bool(value))
    }

    /// `getVariable(String)`.
    fn get_variable_named(&self, key: &str) -> Option<VariableValue> {
        AnimationVariableHandle::alloc(key).and_then(|h| self.get_variable(&h))
    }

    /// `getVariableString`.
    fn get_variable_string(&self, key: &str) -> String {
        self.get_variable_named(key).map(|v| v.as_string()).unwrap_or_default()
    }

    /// `getVariableFloat`.
    fn get_variable_float(&self, key: &str, default: f32) -> f32 {
        self.get_variable_named(key).map(|v| v.as_float()).unwrap_or(default)
    }

    /// `getVariableBoolean(String)`.
    fn get_variable_boolean(&self, key: &str) -> bool {
        self.get_variable_named(key).is_some_and(|v| v.as_bool())
    }

    /// `getVariableBoolean(AnimationVariableHandle)`.
    fn get_variable_boolean_handle(&self, handle: &AnimationVariableHandle) -> bool {
        self.get_variable(handle).is_some_and(|v| v.as_bool())
    }

    /// `isVariable(name, value)`.
    fn is_variable(&self, name: &str, val: &str) -> bool {
        equals_ignore_case(&self.get_variable_string(name), val)
    }

    /// `containsVariable`.
    fn contains_variable(&self, key: &str) -> bool {
        self.get_variable_named(key).is_some()
    }
}

/// `AnimationVariableSource`: slots indexed by handle.
#[derive(Clone, Debug, Default)]
pub struct MapVariableSource {
    slots: Vec<Option<Slot>>,
}

#[derive(Clone, Debug)]
struct Slot {
    key: String,
    value: VariableValue,
    read_only: bool,
}

impl MapVariableSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|s| s.is_none())
    }

    fn slot_mut(&mut self, handle: &AnimationVariableHandle) -> Option<&mut Slot> {
        let idx = handle.index();
        if idx < 0 {
            return None;
        }
        self.slots.get_mut(idx as usize).and_then(|s| s.as_mut())
    }

    /// `setVariable(IAnimationVariableSlot)` with a fresh typed slot.
    pub fn insert_slot(&mut self, name: &str, value: VariableValue) {
        let Some(h) = AnimationVariableHandle::alloc(name) else { return };
        let idx = h.index() as usize;
        if idx >= self.slots.len() {
            self.slots.resize(idx + 1, None);
        }
        self.slots[idx] = Some(Slot { key: java_trim(name).to_lowercase(), value, read_only: false });
    }

    pub fn set_read_only(&mut self, name: &str, read_only: bool) {
        if let Some(h) = AnimationVariableHandle::alloc(name) {
            if let Some(s) = self.slot_mut(&h) {
                s.read_only = read_only;
            }
        }
    }

    /// `removeAllVariables`.
    pub fn remove_all_variables(&mut self) {
        self.slots.clear();
    }

    /// `getGameVariables`: (key, value) in handle order.
    pub fn game_variables(&self) -> impl Iterator<Item = (&str, &VariableValue)> {
        self.slots.iter().flatten().map(|s| (s.key.as_str(), &s.value))
    }
}

impl AnimationVariableSource for MapVariableSource {
    fn get_variable(&self, handle: &AnimationVariableHandle) -> Option<VariableValue> {
        let idx = handle.index();
        if idx < 0 {
            return None;
        }
        self.slots.get(idx as usize).and_then(|s| s.as_ref()).map(|s| s.value.clone())
    }

    fn set_existing_variable(&mut self, handle: &AnimationVariableHandle, value: VariableValue) -> bool {
        match self.slot_mut(handle) {
            Some(s) => {
                if !s.read_only {
                    s.value = value.converted_to(s.value.kind());
                }
                true
            }
            None => false,
        }
    }

    fn set_variable(&mut self, name: &str, value: VariableValue) -> bool {
        let Some(h) = AnimationVariableHandle::alloc(name) else { return true };
        if !self.set_existing_variable(&h, value.clone()) {
            let kind = match &value {
                VariableValue::Int(i) => VariableValue::Float(*i as f32),
                v => v.clone(),
            };
            self.insert_slot(name, kind);
        }
        true
    }

    fn clear_variable(&mut self, name: &str) -> bool {
        if let Some(h) = AnimationVariableHandle::alloc(name) {
            if let Some(s) = self.slot_mut(&h) {
                if !s.read_only {
                    s.value = s.value.cleared();
                }
            }
        }
        true
    }
}

/// `AnimationVariableReference`.
#[derive(Clone, Debug)]
pub struct AnimationVariableReference {
    sub_variable_source_name: Option<String>,
    name: String,
    handle: Option<AnimationVariableHandle>,
}

/// References are shared by the events and conditions that own them; the
/// while-alive flags of a live node compare them by identity like the
/// original.
pub type VariableReferenceRc = Rc<AnimationVariableReference>;

impl AnimationVariableReference {
    /// `fromRawVariableName`.
    pub fn from_raw_variable_name(raw: &str) -> AnimationVariableReference {
        let mut r = AnimationVariableReference { sub_variable_source_name: None, name: raw.to_string(), handle: None };
        r.parse();
        r.handle = AnimationVariableHandle::alloc(&r.name);
        r
    }

    fn parse(&mut self) {
        let raw = self.name.clone();
        if !raw.starts_with('$') {
            return;
        }
        let Some(dot) = raw.find('.') else { return };
        if dot <= 1 {
            return;
        }
        let sub = &raw[1..dot];
        let var = &raw[dot + 1..];
        if is_null_or_whitespace(var) || !is_valid_variable_name(sub) {
            return;
        }
        self.sub_variable_source_name = Some(sub.to_string());
        self.name = var.to_string();
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn sub_variable_source_name(&self) -> Option<&str> {
        self.sub_variable_source_name.as_deref()
    }

    /// `equals(AnimationVariableReference)`.
    pub fn equals(&self, rhs: &AnimationVariableReference) -> bool {
        equals_ignore_case(self.sub_variable_source_name.as_deref().unwrap_or(""), rhs.sub_variable_source_name.as_deref().unwrap_or(""))
            && equals_ignore_case(&self.name, &rhs.name)
            && self.handle == rhs.handle
    }

    fn source<'a>(&self, src: &'a dyn AnimationVariableSource) -> Option<&'a dyn AnimationVariableSource> {
        match &self.sub_variable_source_name {
            Some(n) => src.sub_variable_source(n),
            None => Some(src),
        }
    }

    fn source_mut<'a>(&self, src: &'a mut dyn AnimationVariableSource) -> Option<&'a mut dyn AnimationVariableSource> {
        match &self.sub_variable_source_name {
            Some(n) => src.sub_variable_source_mut(n),
            None => Some(src),
        }
    }

    /// `getVariable(IAnimationVariableSource)`.
    pub fn get_variable(&self, src: &dyn AnimationVariableSource) -> Option<VariableValue> {
        if self.name.trim().is_empty() {
            return None;
        }
        let h = self.handle.as_ref()?;
        self.source(src)?.get_variable(h)
    }

    /// `setVariable(owner, value)`.
    pub fn set_variable(&self, owner: &mut dyn AnimationVariableSource, value: VariableValue) {
        if self.name.trim().is_empty() {
            return;
        }
        let Some(src) = self.source_mut(owner) else { return };
        if let Some(h) = &self.handle {
            if src.set_existing_variable(h, value.clone()) {
                return;
            }
        }
        src.set_variable(&self.name, value);
    }

    /// `clearVariable(owner)`.
    pub fn clear_variable(&self, owner: &mut dyn AnimationVariableSource) {
        if self.name.trim().is_empty() {
            return;
        }
        if let Some(src) = self.source_mut(owner) {
            src.clear_variable(&self.name);
        }
    }
}

// ---- string helpers shared with the XML and condition code ----

/// `String.trim()`: strips chars `<= ' '`.
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

pub fn is_null_or_whitespace(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// `StringUtils.equalsIgnoreCase` (null and "" compare equal).
pub fn equals_ignore_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || (!a.is_ascii() || !b.is_ascii()) && a.to_lowercase() == b.to_lowercase()
}

pub fn is_valid_variable_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// `StringUtils.tryParseBoolean`.
pub fn try_parse_boolean(s: &str) -> bool {
    if is_null_or_whitespace(s) {
        return false;
    }
    let p = java_trim(s);
    p.eq_ignore_ascii_case("true") || p == "1" || p == "1.0"
}

/// `Float.parseFloat` (after `trim`); `None` where it throws.
pub fn java_parse_float(s: &str) -> Option<f32> {
    let t = java_trim(s);
    let body = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    match body.trim_start_matches(['+', '-']) {
        "NaN" => return Some(f32::NAN),
        "Infinity" => return Some(if body.starts_with('-') { f32::NEG_INFINITY } else { f32::INFINITY }),
        _ => {}
    }
    if body.chars().any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E') {
        return None;
    }
    body.parse::<f32>().ok()
}

/// `PZMath.tryParseFloat(s, default)`.
pub fn try_parse_float(s: &str, default: f32) -> f32 {
    if is_null_or_whitespace(s) {
        return default;
    }
    java_parse_float(s).unwrap_or(default)
}

/// `Float.toString(float)` / `String.valueOf(float)`.
pub fn java_float_to_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let sci = format!("{:e}", f);
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    let a = f.abs();
    if (1.0e-3..1.0e7).contains(&a) {
        if exp >= 0 {
            let int_len = (exp + 1) as usize;
            let mut d = digits.clone();
            while d.len() < int_len {
                d.push('0');
            }
            let (i, frac) = d.split_at(int_len);
            out.push_str(i);
            out.push('.');
            out.push_str(if frac.is_empty() { "0" } else { frac });
        } else {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        }
    } else {
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        out.push('.');
        out.push_str(if rest.is_empty() { "0" } else { rest });
        out.push('E');
        out.push_str(&exp.to_string());
    }
    out
}
