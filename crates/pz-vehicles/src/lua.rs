//! Calling the vehicle Lua (`Vehicles.Create.*`, `Vehicles.Update.*`, ...)
//! by name the way the original does (`LuaManager.getFunctionObject` +
//! `LuaCaller.protectedCallVoid` / `protectedCallBoolean`), and the Lua
//! objects vehicles, parts and items are passed as.
//!
//! The Kahlua thread is not owned here: whoever runs vehicle code that may
//! call Lua wraps it in [`with_thread`] (the game loop with its
//! `LuaManager` thread; exposed methods with the thread of their call), the
//! way the original reads the static `LuaManager.thread`.
//!
//! The objects' methods are registered by `pz-lua-api` (`bind_vehicles`),
//! keyed by the Java class names used here.

use crate::items::ItemRef;
use crate::part::{PartRef, VehiclePart};
use crate::vehicle::BaseVehicle;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{TableRef, Thread, Value};
use pz_scripts::objects::vehicles::{LuaKey, LuaTable as ScriptTable, LuaValue};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub const BASE_VEHICLE_CLASS: &str = "zombie.vehicles.BaseVehicle";
pub const VEHICLE_PART_CLASS: &str = "zombie.vehicles.VehiclePart";
pub const VEHICLE_DOOR_CLASS: &str = "zombie.vehicles.VehicleDoor";
pub const VEHICLE_WINDOW_CLASS: &str = "zombie.vehicles.VehicleWindow";
pub const VEHICLE_LIGHT_CLASS: &str = "zombie.vehicles.VehicleLight";
pub const VEHICLE_ENGINE_CLASS: &str = "zombie.vehicles.VehicleEngine";
pub const VEHICLE_SCRIPT_CLASS: &str = "zombie.scripting.objects.VehicleScript";

