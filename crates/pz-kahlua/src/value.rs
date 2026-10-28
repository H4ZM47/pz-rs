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
        format!(
            "{}@{:x}",
            self.class_name(),
            self as *const Self as *const () as usize
        )
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
            _ => {
                self.type_name() == other.type_name()
                    && self.ptr_id() == other.ptr_id()
                    && self.ptr_id() != 0
            }
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
/// boxed Float keys use Float.equals, strings compare by content and
/// other objects compare by identity. This differs from Lua raw equality.
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
            (Value::Object(a), Value::Object(b)) => {
                match (
                    (&**a as &dyn Any).downcast_ref::<crate::table::BoxedFloat>(),
                    (&**b as &dyn Any).downcast_ref::<crate::table::BoxedFloat>(),
                ) {
                    (Some(a), Some(b)) => a.canonical_bits() == b.canonical_bits(),
                    _ => Rc::ptr_eq(a, b),
                }
            }
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
            Value::Object(object) => {
                if let Some(value) =
                    (&**object as &dyn Any).downcast_ref::<crate::table::BoxedFloat>()
                {
                    value.canonical_bits().hash(state);
                } else {
                    self.0.ptr_id().hash(state);
                }
            }
            other => other.ptr_id().hash(state),
        }
    }
}

#[cfg(test)]
mod boxed_float_key_tests {
    struct Opaque;
    impl super::LuaObject for Opaque {
        fn class_name(&self) -> &str {
            panic!("key bookkeeping called class_name")
        }
        fn to_lua_string(&self) -> String {
            panic!("key bookkeeping called to_lua_string")
        }
        fn as_any(&self) -> &dyn std::any::Any {
            panic!("key bookkeeping called as_any")
        }
        fn super_classes(&self) -> &[&'static str] {
            panic!("key bookkeeping called super_classes")
        }
    }

    #[test]
    fn ordinary_userdata_keys_never_invoke_overridable_host_methods() {
        let table = super::TableRef::new();
        let first = super::Value::Object(std::rc::Rc::new(Opaque));
        let second = super::Value::Object(std::rc::Rc::new(Opaque));
        table.rawset(first.clone(), super::Value::Num(1.0));
        table.rawset(second.clone(), super::Value::Num(2.0));
        table.rawset(first.clone(), super::Value::Num(3.0));
        assert_eq!(table.rawget(&first).as_num(), Some(3.0));
        assert_eq!(table.rawget(&second).as_num(), Some(2.0));
        assert_eq!(table.size(), 2);
        assert!(table.entries()[0].0.raw_equals(&first));
    }

    use super::*;
    use crate::table::BoxedFloat;

    #[test]
    fn equal_float_keys_replace_values_but_keep_the_first_actual_key_and_remain_distinct_from_double(
    ) {
        let first = BoxedFloat::value(1.0);
        let second = BoxedFloat::value(1.0);
        assert!(!first.raw_equals(&second));
        let table = TableRef::new();
        table.rawset(first.clone(), Value::str("first"));
        table.rawset(second.clone(), Value::str("replacement"));
        table.rawset(Value::Num(1.0), Value::str("double"));
        let entries = table.entries();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].0.raw_equals(&first));
        assert!(!entries[0].0.raw_equals(&second));
        assert_eq!(table.rawget(&second).as_str(), Some("replacement"));
        assert_eq!(table.rawget(&Value::Num(1.0)).as_str(), Some("double"));
        assert_eq!(first.type_name(), "userdata");
        assert_eq!(first.as_num(), None);
    }

    #[test]
    fn float_key_signed_zeros_are_distinct_and_nan_payloads_share_java_canonical_equality() {
        let table = TableRef::new();
        let plus = BoxedFloat::value(0.0);
        let minus = BoxedFloat::value(-0.0);
        table.rawset(plus.clone(), Value::str("positive"));
        table.rawset(minus.clone(), Value::str("negative"));
        let nan1 = BoxedFloat::value(f32::from_bits(0x7fc0_1234));
        let nan2 = BoxedFloat::value(f32::from_bits(0xffc0_5678));
        table.rawset(nan1.clone(), Value::str("nan1"));
        table.rawset(nan2.clone(), Value::str("nan2"));
        assert!(!nan1.raw_equals(&nan2));
        assert_eq!(table.entries().len(), 3);
        assert_eq!(table.rawget(&plus).as_str(), Some("positive"));
        assert_eq!(table.rawget(&minus).as_str(), Some("negative"));
        assert_eq!(table.rawget(&nan1).as_str(), Some("nan2"));
        assert!(table.entries()[2].0.raw_equals(&nan1));
    }
}
