//! `zombie.iso.ISWorldObjectContextMenuLogic` (42.21): the Java half of the
//! world context menu. `ISWorldObjectContextMenu.createMenu` calls its
//! `fetch` for every clicked object and then `createMenuEntries`, and the
//! inventory and world menus call `callCustomFunction`.
//!
//! The original works on the Java objects directly. This port calls the
//! same methods through the objects' Lua bindings, so a method that the
//! port does not bind yet fails with its name instead of a silent default.
//! The helpers for objects that the port cannot build yet stop with
//! `unsupported` on the path where such an object is really present.

mod entries;
mod fetch;
mod fuel;
mod water;
mod helpers;
mod object_power;
mod reads;
mod sprite_grid;

use pz_kahlua::{LuaError, LuaResult, NativeCall, NativeFunction, TableRef, Thread, Value};

use crate::bind_world_object::unsupported;
use crate::expose::expose_class;

const CLASS: &str = "zombie.iso.ISWorldObjectContextMenuLogic";

pub(crate) fn register(thread: &mut Thread) {
    reads::register(thread);
    sprite_grid::register(thread);
    object_power::register(thread);
    let class = expose_class(thread, CLASS, None);
    // fetch(fetch, object, player, doSquare)
    class.static_fn("fetch", |c| {
        let fetch = table_arg(c, 0, "fetch")?;
        let object = c.arg(1);
        let player = c.arg(2).as_num().unwrap_or(0.0);
        let do_square = c.arg(3).truthy();
        fetch::fetch(&mut Cx { t: c.thread }, &fetch, &object, player, do_square)?;
        Ok(0)
    });
    // createMenuEntries(fetch, context, player, worldobjects, x, y, test)
    class.static_fn("createMenuEntries", |c| {
        let fetch = table_arg(c, 0, "fetch")?;
        let context = table_arg(c, 1, "context")?;
        let player = c.arg(2).as_num().unwrap_or(0.0);
        let worldobjects = table_arg(c, 3, "worldobjects")?;
        let x = crate::expose::int_arg(c, 4);
        let y = crate::expose::int_arg(c, 5);
        let test = c.arg(6).truthy();
        let args = entries::Args { fetch, context, player, worldobjects, x, y, test };
        let r = entries::create_menu_entries(&mut Cx { t: c.thread }, &args)?;
        Ok(c.push(r))
    });
    class.static_fn("callCustomFunction", |_c| {
        Err(unsupported("ISWorldObjectContextMenuLogic.callCustomFunction"))
    });
}

fn table_arg(c: &NativeCall<'_>, i: usize, what: &str) -> LuaResult<TableRef> {
    match c.arg(i) {
        Value::Table(t) => Ok(t),
        Value::Nil => Err(null_pointer()),
        other => Err(crate::bind_java_util::class_cast(&other, &format!("se.krka.kahlua.vm.KahluaTable ({what})"))),
    }
}

pub(super) fn null_pointer() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

/// The calls the original makes on Java objects, made through Lua.
pub(super) struct Cx<'a> {
    pub t: &'a mut Thread,
}

