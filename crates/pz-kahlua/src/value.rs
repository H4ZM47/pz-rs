//! Lua values with Kahlua semantics.
//!
//! Kahlua represents Lua values as Java objects: `null`, `Boolean`, `Double`,
//! `String`, `KahluaTable`, `LuaClosure`, `JavaFunction`, `Coroutine` and any
//! other exposed Java object. [`Value`] mirrors that set; engine objects are
//! carried as [`Value::Object`].

use crate::table::TableRef;
use crate::vm::{CoroutineRef, LuaClosure, NativeFunction};
use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Lua string. Kahlua strings are Java strings; length and indexing follow
/// UTF-16 code units (see [`crate::jstr`]).
pub type LStr = Rc<str>;

/// An engine object exposed to Lua (the equivalent of an exposed Java object).
pub trait LuaObject: Any {
    /// Fully qualified Java class name this object stands in for, e.g.
    /// `zombie.ui.UIElement`. Used for metatable lookup and `instanceof`.
    fn class_name(&self) -> &str;
    /// Java `toString()`.
    fn to_lua_string(&self) -> String {
        format!("{}@{:x}", self.class_name(), self as *const Self as *const () as usize)
    }
    fn as_any(&self) -> &dyn Any;
    /// Superclasses and interfaces, nearest first, for metatable lookup
    /// when no metatable is registered for `class_name` itself.
    fn super_classes(&self) -> &[&'static str] {
        &[]
    }
}

#[derive(Clone, Default)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    Num(f64),
    Str(LStr),
    Table(TableRef),
    Closure(Rc<LuaClosure>),
    Native(Rc<NativeFunction>),
    Coroutine(CoroutineRef),
    Object(Rc<dyn LuaObject>),
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }
    /// `KahluaUtil.boolEval`: only `nil` and `false` are false.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&TableRef> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
    pub fn is_function(&self) -> bool {
        matches!(self, Value::Closure(_) | Value::Native(_))
    }
    /// `KahluaUtil.type`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Table(_) => "table",
            Value::Closure(_) | Value::Native(_) => "function",
            Value::Coroutine(_) => "coroutine",
            Value::Object(_) => "userdata",
        }
    }
    /// Identity used for Java `==` on references.
    pub fn ptr_id(&self) -> usize {
        match self {
            Value::Table(t) => t.ptr_id(),
            Value::Closure(c) => Rc::as_ptr(c) as *const () as usize,
            Value::Native(f) => Rc::as_ptr(f) as *const () as usize,
            Value::Coroutine(c) => Rc::as_ptr(c) as *const () as usize,
            Value::Object(o) => Rc::as_ptr(o) as *const () as usize,
            _ => 0,
        }
    }
    /// `BaseLib.luaEquals` plus Kahlua's string-content comparison.
    pub fn raw_equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Nil, _) | (_, Value::Nil) => false,
            _ => self.type_name() == other.type_name() && self.ptr_id() == other.ptr_id() && self.ptr_id() != 0,
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Num(n) => write!(f, "{}", crate::number::number_to_string(*n)),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Table(t) => write!(f, "table 0x{:x}", t.ptr_id()),
            Value::Closure(_) => write!(f, "function"),
            Value::Native(n) => write!(f, "{}", n.name),
            Value::Coroutine(_) => write!(f, "coroutine"),
            Value::Object(o) => write!(f, "{}", o.to_lua_string()),
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Num(n)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Num(n as f64)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::str(s)
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(Rc::from(s))
    }
}
impl From<TableRef> for Value {
    fn from(t: TableRef) -> Self {
        Value::Table(t)
    }
}

/// Table key with Java `HashMap` equality: `Double.equals` compares bit
/// patterns after NaN canonicalisation (so `-0.0 != 0.0`, `NaN == NaN`),
/// strings compare by content, everything else by identity.
#[derive(Clone)]
pub struct Key(pub Value);

impl Key {
    fn num_bits(n: f64) -> u64 {
        if n.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            n.to_bits()
        }
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Value::Num(a), Value::Num(b)) => Key::num_bits(*a) == Key::num_bits(*b),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Nil, Value::Nil) => true,
            (a, b) => a.type_name() == b.type_name() && a.ptr_id() == b.ptr_id() && a.ptr_id() != 0,
        }
    }
}
impl Eq for Key {}

impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.0 {
            Value::Nil => 0u8.hash(state),
            Value::Bool(b) => b.hash(state),
            Value::Num(n) => Key::num_bits(*n).hash(state),
            Value::Str(s) => s.hash(state),
            other => other.ptr_id().hash(state),
        }
    }
}
