//! `java.util` collections as Lua sees them: `ArrayList`, `HashMap`,
//! `LinkedHashMap`, `HashSet`, their key/value/entry views and iterators.
//!
//! Elements are Lua values. Equality and hashing follow Java's boxed types
//! (`Double.equals`, `String.hashCode`, ...), and `HashMap`/`HashSet`
//! iterate in Java's bucket order (`hash ^ hash >>> 16` masked by the table
//! capacity, insertion order within a bucket), so Lua code walking a map
//! sees keys in the same order as in the original. Reference-typed keys
//! (tables, objects) use identity, whose hash is arbitrary in Java too.
//! Views (`keySet`, `values`, `entrySet`) are snapshots.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use indexmap::IndexMap;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};

use crate::expose::{expose_class, int_arg, this};

pub const ARRAY_LIST: &str = "java.util.ArrayList";
pub const HASH_MAP: &str = "java.util.HashMap";
pub const LINKED_HASH_MAP: &str = "java.util.LinkedHashMap";
pub const HASH_SET: &str = "java.util.HashSet";
pub const ITERATOR: &str = "java.util.Iterator";
pub const ENTRY: &str = "java.util.Map$Entry";

// ---- Java value semantics ---------------------------------------------------

/// A hashable stand-in for a key with Java `equals` semantics.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Null,
    Bool(bool),
    /// `Double.doubleToLongBits`: NaNs are one value, `-0.0 != 0.0`.
    Num(u64),
    Str(Rc<str>),
    Ref(&'static str, usize),
}

fn key_of(v: &Value) -> Key {
    match v {
        Value::Nil => Key::Null,
        Value::Bool(b) => Key::Bool(*b),
        Value::Num(n) => Key::Num(if n.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            n.to_bits()
        }),
        Value::Str(s) => Key::Str(s.clone()),
        other => Key::Ref(other.type_name(), other.ptr_id()),
    }
}

/// `Object.equals` between two boxed Lua values.
pub fn java_equals(a: &Value, b: &Value) -> bool {
    key_of(a) == key_of(b)
}

/// `Object.hashCode`.
fn java_hash(k: &Key) -> i32 {
    match k {
        Key::Null => 0,
        Key::Bool(b) => {
            if *b {
                1231
            } else {
                1237
            }
        }
        Key::Num(bits) => (*bits ^ (*bits >> 32)) as i32,
        Key::Str(s) => s
            .encode_utf16()
            .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c))),
        Key::Ref(_, p) => (*p as u64 ^ (*p as u64 >> 32)) as i32,
    }
}

/// `String.valueOf(o)` for collection `toString`.
fn java_to_string(v: &Value) -> String {
    match v {
        Value::Nil => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => pz_kahlua::number::java_double_to_string(*n),
        Value::Str(s) => s.to_string(),
        Value::Object(o) => o.to_lua_string(),
        other => format!(
            "{}@{:x}",
            pz_kahlua::stdlib::java_class_name(other),
            other.ptr_id()
        ),
    }
}

fn index_error(i: i32, len: usize) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {i} out of bounds for length {len}"),
        class: "java.lang.IndexOutOfBoundsException",
    }
}

fn no_such_element() -> LuaError {
    LuaError::Runtime {
        message: "No such element".into(),
        class: "java.util.NoSuchElementException",
    }
}

// ---- objects ------------------------------------------------------------------

/// `java.util.ArrayList`.
pub struct JavaList {
    pub items: RefCell<Vec<Value>>,
}

/// `java.util.HashMap` / `LinkedHashMap` (and, without values, `HashSet`).
pub struct JavaMap {
    class: &'static str,
    linked: bool,
    entries: RefCell<IndexMap<Key, (Value, Value)>>,
    /// Table capacity; 0 until the first insertion allocates it.
    capacity: Cell<usize>,
    initial: usize,
}

/// `java.util.Iterator` over a snapshot.
pub struct JavaIterator {
    items: Vec<Value>,
    pos: Cell<usize>,
}

/// `java.util.Map$Entry`.
pub struct JavaEntry {
    pub key: Value,
    pub value: RefCell<Value>,
}

