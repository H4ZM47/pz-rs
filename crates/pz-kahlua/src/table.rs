//! `KahluaTableImpl`: an insertion-ordered map (Java `LinkedHashMap`).
//!
//! Two Kahlua behaviours differ from reference Lua and are preserved here:
//! * iteration follows insertion order; re-assigning an existing key keeps its
//!   position, removing and re-adding moves it to the end;
//! * `rawget` of a missing key falls through to the metatable's own `rawget`
//!   (recursively), before any `__index` handling.

use crate::value::{Key, Value};
use indexmap::IndexMap;
use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

/// A host borrow/capability failure, never a Lua missing-key result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableAccessError {
    Borrowed,
    /// The existing bounded metatable walk cannot represent the native
    /// recursive stack-overflow timing.
    TraversalLimit,
}

impl std::fmt::Display for TableAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Borrowed => f.write_str("actual table body is borrowed"),
            Self::TraversalLimit => f.write_str("host metatable traversal limit exceeded"),
        }
    }
}
impl std::error::Error for TableAccessError {}

/// Removed physical slots are held until the caller has left every outer
/// host guard. This is bookkeeping, not a second table or a native reset.
#[derive(Default)]
#[must_use = "retire removed values only after all enclosing host guards end"]
pub struct RetiredTableValues {
    values: Vec<Value>,
}

impl std::fmt::Debug for RetiredTableValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetiredTableValues")
            .field("values", &self.values.len())
            .finish()
    }
}

impl RetiredTableValues {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn push(&mut self, value: Value) {
        self.values.push(value);
    }
    pub fn append(&mut self, other: &mut Self) {
        self.values.append(&mut other.values);
    }
}

/// A real boxed Java Float, distinct from Kahlua's Double numeric value.
/// Lua treats it as userdata, does not coerce it through rawTonumber, and
/// compares it by reference. Java map-key equality is handled separately.
#[derive(Debug)]
pub struct BoxedFloat(f32);

impl BoxedFloat {
    pub fn new(value: f32) -> Rc<Self> {
        let owner = Rc::new(Self(value));
        // Like table allocation registration, an unavailable registry leaves
        // this immutable scalar conservative. The explicit observer permits
        // an outer host to retry without replacing the actual allocation.
        let _ = Self::observe(&owner);
        owner
    }
    pub fn observe(owner: &Rc<Self>) -> Result<bool, crate::gc::RegistrationError> {
        crate::gc::register_userdata::<Self, BoxedFloatDescriptor>(owner)
    }
    pub fn value(value: f32) -> Value {
        Value::Object(Self::new(value))
    }
    pub fn get(&self) -> f32 {
        self.0
    }
    pub fn canonical_bits(&self) -> u32 {
        if self.0.is_nan() {
            0x7fc0_0000
        } else {
            self.0.to_bits()
        }
    }
}

struct BoxedFloatDescriptor;
impl crate::gc::UserdataDescriptor<BoxedFloat> for BoxedFloatDescriptor {
    fn trace(
        _: &BoxedFloat,
        _: &mut crate::gc::TraceVisitor,
    ) -> Result<(), crate::gc::TraceDeferred> {
        Ok(())
    }
}

impl crate::LuaObject for BoxedFloat {
    fn class_name(&self) -> &str {
        "java.lang.Float"
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.lang.Number", "java.lang.Object"]
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn to_lua_string(&self) -> String {
        let value = self.0;
        if value.is_nan() {
            return "NaN".into();
        }
        if value.is_infinite() {
            return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
        }
        if value == 0.0 {
            return if value.is_sign_negative() {
                "-0.0"
            } else {
                "0.0"
            }
            .into();
        }
        // Same shortest-f32 formatting convention as the existing core
        // Float formatter. Exact cross-JDK shortest-decimal tie cases are
        // outside this boxed-value identity/persistence slice.
        let scientific = format!("{:e}", value.abs());
        let (mantissa, exponent) = scientific.split_once('e').expect("Rust scientific format");
        let exponent: i32 = exponent.parse().expect("Rust scientific exponent");
        let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
        let sign = if value < 0.0 { "-" } else { "" };
        if (1.0e-3..1.0e7).contains(&value.abs()) {
            let point = exponent + 1;
            let body = if point <= 0 {
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
            format!("{sign}{body}")
        } else {
            let fraction = if digits.len() > 1 { &digits[1..] } else { "0" };
            format!("{sign}{}.{}E{exponent}", &digits[..1], fraction)
        }
    }
}

#[derive(Default)]
pub struct Table {
    pub(crate) map: IndexMap<Key, Value>,
    pub(crate) meta: Option<TableRef>,
}

#[derive(Clone)]
pub struct TableRef(pub(crate) Rc<RefCell<Table>>);

impl Default for TableRef {
    fn default() -> Self {
        let table = Rc::new(RefCell::new(Table::default()));
        crate::gc::register_table(Rc::downgrade(&table));
        Self(table)
    }
}

impl std::fmt::Debug for TableRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "table 0x{:x}", self.ptr_id())
    }
}

