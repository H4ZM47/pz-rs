//! Exposing engine types to Lua the way Kahlua's `LuaJavaClassExposer`
//! exposes Java classes.
//!
//! For a class `a.b.C` the exposer creates nested package tables
//! `a.b.C` under the globals, aliases the leaf as global `C` (unless taken),
//! stores static methods, static fields and `new` constructors in it, and
//! registers an instance metatable `{__index = methods}` whose method table
//! inherits from the superclass's through its own metatable.

use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, NativeFunction, TableRef, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub type Method = dyn Fn(&mut NativeCall<'_>) -> LuaResult<usize>;

/// Builder returned by [`expose_class`].
pub struct ClassBuilder {
    pub fqn: &'static str,
    pub statics: TableRef,
    pub methods: TableRef,
}

/// Simple name as Kahlua derives it: after the last '.', with '$' kept for
/// nested classes in the global alias (`clazz.getSimpleName()`).
pub fn simple_name(fqn: &str) -> &str {
    let leaf = fqn.rsplit('.').next().unwrap_or(fqn);
    leaf.rsplit('$').next().unwrap_or(leaf)
}

/// Create (or fetch) the exposure tables for class `fqn` with superclass
/// `superclass` (whose exposure must already exist if given).
pub fn expose_class(thread: &mut Thread, fqn: &'static str, superclass: Option<&str>) -> ClassBuilder {
    let env = thread.globals().clone();
    // Metatable with __index method table.
    let meta = match thread.class_metatables.get_str(fqn) {
        Value::Table(t) => t,
        _ => {
            let meta = TableRef::new();
            let methods = TableRef::new();
            meta.set_str("__index", methods.clone());
            if let Some(sup) = superclass {
                if let Value::Table(sm) = thread.class_metatables.get_str(sup) {
                    let nw = sm.get_str("__newindex");
                    if !nw.is_nil() {
                        meta.set_str("__newindex", nw);
                    }
                    methods.set_metatable(Some(sm));
                }
            }
            thread.class_metatables.set_str(fqn, meta.clone());
            meta
        }
    };
    let methods = meta.get_str("__index").as_table().cloned().unwrap();
    // Package tables: a.b.C (with '$' nesting split into tables too).
    let parts: Vec<String> = fqn.replace('$', ".").split('.').map(str::to_string).collect();
    let mut container = env.clone();
    for p in &parts {
        let next = match container.get_own(&Value::str(p)) {
            Value::Table(t) => t,
            _ => {
                let t = TableRef::new();
                container.set_str(p, t.clone());
                t
            }
        };
        container = next;
    }
    container.set_str("class", Value::str(fqn));
    let simple = simple_name(fqn);
    if env.get_own(&Value::str(simple)).is_nil() {
        env.set_str(simple, container.clone());
    }
    ClassBuilder { fqn, statics: container, methods }
}

impl ClassBuilder {
    /// Instance method, called as `obj:name(...)`; argument 0 is `self`.
    pub fn method(&self, name: &str, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<usize> + 'static) -> &Self {
        self.methods.set_str(name, Value::Native(NativeFunction::new(name, f)));
        self
    }
    /// Static method, called as `Class.name(...)`.
    pub fn static_fn(&self, name: &str, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<usize> + 'static) -> &Self {
        self.statics.set_str(name, Value::Native(NativeFunction::new(name, f)));
        self
    }
    /// Public static field value (copied at exposure time, like Kahlua).
    pub fn field(&self, name: &str, v: impl Into<Value>) -> &Self {
        if self.statics.get_own(&Value::str(name)).is_nil() {
            self.statics.set_str(name, v.into());
        }
        self
    }
    /// Public constructor(s), exposed as `Class.new(...)`.
    pub fn ctor(&self, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<usize> + 'static) -> &Self {
        self.statics.set_str("new", Value::Native(NativeFunction::new("new", f)));
        self
    }
}

/// Register a global function (`@LuaMethod(global = true)`).
pub fn global(thread: &mut Thread, name: &str, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<usize> + 'static) {
    thread.globals().set_str(name, Value::Native(NativeFunction::new(name, f)));
}

/// Downcast `self` (argument 0) of a method call.
pub fn this<T: 'static>(c: &NativeCall<'_>) -> LuaResult<Rc<dyn LuaObject>> {
    match c.arg(0) {
        Value::Object(o) if o.as_any().is::<T>() => Ok(o),
        other => Err(LuaError::runtime(format!(
            "Expected object of type {} but got {}",
            std::any::type_name::<T>(),
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

/// Run `f` on the downcast `self`.
pub fn with_this<T: 'static, R>(c: &NativeCall<'_>, f: impl FnOnce(&T) -> R) -> LuaResult<R> {
    let o = this::<T>(c)?;
    Ok(f(o.as_any().downcast_ref::<T>().unwrap()))
}

/// Kahlua's Lua->Java numeric conversion for `int` parameters
/// (`Double.intValue()`); missing or non-numeric arguments become 0.
pub fn int_arg(c: &NativeCall<'_>, i: usize) -> i32 {
    match pz_kahlua::exec::raw_tonumber(&c.arg(i)) {
        Some(d) => d as i32,
        None => 0,
    }
}

pub fn num_arg(c: &NativeCall<'_>, i: usize) -> f64 {
    pz_kahlua::exec::raw_tonumber(&c.arg(i)).unwrap_or(0.0)
}

pub fn bool_arg(c: &NativeCall<'_>, i: usize) -> bool {
    matches!(c.arg(i), Value::Bool(true))
}

pub fn str_arg(c: &NativeCall<'_>, i: usize) -> Option<Rc<str>> {
    match c.arg(i) {
        Value::Str(s) => Some(s),
        Value::Num(n) => Some(Rc::from(pz_kahlua::number::number_to_string(n))),
        _ => None,
    }
}

/// A live `java.util.ArrayList` held by an engine object: reads (and
/// `clear`, when the owner allows it) go to the owner's current list.
pub struct ListView {
    pub items: Box<dyn Fn() -> Vec<Value>>,
    pub clear: Option<Box<dyn Fn()>>,
}

impl ListView {
    pub fn new(items: impl Fn() -> Vec<Value> + 'static) -> Self {
        ListView { items: Box::new(items), clear: None }
    }

    pub fn with_clear(mut self, clear: impl Fn() + 'static) -> Self {
        self.clear = Some(Box::new(clear));
        self
    }
}

impl LuaObject for ListView {
    fn class_name(&self) -> &str {
        "java.util.ArrayList"
    }
    fn to_lua_string(&self) -> String {
        let parts: Vec<String> = (self.items)().iter().map(pz_kahlua::exec::java_value_of).collect();
        format!("[{}]", parts.join(", "))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `java.lang.Integer`: a boxed int that reaches Lua without Kahlua's
/// number conversion, e.g. a `HashMap<?, Integer>` value copied raw by
/// `transformIntoKahluaTable`. Lua reads it with `intValue()`.
pub struct JavaInteger(pub i32);

pub const INTEGER_CLASS: &str = "java.lang.Integer";

impl LuaObject for JavaInteger {
    fn class_name(&self) -> &str {
        INTEGER_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.0.to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.lang.Number"]
    }
}

/// Expose `java.lang.Integer`'s value methods (once per thread).
pub fn expose_integer(thread: &mut Thread) {
    if !thread.class_metatables.get_str(INTEGER_CLASS).is_nil() {
        return;
    }
    let b = expose_class(thread, INTEGER_CLASS, None);
    for name in ["intValue", "longValue", "floatValue", "doubleValue", "shortValue", "byteValue"] {
        b.method(name, move |c| {
            let v = with_this::<JavaInteger, _>(c, |i| i.0)?;
            let v = match name {
                "shortValue" => v as i16 as f64,
                "byteValue" => v as i8 as f64,
                _ => v as f64,
            };
            Ok(c.push(v))
        });
    }
    b.method("toString", |c| {
        let v = with_this::<JavaInteger, _>(c, |i| i.0.to_string())?;
        Ok(c.push(v))
    });
    b.method("hashCode", |c| {
        let v = with_this::<JavaInteger, _>(c, |i| i.0)?;
        Ok(c.push(v as f64))
    });
    // `Integer.equals`: another Integer with the same value.
    b.method("equals", |c| {
        let v = with_this::<JavaInteger, _>(c, |i| i.0)?;
        let eq = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<JavaInteger>().is_some_and(|o| o.0 == v),
            _ => false,
        };
        Ok(c.push(eq))
    });
    b.method("compareTo", |c| {
        let v = with_this::<JavaInteger, _>(c, |i| i.0)?;
        let other = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<JavaInteger>().map(|o| o.0),
            _ => None,
        };
        let Some(other) = other else {
            return Err(LuaError::Runtime { message: String::new(), class: "java.lang.ClassCastException" });
        };
        Ok(c.push(v.cmp(&other) as i32 as f64))
    });
}

/// A live `java.util.HashMap` held by an engine object, like [`ListView`]:
/// reads go to the owner's current map. [`MapView::entries`] is the
/// iteration order (`transformIntoKahluaTable` copies it raw); each owner
/// documents its order.
pub struct MapView {
    pub entries: Box<dyn Fn() -> Vec<(Value, Value)>>,
}

pub const HASH_MAP_CLASS: &str = "java.util.HashMap";

impl MapView {
    pub fn new(entries: impl Fn() -> Vec<(Value, Value)> + 'static) -> Self {
        MapView { entries: Box::new(entries) }
    }

    /// `get(key)` with the raw (unconverted) value.
    pub fn get_raw(&self, key: &Value) -> Value {
        (self.entries)().into_iter().find(|(k, _)| k.raw_equals(key)).map(|(_, v)| v).unwrap_or_default()
    }
}

impl LuaObject for MapView {
    fn class_name(&self) -> &str {
        HASH_MAP_CLASS
    }
    /// `AbstractMap.toString`.
    fn to_lua_string(&self) -> String {
        let parts: Vec<String> = (self.entries)()
            .iter()
            .map(|(k, v)| format!("{}={}", pz_kahlua::exec::java_value_of(k), pz_kahlua::exec::java_value_of(v)))
            .collect();
        format!("{{{}}}", parts.join(", "))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.util.AbstractMap", "java.util.Map"]
    }
}

/// Kahlua's return conversion of a boxed number (`Integer` -> `Double`).
pub fn to_lua_number(v: Value) -> Value {
    match &v {
        Value::Object(o) => match o.as_any().downcast_ref::<JavaInteger>() {
            Some(i) => Value::Num(i.0 as f64),
            None => v,
        },
        _ => v,
    }
}

/// Expose the read side of `java.util.HashMap` for [`MapView`]s (once per
/// thread; a fuller `HashMap` binding registered first takes precedence).
pub fn expose_map_view(thread: &mut Thread) {
    if !thread.class_metatables.get_str(HASH_MAP_CLASS).is_nil() {
        return;
    }
    let b = expose_class(thread, HASH_MAP_CLASS, None);
    fn view<R>(c: &NativeCall<'_>, f: impl FnOnce(&MapView) -> R) -> LuaResult<R> {
        with_this::<MapView, _>(c, f)
    }
    b.method("size", |c| {
        let n = view(c, |m| (m.entries)().len())?;
        Ok(c.push(n as f64))
    });
    b.method("isEmpty", |c| {
        let e = view(c, |m| (m.entries)().is_empty())?;
        Ok(c.push(e))
    });
    b.method("get", |c| {
        let k = c.arg(1);
        let v = view(c, |m| m.get_raw(&k))?;
        Ok(c.push(to_lua_number(v)))
    });
    b.method("containsKey", |c| {
        let k = c.arg(1);
        let v = view(c, |m| (m.entries)().iter().any(|(e, _)| e.raw_equals(&k)))?;
        Ok(c.push(v))
    });
    b.method("toString", |c| {
        let s = view(c, |m| m.to_lua_string())?;
        Ok(c.push(s))
    });
}

/// A Java enum constant. Constants are singletons so Lua `==` (identity)
/// behaves like Java.
pub struct JavaEnum {
    pub class: &'static str,
    pub name: &'static str,
    pub ordinal: i32,
}

impl LuaObject for JavaEnum {
    fn class_name(&self) -> &str {
        self.class
    }
    fn to_lua_string(&self) -> String {
        self.name.to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.lang.Enum"]
    }
}

thread_local! {
    static ENUMS: RefCell<HashMap<&'static str, Vec<Value>>> = RefCell::new(HashMap::new());
}

/// Expose an enum: constants become static fields; instances get
/// `name()`, `toString()`, `ordinal()` (from `java.lang.Enum`).
pub fn expose_enum(thread: &mut Thread, fqn: &'static str, constants: &[&'static str]) -> ClassBuilder {
    ensure_enum_base(thread);
    let b = expose_class(thread, fqn, Some("java.lang.Enum"));
    let vals: Vec<Value> = constants
        .iter()
        .enumerate()
        .map(|(i, n)| Value::Object(Rc::new(JavaEnum { class: fqn, name: n, ordinal: i as i32 })))
        .collect();
    for (n, v) in constants.iter().zip(&vals) {
        b.field(n, v.clone());
    }
    ENUMS.with(|e| e.borrow_mut().insert(fqn, vals.clone()));
    let values = vals.clone();
    b.static_fn("values", move |c| {
        let t = TableRef::new();
        for (i, v) in values.iter().enumerate() {
            t.set_int(i as i64 + 1, v.clone());
        }
        Ok(c.push(t))
    });
    let values = vals;
    b.static_fn("valueOf", move |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        for v in &values {
            if let Value::Object(o) = v {
                if o.as_any().downcast_ref::<JavaEnum>().is_some_and(|e| *e.name == *name) {
                    return Ok(c.push(v.clone()));
                }
            }
        }
        Err(LuaError::Runtime { message: format!("No enum constant {fqn}.{name}"), class: "java.lang.IllegalArgumentException" })
    });
    b
}

/// Fetch an enum constant by name (for Rust code returning enums to Lua).
pub fn enum_value(fqn: &str, name: &str) -> Value {
    ENUMS.with(|e| {
        e.borrow()
            .get(fqn)
            .and_then(|vals| {
                vals.iter().find(|v| match v {
                    Value::Object(o) => o.as_any().downcast_ref::<JavaEnum>().is_some_and(|e| e.name == name),
                    _ => false,
                })
            })
            .cloned()
            .unwrap_or_default()
    })
}

fn ensure_enum_base(thread: &mut Thread) {
    if !thread.class_metatables.get_str("java.lang.Enum").is_nil() {
        return;
    }
    let b = expose_class(thread, "java.lang.Enum", None);
    b.method("name", |c| {
        let n = with_this::<JavaEnum, _>(c, |e| e.name)?;
        Ok(c.push(n))
    });
    b.method("toString", |c| {
        let n = with_this::<JavaEnum, _>(c, |e| e.name)?;
        Ok(c.push(n))
    });
    b.method("ordinal", |c| {
        let n = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push(n))
    });
    b.method("equals", |c| {
        let r = c.arg(0).raw_equals(&c.arg(1));
        Ok(c.push(r))
    });
}