macro_rules! lua_object {
    ($t:ty, $self:ident => $class:expr, $supers:expr) => {
        impl LuaObject for $t {
            fn class_name(&$self) -> &str {
                $class
            }
            fn to_lua_string(&$self) -> String {
                $self.java_string()
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn super_classes(&self) -> &[&'static str] {
                $supers
            }
        }
    };
}

lua_object!(JavaList, self => ARRAY_LIST, &["java.util.AbstractList", "java.util.List", "java.util.Collection"]);
lua_object!(JavaMap, self => self.class, &[HASH_MAP, "java.util.AbstractMap", "java.util.Map"]);
lua_object!(JavaIterator, self => ITERATOR, &[]);
lua_object!(JavaEntry, self => ENTRY, &[]);

impl JavaList {
    pub fn new(items: Vec<Value>) -> Self {
        JavaList {
            items: RefCell::new(items),
        }
    }

    fn java_string(&self) -> String {
        let parts: Vec<String> = self.items.borrow().iter().map(java_to_string).collect();
        format!("[{}]", parts.join(", "))
    }
}

/// `HashMap.tableSizeFor`.
fn table_size_for(cap: usize) -> usize {
    cap.max(1).next_power_of_two().min(1 << 30)
}

impl JavaMap {
    fn with_class(class: &'static str, initial_capacity: usize) -> Self {
        JavaMap {
            class,
            linked: class == LINKED_HASH_MAP,
            entries: RefCell::new(IndexMap::new()),
            capacity: Cell::new(0),
            initial: table_size_for(initial_capacity),
        }
    }

    pub fn hash_map() -> Self {
        Self::with_class(HASH_MAP, 16)
    }

    pub fn linked_hash_map() -> Self {
        Self::with_class(LINKED_HASH_MAP, 16)
    }

    pub fn hash_set() -> Self {
        Self::with_class(HASH_SET, 16)
    }

    fn is_set(&self) -> bool {
        self.class == HASH_SET
    }

    /// `put`; returns the previous value.
    pub fn put(&self, k: Value, v: Value) -> Value {
        let key = key_of(&k);
        let mut e = self.entries.borrow_mut();
        if let Some(slot) = e.get_mut(&key) {
            return std::mem::replace(&mut slot.1, v);
        }
        e.insert(key, (k, v));
        if self.capacity.get() == 0 {
            self.capacity.set(self.initial);
        }
        // `++size > threshold` doubles the table.
        while e.len() > self.capacity.get() * 3 / 4 && self.capacity.get() < 1 << 30 {
            self.capacity.set(self.capacity.get() * 2);
        }
        Value::Nil
    }

    pub fn get(&self, k: &Value) -> Value {
        self.entries
            .borrow()
            .get(&key_of(k))
            .map(|e| e.1.clone())
            .unwrap_or_default()
    }

    pub fn remove(&self, k: &Value) -> Option<Value> {
        self.entries
            .borrow_mut()
            .shift_remove(&key_of(k))
            .map(|e| e.1)
    }

    pub fn contains_key(&self, k: &Value) -> bool {
        self.entries.borrow().contains_key(&key_of(k))
    }

    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Entries in Java iteration order.
    pub fn ordered(&self) -> Vec<(Value, Value)> {
        let e = self.entries.borrow();
        let mut v: Vec<(Value, Value)> = e.values().cloned().collect();
        if !self.linked {
            let mask = self.capacity.get().max(1) - 1;
            let mut idx: Vec<(usize, usize)> = e
                .keys()
                .enumerate()
                .map(|(i, k)| {
                    let h = java_hash(k);
                    ((h ^ ((h as u32) >> 16) as i32) as u32 as usize & mask, i)
                })
                .collect();
            idx.sort_by_key(|&(bucket, i)| (bucket, i));
            v = idx.into_iter().map(|(_, i)| v[i].clone()).collect();
        }
        v
    }

    fn java_string(&self) -> String {
        let parts: Vec<String> = self
            .ordered()
            .iter()
            .map(|(k, v)| {
                if self.is_set() {
                    java_to_string(k)
                } else {
                    format!("{}={}", java_to_string(k), java_to_string(v))
                }
            })
            .collect();
        if self.is_set() {
            format!("[{}]", parts.join(", "))
        } else {
            format!("{{{}}}", parts.join(", "))
        }
    }
}

impl JavaIterator {
    fn java_string(&self) -> String {
        format!("{ITERATOR}@{:x}", self as *const Self as usize)
    }
}

impl JavaEntry {
    fn java_string(&self) -> String {
        format!(
            "{}={}",
            java_to_string(&self.key),
            java_to_string(&self.value.borrow())
        )
    }
}

/// A new Lua `ArrayList` holding `items`.
pub fn list_value(items: Vec<Value>) -> Value {
    Value::Object(Rc::new(JavaList::new(items)))
}

fn iterator_value(items: Vec<Value>) -> Value {
    Value::Object(Rc::new(JavaIterator {
        items,
        pos: Cell::new(0),
    }))
}

// ---- bindings -------------------------------------------------------------------

fn list<R>(c: &NativeCall<'_>, f: impl FnOnce(&JavaList) -> R) -> LuaResult<R> {
    let o = this::<JavaList>(c)?;
    Ok(f(o.as_any().downcast_ref::<JavaList>().unwrap()))
}

fn map<R>(c: &NativeCall<'_>, f: impl FnOnce(&JavaMap) -> R) -> LuaResult<R> {
    match c.arg(0) {
        Value::Object(o) if o.as_any().is::<JavaMap>() => {
            Ok(f(o.as_any().downcast_ref::<JavaMap>().unwrap()))
        }
        other => Err(LuaError::runtime(format!(
            "Expected a map or set but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

/// Items of any collection argument (for `addAll` and copy constructors).
fn collection_items(v: &Value) -> Option<Vec<Value>> {
    let Value::Object(o) = v else { return None };
    if let Some(l) = o.as_any().downcast_ref::<JavaList>() {
        return Some(l.items.borrow().clone());
    }
    o.as_any()
        .downcast_ref::<JavaMap>()
        .filter(|m| m.is_set())
        .map(|m| m.ordered().into_iter().map(|e| e.0).collect())
}

fn checked_index(c: &NativeCall<'_>, arg: usize, len: usize, inclusive: bool) -> LuaResult<usize> {
    let i = int_arg(c, arg);
    let max = if inclusive {
        len
    } else {
        len.saturating_sub(1)
    };
    if i < 0 || i as usize > max || (!inclusive && len == 0) {
        return Err(index_error(i, len));
    }
    Ok(i as usize)
}

pub fn register(thread: &mut Thread) {
    register_list(thread);
    register_maps(thread);
    register_set(thread);
    register_views(thread);
}

fn register_list(thread: &mut Thread) {
    let b = expose_class(thread, ARRAY_LIST, None);
    b.ctor(|c| {
        // `new ArrayList()`, `new ArrayList(int)`, `new ArrayList(Collection)`.
        let items = collection_items(&c.arg(0)).unwrap_or_default();
        Ok(c.push(list_value(items)))
    });
    b.method("size", |c| {
        let n = list(c, |l| l.items.borrow().len() as f64)?;
        Ok(c.push(n))
    });
    b.method("isEmpty", |c| {
        let n = list(c, |l| l.items.borrow().is_empty())?;
        Ok(c.push(n))
    });
    b.method("get", |c| {
        let len = list(c, |l| l.items.borrow().len())?;
        let i = checked_index(c, 1, len, false)?;
        let v = list(c, |l| l.items.borrow()[i].clone())?;
        Ok(c.push(v))
    });
    b.method("set", |c| {
        let len = list(c, |l| l.items.borrow().len())?;
        let i = checked_index(c, 1, len, false)?;
        let v = c.arg(2);
        let old = list(c, |l| std::mem::replace(&mut l.items.borrow_mut()[i], v))?;
        Ok(c.push(old))
    });
    b.method("add", |c| {
        if c.nargs() >= 3 {
            let len = list(c, |l| l.items.borrow().len())?;
            let i = checked_index(c, 1, len, true)?;
            let v = c.arg(2);
            list(c, |l| l.items.borrow_mut().insert(i, v))?;
            return Ok(0);
        }
        let v = c.arg(1);
        list(c, |l| l.items.borrow_mut().push(v))?;
        Ok(c.push(true))
    });
    b.method("addAll", |c| {
        let (at, src) = if c.nargs() >= 3 {
            (Some(int_arg(c, 1)), c.arg(2))
        } else {
            (None, c.arg(1))
        };
        let items = collection_items(&src)
            .ok_or_else(|| LuaError::runtime("addAll expects a collection"))?;
        let changed = !items.is_empty();
        let len = list(c, |l| l.items.borrow().len())?;
        if let Some(i) = at {
            if i < 0 || i as usize > len {
                return Err(index_error(i, len));
            }
            list(c, |l| {
                let mut v = l.items.borrow_mut();
                let tail = v.split_off(i as usize);
                v.extend(items);
                v.extend(tail);
            })?;
        } else {
            list(c, |l| l.items.borrow_mut().extend(items))?;
        }
        Ok(c.push(changed))
    });
    b.method("remove", |c| {
        // Kahlua passes Lua numbers to `remove(int)`.
        if let Value::Num(_) = c.arg(1) {
            let len = list(c, |l| l.items.borrow().len())?;
            let i = checked_index(c, 1, len, false)?;
            let v = list(c, |l| l.items.borrow_mut().remove(i))?;
            return Ok(c.push(v));
        }
        let target = c.arg(1);
        let removed = list(c, |l| {
            let mut v = l.items.borrow_mut();
            match v.iter().position(|x| java_equals(x, &target)) {
                Some(i) => {
                    v.remove(i);
                    true
                }
                None => false,
            }
        })?;
        Ok(c.push(removed))
    });
    b.method("contains", |c| {
        let t = c.arg(1);
        let r = list(c, |l| l.items.borrow().iter().any(|x| java_equals(x, &t)))?;
        Ok(c.push(r))
    });
    b.method("indexOf", |c| {
        let t = c.arg(1);
        let r = list(c, |l| {
            l.items
                .borrow()
                .iter()
                .position(|x| java_equals(x, &t))
                .map_or(-1.0, |i| i as f64)
        })?;
        Ok(c.push(r))
    });
    b.method("lastIndexOf", |c| {
        let t = c.arg(1);
        let r = list(c, |l| {
            l.items
                .borrow()
                .iter()
                .rposition(|x| java_equals(x, &t))
                .map_or(-1.0, |i| i as f64)
        })?;
        Ok(c.push(r))
    });
    b.method("clear", |c| {
        list(c, |l| l.items.borrow_mut().clear())?;
        Ok(0)
    });
    b.method("getFirst", |c| {
        let v = list(c, |l| l.items.borrow().first().cloned())?.ok_or_else(no_such_element)?;
        Ok(c.push(v))
    });
    b.method("getLast", |c| {
        let v = list(c, |l| l.items.borrow().last().cloned())?.ok_or_else(no_such_element)?;
        Ok(c.push(v))
    });
    b.method("removeFirst", |c| {
        let v = list(c, |l| {
            let mut v = l.items.borrow_mut();
            (!v.is_empty()).then(|| v.remove(0))
        })?
        .ok_or_else(no_such_element)?;
        Ok(c.push(v))
    });
    b.method("removeLast", |c| {
        let v = list(c, |l| l.items.borrow_mut().pop())?.ok_or_else(no_such_element)?;
        Ok(c.push(v))
    });
    b.method("iterator", |c| {
        let items = list(c, |l| l.items.borrow().clone())?;
        Ok(c.push(iterator_value(items)))
    });
    b.method("toString", |c| {
        let s = list(c, |l| l.java_string())?;
        Ok(c.push(Value::str(&s)))
    });
}

fn register_map_methods(b: &crate::expose::ClassBuilder) {
    b.method("put", |c| {
        let (k, v) = (c.arg(1), c.arg(2));
        let old = map(c, |m| m.put(k, v))?;
        Ok(c.push(old))
    });
    b.method("get", |c| {
        let k = c.arg(1);
        let v = map(c, |m| m.get(&k))?;
        Ok(c.push(v))
    });
    b.method("getOrDefault", |c| {
        let (k, d) = (c.arg(1), c.arg(2));
        let v = map(c, |m| if m.contains_key(&k) { m.get(&k) } else { d })?;
        Ok(c.push(v))
    });
    b.method("putIfAbsent", |c| {
        let (k, v) = (c.arg(1), c.arg(2));
        let old = map(c, |m| {
            let cur = m.get(&k);
            if cur.is_nil() {
                m.put(k, v);
            }
            cur
        })?;
        Ok(c.push(old))
    });
    b.method("remove", |c| {
        let k = c.arg(1);
        let v = map(c, |m| m.remove(&k).unwrap_or_default())?;
        Ok(c.push(v))
    });
    b.method("containsKey", |c| {
        let k = c.arg(1);
        let r = map(c, |m| m.contains_key(&k))?;
        Ok(c.push(r))
    });
    b.method("containsValue", |c| {
        let v = c.arg(1);
        let r = map(c, |m| {
            m.entries.borrow().values().any(|e| java_equals(&e.1, &v))
        })?;
        Ok(c.push(r))
    });
    b.method("keySet", |c| {
        let keys = map(c, |m| {
            m.ordered().into_iter().map(|e| e.0).collect::<Vec<_>>()
        })?;
        let set = JavaMap::hash_set();
        // A snapshot that keeps the map's order.
        let set = JavaMap {
            linked: true,
            ..set
        };
        for k in keys {
            set.put(k, Value::Nil);
        }
        Ok(c.push(Value::Object(Rc::new(set))))
    });
    b.method("values", |c| {
        let vals = map(c, |m| m.ordered().into_iter().map(|e| e.1).collect())?;
        Ok(c.push(list_value(vals)))
    });
    b.method("entrySet", |c| {
        let entries: Vec<Value> = map(c, |m| {
            m.ordered()
                .into_iter()
                .map(|(k, v)| {
                    Value::Object(Rc::new(JavaEntry {
                        key: k,
                        value: RefCell::new(v),
                    }))
                })
                .collect()
        })?;
        Ok(c.push(list_value(entries)))
    });
    b.method("size", |c| {
        let n = map(c, |m| m.len() as f64)?;
        Ok(c.push(n))
    });
    b.method("isEmpty", |c| {
        let n = map(c, JavaMap::is_empty)?;
        Ok(c.push(n))
    });
    b.method("clear", |c| {
        map(c, |m| m.entries.borrow_mut().clear())?;
        Ok(0)
    });
    b.method("toString", |c| {
        let s = map(c, |m| m.java_string())?;
        Ok(c.push(Value::str(&s)))
    });
}

/// `new HashMap(n)`'s capacity argument; collections are copied.
fn map_ctor(c: &mut NativeCall<'_>, class: &'static str) -> LuaResult<usize> {
    let m = match c.arg(0) {
        Value::Num(n) => JavaMap::with_class(class, n as usize),
        Value::Object(o) if o.as_any().is::<JavaMap>() => {
            let src = o.as_any().downcast_ref::<JavaMap>().unwrap();
            let m = JavaMap::with_class(class, ((src.len() as f64 / 0.75) as usize + 1).max(16));
            for (k, v) in src.ordered() {
                m.put(k, v);
            }
            m
        }
        _ => JavaMap::with_class(class, 16),
    };
    Ok(c.push(Value::Object(Rc::new(m))))
}

/// `HashMap.newHashMap(numMappings)`: capacity `ceil(n / 0.75)`.
fn sized_ctor(c: &mut NativeCall<'_>, class: &'static str) -> LuaResult<usize> {
    let n = int_arg(c, 0).max(0) as f64;
    let m = JavaMap::with_class(class, (n / 0.75).ceil() as usize);
    Ok(c.push(Value::Object(Rc::new(m))))
}

fn register_maps(thread: &mut Thread) {
    let b = expose_class(thread, HASH_MAP, None);
    b.ctor(|c| map_ctor(c, HASH_MAP));
    b.static_fn("newHashMap", |c| sized_ctor(c, HASH_MAP));
    register_map_methods(&b);
    let b = expose_class(thread, LINKED_HASH_MAP, Some(HASH_MAP));
    b.ctor(|c| map_ctor(c, LINKED_HASH_MAP));
    b.static_fn("newLinkedHashMap", |c| sized_ctor(c, LINKED_HASH_MAP));
    b.static_fn("newHashMap", |c| sized_ctor(c, HASH_MAP));
}

fn register_set(thread: &mut Thread) {
    let b = expose_class(thread, HASH_SET, None);
    b.ctor(|c| {
        let set = match c.arg(0) {
            Value::Num(n) => JavaMap::with_class(HASH_SET, n as usize),
            other => {
                let items = collection_items(&other).unwrap_or_default();
                let s = JavaMap::with_class(
                    HASH_SET,
                    ((items.len() as f64 / 0.75) as usize + 1).max(16),
                );
                for v in items {
                    s.put(v, Value::Nil);
                }
                s
            }
        };
        Ok(c.push(Value::Object(Rc::new(set))))
    });
    b.static_fn("newHashSet", |c| sized_ctor(c, HASH_SET));
    b.method("add", |c| {
        let v = c.arg(1);
        let added = map(c, |m| {
            let had = m.contains_key(&v);
            if !had {
                m.put(v, Value::Nil);
            }
            !had
        })?;
        Ok(c.push(added))
    });
    b.method("remove", |c| {
        let v = c.arg(1);
        let r = map(c, |m| m.remove(&v).is_some())?;
        Ok(c.push(r))
    });
    b.method("contains", |c| {
        let v = c.arg(1);
        let r = map(c, |m| m.contains_key(&v))?;
        Ok(c.push(r))
    });
    b.method("size", |c| {
        let n = map(c, |m| m.len() as f64)?;
        Ok(c.push(n))
    });
    b.method("isEmpty", |c| {
        let n = map(c, JavaMap::is_empty)?;
        Ok(c.push(n))
    });
    b.method("clear", |c| {
        map(c, |m| m.entries.borrow_mut().clear())?;
        Ok(0)
    });
    b.method("iterator", |c| {
        let items = map(c, |m| m.ordered().into_iter().map(|e| e.0).collect())?;
        Ok(c.push(iterator_value(items)))
    });
    b.method("toString", |c| {
        let s = map(c, |m| m.java_string())?;
        Ok(c.push(Value::str(&s)))
    });
}

fn register_views(thread: &mut Thread) {
    let b = expose_class(thread, ITERATOR, None);
    b.method("hasNext", |c| {
        let o = this::<JavaIterator>(c)?;
        let it = o.as_any().downcast_ref::<JavaIterator>().unwrap();
        Ok(c.push(it.pos.get() < it.items.len()))
    });
    b.method("next", |c| {
        let o = this::<JavaIterator>(c)?;
        let it = o.as_any().downcast_ref::<JavaIterator>().unwrap();
        let v = it
            .items
            .get(it.pos.get())
            .cloned()
            .ok_or_else(no_such_element)?;
        it.pos.set(it.pos.get() + 1);
        Ok(c.push(v))
    });
    let b = expose_class(thread, ENTRY, None);
    b.method("getKey", |c| {
        let o = this::<JavaEntry>(c)?;
        let k = o.as_any().downcast_ref::<JavaEntry>().unwrap().key.clone();
        Ok(c.push(k))
    });
    b.method("getValue", |c| {
        let o = this::<JavaEntry>(c)?;
        let v = o
            .as_any()
            .downcast_ref::<JavaEntry>()
            .unwrap()
            .value
            .borrow()
            .clone();
        Ok(c.push(v))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_map_iterates_in_java_bucket_order() {
        // Java: new HashMap<>() with "banana","apple","cherry","date" puts
        // keys in buckets 6 ("banana"), 5 ("apple"), 8 ("cherry"), 4 ("date")
        // of 16; iteration is date, apple, banana, cherry.
        let m = JavaMap::hash_map();
        for k in ["banana", "apple", "cherry", "date"] {
            m.put(Value::str(k), Value::Num(1.0));
        }
        let keys: Vec<String> = m
            .ordered()
            .iter()
            .map(|e| e.0.as_str().unwrap().to_string())
            .collect();
        let mut expect: Vec<(usize, &str)> = ["banana", "apple", "cherry", "date"]
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let h = java_hash(&Key::Str(Rc::from(*k)));
                (((h ^ ((h as u32) >> 16) as i32) & 15) as usize * 10 + i, *k)
            })
            .collect();
        expect.sort();
        assert_eq!(keys, expect.iter().map(|e| e.1).collect::<Vec<_>>());
        assert_eq!(java_hash(&Key::Str(Rc::from("apple"))), 93029210);
    }

    #[test]
    fn java_equality_rules() {
        assert!(java_equals(&Value::Num(f64::NAN), &Value::Num(f64::NAN)));
        assert!(!java_equals(&Value::Num(0.0), &Value::Num(-0.0)));
        assert!(java_equals(&Value::str("a"), &Value::str("a")));
        assert_eq!(java_hash(&Key::Num(1.0f64.to_bits())), 1072693248);
        assert_eq!(java_hash(&Key::Bool(true)), 1231);
    }

    #[test]
    fn capacity_grows_like_java() {
        let m = JavaMap::hash_map();
        for i in 0..13 {
            m.put(Value::Num(f64::from(i)), Value::Nil);
        }
        assert_eq!(m.capacity.get(), 32);
        assert_eq!(table_size_for(100), 128);
    }

    #[test]
    fn list_from_lua() {
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        register(&mut t);
        let new = t
            .globals()
            .get_str("ArrayList")
            .as_table()
            .unwrap()
            .get_str("new");
        let l = t.call(&new, &[]).unwrap().remove(0);
        let Value::Object(o) = &l else { panic!() };
        let jl = o.as_any().downcast_ref::<JavaList>().unwrap();
        jl.items
            .borrow_mut()
            .extend([Value::str("a"), Value::Num(2.0)]);
        assert_eq!(jl.java_string(), "[a, 2.0]");
    }
}