impl Cx<'_> {
    /// `obj.name(args)` through the object's Lua binding; a null receiver
    /// throws like Java, an unbound method names itself.
    pub fn call(&mut self, obj: &Value, name: &str, args: &[Value]) -> LuaResult<Value> {
        if obj.is_nil() {
            return Err(null_pointer());
        }
        let f = self.t.table_get(obj, &Value::str(name))?;
        if f.is_nil() {
            return Err(LuaError::runtime(format!(
                "ISWorldObjectContextMenuLogic: {}.{name} is not bound",
                pz_kahlua::stdlib::java_class_name(obj)
            )));
        }
        let mut all = Vec::with_capacity(args.len() + 1);
        all.push(obj.clone());
        all.extend_from_slice(args);
        self.t.call1(&f, &all)
    }

    /// A boolean method.
    pub fn is(&mut self, obj: &Value, name: &str, args: &[Value]) -> LuaResult<bool> {
        Ok(self.call(obj, name, args)?.truthy())
    }

    /// A number method.
    pub fn num(&mut self, obj: &Value, name: &str, args: &[Value]) -> LuaResult<f64> {
        Ok(self.call(obj, name, args)?.as_num().unwrap_or(0.0))
    }

    /// A global Lua function (`getCore`, `getSpecificPlayer`, ...).
    pub fn global(&mut self, name: &str, args: &[Value]) -> LuaResult<Value> {
        let f = self.t.globals().rawget(&Value::str(name));
        if f.is_nil() {
            return Err(LuaError::runtime(format!("ISWorldObjectContextMenuLogic: global {name} is not bound")));
        }
        self.t.call1(&f, args)
    }

    /// `Translator.getText(key)`.
    pub fn text(&mut self, key: &str) -> LuaResult<Value> {
        self.global("getText", &[Value::str(key)])
    }

    /// `Translator.getText(key, args...)`.
    pub fn text_args(&mut self, key: &str, args: &[Value]) -> LuaResult<Value> {
        let mut all = vec![Value::str(key)];
        all.extend_from_slice(args);
        self.global("getText", &all)
    }

    /// `LuaManager.getFunctionObject(path)`.
    pub fn func(&self, path: &str) -> Value {
        crate::bind_item_container::function_object(self.t, path)
    }

    /// A static field of an exposed class or enum, e.g. `IsoFlagType.bed`.
    pub fn static_field(&self, class: &str, field: &str) -> Value {
        match self.t.globals().rawget(&Value::str(class)) {
            Value::Table(t) => t.rawget(&Value::str(field)),
            _ => Value::Nil,
        }
    }

    /// A static method of an exposed class, e.g. `SafeHouse.getSafeHouse`.
    pub fn static_call(&mut self, class: &str, method: &str, args: &[Value]) -> LuaResult<Value> {
        let f = self.static_field(class, method);
        if f.is_nil() {
            return Err(LuaError::runtime(format!("ISWorldObjectContextMenuLogic: {class}.{method} is not bound")));
        }
        self.t.call1(&f, args)
    }

    /// `props.has(IsoFlagType.<flag>)`.
    pub fn has_flag(&mut self, props: &Value, flag: &str) -> LuaResult<bool> {
        let f = self.static_field("IsoFlagType", flag);
        if f.is_nil() {
            return Err(LuaError::runtime(format!("ISWorldObjectContextMenuLogic: IsoFlagType.{flag} is not bound")));
        }
        self.is(props, "has", &[f])
    }

    /// `object.getSprite().getProperties().has(flag)` (a null sprite fails
    /// in the original too, so callers check the sprite first).
    pub fn sprite_has(&mut self, object: &Value, flag: &str) -> LuaResult<bool> {
        let sprite = self.call(object, "getSprite", &[])?;
        let props = self.call(&sprite, "getProperties", &[])?;
        self.has_flag(&props, flag)
    }

    /// `ItemTag.<name>`.
    pub fn item_tag(&self, name: &str) -> Value {
        self.static_field("ItemTag", name)
    }

    /// `LuaHelpers.callLuaClass(class, method, self, args)`: a protected
    /// call; a failure is logged and gives null.
    pub fn lua_class(&mut self, class: &str, method: &str, this: Value, args: &[Value]) -> Value {
        let f = self.func(&format!("{class}.{method}"));
        let mut all = Vec::with_capacity(args.len() + 1);
        if !this.is_nil() {
            all.push(this);
        }
        all.extend_from_slice(args);
        match self.t.pcall(&f, &all) {
            Ok(r) => r.into_iter().next().unwrap_or_default(),
            Err((msg, trace)) => {
                (self.t.log)(&format!("{}\n{}", pz_kahlua::exec::java_value_of(&msg), trace));
                Value::Nil
            }
        }
    }

    /// `ItemContainer.getFirstRecurse(predicate)`, with the predicate as a
    /// native Lua function for `getFirstEvalRecurse`.
    pub fn first_recurse(
        &mut self,
        container: &Value,
        pred: impl Fn(&mut Cx<'_>, &Value) -> LuaResult<bool> + 'static,
    ) -> LuaResult<Value> {
        let f = NativeFunction::new("predicate", move |c| {
            let item = c.arg(0);
            let r = pred(&mut Cx { t: c.thread }, &item)?;
            Ok(c.push(r))
        });
        self.call(container, "getFirstEvalRecurse", &[Value::Native(f)])
    }

    /// `!item.isBroken() && item.hasTag(ItemTag.<tag>)`.
    pub fn not_broken_with_tag(&mut self, item: &Value, tag: &str) -> LuaResult<bool> {
        if self.is(item, "isBroken", &[])? {
            return Ok(false);
        }
        let tag = self.item_tag(tag);
        self.is(item, "hasTag", &[tag])
    }

    /// `ISContextMenuWrapper.<method>(name, target, onSelect, args...)`:
    /// the wrapper's option methods are ports of the Lua `ISContextMenu`
    /// methods of the same names, so this calls those.
    pub fn option(
        &mut self,
        menu: &Value,
        method: &str,
        name: Value,
        target: Value,
        on_select: Value,
        args: &[Value],
    ) -> LuaResult<Value> {
        let mut all = vec![name, target, on_select];
        all.extend_from_slice(args);
        self.call(menu, method, &all)
    }

    /// `Core.debug`.
    pub fn core_debug(&mut self) -> LuaResult<bool> {
        let f = self.t.globals().rawget(&Value::str("isDebugEnabled"));
        Ok(!f.is_nil() && self.t.call1(&f, &[])?.truthy())
    }

    /// `IsoPlayer.players[player]`.
    pub fn player(&mut self, player: f64) -> LuaResult<Value> {
        self.global("getSpecificPlayer", &[Value::Num(player)])
    }
}

/// `t[t.size() + 1] = v`.
pub(super) fn append(t: &TableRef, v: Value) {
    t.rawset(Value::Num((t.len() + 1) as f64), v);
}

/// `LuaHelpers.tableContainsValue(t, v)`: identity of the values.
pub(super) fn contains_value(t: Option<&TableRef>, v: &Value) -> bool {
    t.is_some_and(|t| t.entries().iter().any(|(_, x)| x.raw_equals(v)))
}

/// `LuaHelpers.tableContainsKey(t, k)`.
pub(super) fn contains_key(t: &TableRef, k: &Value) -> bool {
    t.entries().iter().any(|(x, _)| x.raw_equals(k))
}

/// The table under `key`, or `None` for nil.
pub(super) fn table_at(t: &TableRef, key: &str) -> Option<TableRef> {
    match t.get_str(key) {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