impl TableRef {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn ptr_id(&self) -> usize {
        Rc::as_ptr(&self.0) as *const () as usize
    }
    pub fn ptr_eq(&self, other: &TableRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    /// Borrowing the body is separate from cloning the pointer. An existing
    /// item-table getter can return the pointer even while this guard is held.
    pub fn try_borrow_body(&self) -> Result<Ref<'_, Table>, TableAccessError> {
        self.0.try_borrow().map_err(|_| TableAccessError::Borrowed)
    }
    pub fn try_borrow_body_mut(&self) -> Result<RefMut<'_, Table>, TableAccessError> {
        self.0
            .try_borrow_mut()
            .map_err(|_| TableAccessError::Borrowed)
    }
    pub fn try_metatable(&self) -> Result<Option<TableRef>, TableAccessError> {
        Ok(self.try_borrow_body()?.meta.clone())
    }
    pub fn try_set_metatable(
        &self,
        meta: &Option<TableRef>,
        retired: &mut RetiredTableValues,
    ) -> Result<(), TableAccessError> {
        let old = {
            let mut body = self.try_borrow_body_mut()?;
            std::mem::replace(&mut body.meta, meta.clone())
        };
        if let Some(old) = old {
            retired.push(Value::Table(old));
        }
        Ok(())
    }

    /// Native raw lookup, including the existing metatable fall-through.
    /// A held body is an explicit host fault, never an invented nil.
    pub fn try_rawget(&self, key: &Value) -> Result<Value, TableAccessError> {
        if key.is_nil() {
            return Ok(Value::Nil);
        }
        let mut current = self.clone();
        for _ in 0..1000 {
            let next = {
                let body = current.try_borrow_body()?;
                if let Some(value) = body.map.get(&Key(key.clone())) {
                    return Ok(value.clone());
                }
                body.meta.clone()
            };
            match next {
                Some(next) => current = next,
                None => return Ok(Value::Nil),
            }
        }
        Err(TableAccessError::TraversalLimit)
    }
    pub fn try_get_own(&self, key: &Value) -> Result<Value, TableAccessError> {
        Ok(self
            .try_borrow_body()?
            .map
            .get(&Key(key.clone()))
            .cloned()
            .unwrap_or_default())
    }
    pub fn try_contains_own(&self, key: &Value) -> Result<bool, TableAccessError> {
        Ok(self.try_borrow_body()?.map.contains_key(&Key(key.clone())))
    }
    pub fn try_size(&self) -> Result<usize, TableAccessError> {
        Ok(self.try_borrow_body()?.map.len())
    }
    pub fn try_is_empty(&self) -> Result<bool, TableAccessError> {
        Ok(self.try_borrow_body()?.map.is_empty())
    }
    pub fn try_keys(&self) -> Result<Vec<Value>, TableAccessError> {
        Ok(self
            .try_borrow_body()?
            .map
            .keys()
            .map(|key| key.0.clone())
            .collect())
    }
    pub fn try_entries(&self) -> Result<Vec<(Value, Value)>, TableAccessError> {
        Ok(self
            .try_borrow_body()?
            .map
            .iter()
            .map(|(key, value)| (key.0.clone(), value.clone()))
            .collect())
    }
    /// Inputs remain caller-owned on both success and failure. Replacing an
    /// existing value keeps the original key allocation and insertion order.
    pub fn try_rawset(
        &self,
        key: &Value,
        value: &Value,
        retired: &mut RetiredTableValues,
    ) -> Result<(), TableAccessError> {
        if value.is_nil() {
            let removed = self
                .try_borrow_body_mut()?
                .map
                .shift_remove_entry(&Key(key.clone()));
            if let Some((key, value)) = removed {
                retired.push(key.0);
                retired.push(value);
            }
        } else {
            let old = self
                .try_borrow_body_mut()?
                .map
                .insert(Key(key.clone()), value.clone());
            if let Some(old) = old {
                retired.push(old);
            }
        }
        Ok(())
    }
    /// Wipe only the own delegate, retaining the metatable and every removed
    /// owning key/value until the caller retires them outside outer guards.
    pub fn try_wipe(&self, retired: &mut RetiredTableValues) -> Result<(), TableAccessError> {
        let old = {
            let mut body = self.try_borrow_body_mut()?;
            std::mem::take(&mut body.map)
        };
        for (key, value) in old {
            retired.push(key.0);
            retired.push(value);
        }
        Ok(())
    }
    pub fn metatable(&self) -> Option<TableRef> {
        self.0.borrow().meta.clone()
    }
    pub fn set_metatable(&self, meta: Option<TableRef>) {
        self.0.borrow_mut().meta = meta;
    }

    /// `KahluaTableImpl.rawget`, including the metatable fall-through.
    pub fn rawget(&self, key: &Value) -> Value {
        if key.is_nil() {
            return Value::Nil;
        }
        let mut cur = self.clone();
        // Bounded like a Java stack; metatable chains are short in practice.
        for _ in 0..1000 {
            let next = {
                let t = cur.0.borrow();
                if let Some(v) = t.map.get(&Key(key.clone())) {
                    return v.clone();
                }
                match &t.meta {
                    Some(m) => m.clone(),
                    None => return Value::Nil,
                }
            };
            cur = next;
        }
        Value::Nil
    }

    /// Lookup in this table only, ignoring the metatable.
    pub fn get_own(&self, key: &Value) -> Value {
        self.0
            .borrow()
            .map
            .get(&Key(key.clone()))
            .cloned()
            .unwrap_or_default()
    }

    pub fn contains_own(&self, key: &Value) -> bool {
        self.0.borrow().map.contains_key(&Key(key.clone()))
    }

    /// `KahluaTableImpl.rawset`. A nil value removes the key; a nil key is
    /// stored like any other (Java `LinkedHashMap` permits `null`).
    pub fn rawset(&self, key: Value, value: Value) {
        let mut t = self.0.borrow_mut();
        if value.is_nil() {
            t.map.shift_remove(&Key(key));
        } else {
            t.map.insert(Key(key), value);
        }
    }

    pub fn get_str(&self, key: &str) -> Value {
        self.rawget(&Value::str(key))
    }
    pub fn set_str(&self, key: &str, value: impl Into<Value>) {
        self.rawset(Value::str(key), value.into());
    }
    pub fn get_int(&self, i: i64) -> Value {
        self.rawget(&Value::Num(i as f64))
    }
    pub fn set_int(&self, i: i64, value: impl Into<Value>) {
        self.rawset(Value::Num(i as f64), value.into());
    }

    /// `KahluaUtil.len(t, 0, 2 * size)`: binary search then linear probe,
    /// using the metatable-aware `rawget`.
    pub fn len(&self) -> i64 {
        let mut low: i64 = 0;
        let mut high: i64 = 2 * self.size() as i64;
        while low < high {
            let middle = (high + low + 1) >> 1;
            if self.get_int(middle).is_nil() {
                high = middle - 1;
            } else {
                low = middle;
            }
        }
        while !self.get_int(low + 1).is_nil() {
            low += 1;
        }
        low
    }

    pub fn size(&self) -> usize {
        self.0.borrow().map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.borrow().map.is_empty()
    }
    pub fn wipe(&self) {
        self.0.borrow_mut().map.clear();
    }

    /// Snapshot of keys, as `KahluaTableImpl.iterator()` takes one.
    pub fn keys(&self) -> Vec<Value> {
        self.0.borrow().map.keys().map(|k| k.0.clone()).collect()
    }

    /// Snapshot of entries in iteration order.
    pub fn entries(&self) -> Vec<(Value, Value)> {
        self.0
            .borrow()
            .map
            .iter()
            .map(|(k, v)| (k.0.clone(), v.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_order_and_reinsert() {
        let t = TableRef::new();
        t.set_str("b", 1);
        t.set_str("a", 2);
        t.set_str("b", 3);
        t.set_str("c", 4);
        t.rawset(Value::str("a"), Value::Nil);
        t.set_str("a", 5);
        let keys: Vec<String> = t
            .keys()
            .iter()
            .map(|k| k.as_str().unwrap().to_string())
            .collect();
        assert_eq!(keys, ["b", "c", "a"]);
    }

    #[test]
    fn rawget_falls_through_to_metatable() {
        let base = TableRef::new();
        base.set_str("x", 7);
        let t = TableRef::new();
        t.set_metatable(Some(base));
        assert_eq!(t.get_str("x").as_num(), Some(7.0));
        assert!(t.get_own(&Value::str("x")).is_nil());
    }

    #[test]
    fn length_and_negative_zero_keys() {
        let t = TableRef::new();
        for i in 1..=5 {
            t.set_int(i, i as f64);
        }
        assert_eq!(t.len(), 5);
        t.rawset(Value::Num(-0.0), Value::str("neg"));
        assert!(t.rawget(&Value::Num(0.0)).is_nil());
    }
}

#[cfg(test)]
#[path = "table_native_tests.rs"]
mod native_tests;