thread_local! {
    static THREAD: Cell<*mut Thread> = const { Cell::new(std::ptr::null_mut()) };
    static FUNCTIONS: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
    static ERRORS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with `thread` as the thread vehicle code calls Lua on
/// (`LuaManager.thread`). Nested uses (an exposed method re-entering
/// vehicle code) install the inner thread and restore the outer one.
pub fn with_thread<R>(thread: &mut Thread, f: impl FnOnce() -> R) -> R {
    let prev = THREAD.with(|t| t.replace(thread as *mut Thread));
    struct Restore(*mut Thread);
    impl Drop for Restore {
        fn drop(&mut self) {
            THREAD.with(|t| t.set(self.0));
        }
    }
    let _restore = Restore(prev);
    f()
}

/// Runs `f` on the current thread, if any.
fn on_thread<R>(f: impl FnOnce(&mut Thread) -> R) -> Option<R> {
    let p = THREAD.with(|t| t.get());
    if p.is_null() {
        return None;
    }
    // SAFETY: the pointer was installed by `with_thread` from a live
    // `&mut Thread` whose scope encloses this call; vehicle code never
    // holds another reference to it while calling Lua (callbacks re-enter
    // through their own `NativeCall::thread`, installed by `with_thread`).
    Some(f(unsafe { &mut *p }))
}

/// A global of the current thread's environment.
pub fn global(name: &str) -> Option<Value> {
    on_thread(|t| t.globals().get_own(&Value::str(name))).filter(|v| !v.is_nil())
}

/// Lua errors raised by vehicle callbacks (the original logs them).
pub fn take_errors() -> Vec<String> {
    ERRORS.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

/// Forgets the `LuaManager.luaFunctionMap` cache (a new Lua state).
pub fn reset_function_cache() {
    FUNCTIONS.with(|f| f.borrow_mut().clear());
}

/// `LuaManager.getFunctionObject(name)`.
pub fn function_object(name: &str) -> Option<Value> {
    if name.is_empty() {
        return None;
    }
    if let Some(v) = FUNCTIONS.with(|f| f.borrow().get(name).cloned()) {
        return Some(v);
    }
    let v = on_thread(|t| {
        let mut table = t.globals().clone();
        let parts: Vec<&str> = name.split('.').collect();
        for p in &parts[..parts.len() - 1] {
            match table.get_own(&Value::str(p)) {
                Value::Table(next) => table = next,
                _ => return Value::Nil,
            }
        }
        table.get_own(&Value::str(parts[parts.len() - 1]))
    })?;
    if v.is_function() {
        FUNCTIONS.with(|f| f.borrow_mut().insert(name.to_string(), v.clone()));
        Some(v)
    } else {
        ERRORS.with(|e| e.borrow_mut().push(format!("no such function \"{name}\"")));
        None
    }
}

/// An argument of a vehicle callback.
#[derive(Clone)]
pub enum LuaArg {
    Vehicle(Rc<BaseVehicle>),
    Part(PartRef),
    /// A Java `Float`/`Double` (passed as a Lua number).
    Num(f64),
    Value(Value),
}

impl LuaArg {
    pub fn to_value(&self) -> Value {
        match self {
            LuaArg::Vehicle(v) => vehicle_value(v),
            LuaArg::Part(p) => part_value(p),
            LuaArg::Num(n) => Value::Num(*n),
            LuaArg::Value(v) => v.clone(),
        }
    }
}

fn pcall(name: &str, args: &[LuaArg]) -> Option<Result<Vec<Value>, String>> {
    let f = function_object(name)?;
    let args: Vec<Value> = args.iter().map(LuaArg::to_value).collect();
    on_thread(|t| {
        t.pcall(&f, &args).map_err(|(m, trace)| {
            format!(
                "{}: {}\n{}",
                name,
                pz_kahlua::exec::java_value_of(&m),
                trace
            )
        })
    })
}

/// `LuaCaller.protectedCallVoid(thread, getFunctionObject(name), args)`.
pub fn call_void(name: &str, args: &[LuaArg]) {
    if let Some(Err(e)) = pcall(name, args) {
        ERRORS.with(|x| x.borrow_mut().push(e));
    }
}

/// `LuaCaller.protectedCallBoolean(...)`: the first result when it is a
/// boolean, else `None` (also when the function is missing or fails).
pub fn call_bool(name: &str, args: &[LuaArg]) -> Option<bool> {
    match pcall(name, args)? {
        Ok(r) => match r.first() {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        },
        Err(e) => {
            ERRORS.with(|x| x.borrow_mut().push(e));
            None
        }
    }
}

/// `LuaManager.copyTable` of a script table (`VehicleScript.LoadTable`
/// stores numbers as `Double`, strings as `String`, booleans? no: plain
/// strings and nested tables).
pub fn script_table_to_lua(t: &ScriptTable) -> TableRef {
    let out = TableRef::new();
    for (k, v) in &t.entries {
        let key = match k {
            LuaKey::Str(s) => Value::str(s),
            LuaKey::Num(n) => Value::Num(*n),
        };
        let val = match v {
            LuaValue::Str(s) => Value::str(s),
            LuaValue::Table(t) => Value::Table(script_table_to_lua(t)),
        };
        out.rawset(key, val);
    }
    out
}

macro_rules! lua_object {
    ($name:ident, $inner:ty, $class:expr, [$($sup:expr),*]) => {
        /// A Lua handle (one per object while it lives, so Lua `==` is
        /// Java identity).
        pub struct $name(pub $inner);
        impl LuaObject for $name {
            fn class_name(&self) -> &str {
                $class
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn super_classes(&self) -> &[&'static str] {
                &[$($sup),*]
            }
        }
    };
}

lua_object!(
    LuaVehicle,
    Rc<BaseVehicle>,
    BASE_VEHICLE_CLASS,
    ["zombie.iso.IsoMovingObject", "zombie.iso.IsoObject"]
);
lua_object!(
    LuaPart,
    PartRef,
    VEHICLE_PART_CLASS,
    ["zombie.entity.GameEntity"]
);
lua_object!(LuaDoor, PartRef, VEHICLE_DOOR_CLASS, []);
lua_object!(LuaWindow, PartRef, VEHICLE_WINDOW_CLASS, []);
lua_object!(LuaLight, PartRef, VEHICLE_LIGHT_CLASS, []);
lua_object!(LuaEngine, PartRef, VEHICLE_ENGINE_CLASS, []);
lua_object!(
    LuaVehicleScript,
    Rc<RefCell<pz_scripts::objects::vehicles::VehicleScript>>,
    VEHICLE_SCRIPT_CLASS,
    ["zombie.scripting.objects.BaseScriptObject"]
);

/// The Lua object of a vehicle.
pub fn vehicle_value(v: &Rc<BaseVehicle>) -> Value {
    let cached = v.lua.borrow().as_ref().and_then(Weak::upgrade);
    if let Some(o) = cached {
        return Value::Object(o);
    }
    let o: Rc<dyn LuaObject> = Rc::new(LuaVehicle(v.clone()));
    *v.lua.borrow_mut() = Some(Rc::downgrade(&o));
    Value::Object(o)
}

fn cached_part_object(p: &PartRef, slot: usize, make: impl FnOnce() -> Rc<dyn LuaObject>) -> Value {
    thread_local! {
        static SUB: RefCell<HashMap<(usize, usize), Weak<dyn LuaObject>>> = RefCell::new(HashMap::new());
    }
    if slot == 0 {
        let cached = p.lua.borrow().as_ref().and_then(Weak::upgrade);
        if let Some(o) = cached {
            return Value::Object(o);
        }
        let o = make();
        *p.lua.borrow_mut() = Some(Rc::downgrade(&o));
        return Value::Object(o);
    }
    let key = (Rc::as_ptr(p) as usize, slot);
    if let Some(o) = SUB.with(|s| s.borrow().get(&key).and_then(Weak::upgrade)) {
        return Value::Object(o);
    }
    let o = make();
    SUB.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|_, w| w.strong_count() > 0);
        s.insert(key, Rc::downgrade(&o));
    });
    Value::Object(o)
}

