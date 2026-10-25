//! `KahluaTableImpl`: an insertion-ordered map (Java `LinkedHashMap`).
//!
//! Two Kahlua behaviours differ from reference Lua and are preserved here:
//! * iteration follows insertion order; re-assigning an existing key keeps its
//!   position, removing and re-adding moves it to the end;
//! * `rawget` of a missing key falls through to the metatable's own `rawget`
//!   (recursively), before any `__index` handling.

use crate::value::{Key, Value};
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

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
        self.0.borrow().map.get(&Key(key.clone())).cloned().unwrap_or_default()
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
        self.0.borrow().map.iter().map(|(k, v)| (k.0.clone(), v.clone())).collect()
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
        let keys: Vec<String> = t.keys().iter().map(|k| k.as_str().unwrap().to_string()).collect();
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