/// The Lua object of a part.
pub fn part_value(p: &PartRef) -> Value {
    cached_part_object(p, 0, || Rc::new(LuaPart(p.clone())))
}

/// `part:getDoor()` (nil without a door).
pub fn door_value(p: &PartRef) -> Value {
    if p.st().door.is_none() {
        return Value::Nil;
    }
    cached_part_object(p, 1, || Rc::new(LuaDoor(p.clone())))
}

/// `part:getWindow()`.
pub fn window_value(p: &PartRef) -> Value {
    if p.st().window.is_none() {
        return Value::Nil;
    }
    cached_part_object(p, 2, || Rc::new(LuaWindow(p.clone())))
}

/// `part:getLight()`.
pub fn light_value(p: &PartRef) -> Value {
    if p.st().light.is_none() {
        return Value::Nil;
    }
    cached_part_object(p, 3, || Rc::new(LuaLight(p.clone())))
}

/// `part:getVehicleEngine()`.
pub fn engine_value(p: &PartRef) -> Value {
    if p.st().engine.is_none() {
        return Value::Nil;
    }
    cached_part_object(p, 4, || Rc::new(LuaEngine(p.clone())))
}

/// Item objects: installed by the item binding (pz-lua-api); without one
/// items are opaque handles of class `zombie.inventory.InventoryItem`.
pub type ItemToLua = dyn Fn(&ItemRef) -> Value;

thread_local! {
    static ITEM_TO_LUA: RefCell<Option<Rc<ItemToLua>>> = const { RefCell::new(None) };
    static ITEM_FROM_LUA: RefCell<Option<Rc<dyn Fn(&Value) -> Option<ItemRef>>>> = const { RefCell::new(None) };
}

/// Installs the item <-> Lua conversions of the item binding.
pub fn install_item_conversions(to: Rc<ItemToLua>, from: Rc<dyn Fn(&Value) -> Option<ItemRef>>) {
    ITEM_TO_LUA.with(|t| *t.borrow_mut() = Some(to));
    ITEM_FROM_LUA.with(|t| *t.borrow_mut() = Some(from));
}

/// The Lua object of an item.
pub fn item_value(i: &ItemRef) -> Value {
    match ITEM_TO_LUA.with(|t| t.borrow().clone()) {
        Some(f) => f(i),
        None => Value::Nil,
    }
}

/// The item of a Lua object.
pub fn item_from_value(v: &Value) -> Option<ItemRef> {
    ITEM_FROM_LUA
        .with(|t| t.borrow().clone())
        .and_then(|f| f(v))
}

/// The vehicle of a Lua object.
pub fn vehicle_from_value(v: &Value) -> Option<Rc<BaseVehicle>> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<LuaVehicle>().map(|l| l.0.clone()),
        _ => None,
    }
}

/// The part of a Lua object.
pub fn part_from_value(v: &Value) -> Option<PartRef> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<LuaPart>().map(|l| l.0.clone()),
        _ => None,
    }
}

impl std::fmt::Debug for LuaArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LuaArg::Vehicle(v) => write!(f, "Vehicle({})", v.id()),
            LuaArg::Part(p) => write!(f, "Part({:?})", VehiclePart::id(p)),
            LuaArg::Num(n) => write!(f, "{n}"),
            LuaArg::Value(v) => write!(f, "{v:?}"),
        }
    }
}
