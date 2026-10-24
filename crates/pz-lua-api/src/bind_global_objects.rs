//! Lua exposure of `zombie.globalObjects`: the `CGlobalObjects` and
//! `SGlobalObjects` statics, the `CGlobalObjectSystem` /
//! `SGlobalObjectSystem` and `CGlobalObject` / `SGlobalObject` instances.
//! The behaviour lives in [`crate::global_objects`].

use crate::bind_java_util::{java_equals, list_value, JavaList};
use crate::events::EventManager;
use crate::expose::{expose_class, int_arg, str_arg};
use crate::global_objects::{self as go, GlobalObject, GlobalObjectSystem, ObjectSync, Side};
use crate::LuaManager;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

const GLOBAL_OBJECT_SYSTEM: &str = "zombie.globalObjects.GlobalObjectSystem";
const GLOBAL_OBJECT: &str = "zombie.globalObjects.GlobalObject";
const C_SYSTEM: &str = "zombie.globalObjects.CGlobalObjectSystem";
const S_SYSTEM: &str = "zombie.globalObjects.SGlobalObjectSystem";
const C_OBJECT: &str = "zombie.globalObjects.CGlobalObject";
const S_OBJECT: &str = "zombie.globalObjects.SGlobalObject";

impl LuaObject for GlobalObjectSystem {
    fn class_name(&self) -> &str {
        match self.side {
            Side::Client => C_SYSTEM,
            Side::Server => S_SYSTEM,
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[GLOBAL_OBJECT_SYSTEM]
    }
}

impl LuaObject for GlobalObject {
    fn class_name(&self) -> &str {
        match self.side {
            Side::Client => C_OBJECT,
            Side::Server => S_OBJECT,
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[GLOBAL_OBJECT]
    }
}

fn system_value(s: Option<Rc<GlobalObjectSystem>>) -> Value {
    s.map_or(Value::Nil, |s| Value::Object(s))
}

fn object_value(o: Option<Rc<GlobalObject>>) -> Value {
    o.map_or(Value::Nil, |o| Value::Object(o))
}

fn this_system(c: &NativeCall<'_>) -> LuaResult<Rc<GlobalObjectSystem>> {
    match c.arg(0) {
        Value::Object(o) if o.as_any().is::<GlobalObjectSystem>() => Ok(o
            .as_any()
            .downcast_ref::<GlobalObjectSystem>()
            .unwrap()
            .rc()),
        other => Err(LuaError::runtime(format!(
            "Expected a GlobalObjectSystem but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

fn object_arg(c: &NativeCall<'_>, i: usize) -> Option<Rc<GlobalObject>> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<GlobalObject>()
            .map(GlobalObject::rc),
        _ => None,
    }
}

fn string_arg(c: &NativeCall<'_>, i: usize) -> String {
    str_arg(c, i).map(|s| s.to_string()).unwrap_or_default()
}

thread_local! {
    /// `GlobalObjectSystem.objectListPool`.
    static LIST_POOL: RefCell<VecDeque<Value>> = RefCell::default();
}

/// `allocList()`.
fn alloc_list() -> Value {
    LIST_POOL
        .with(|p| p.borrow_mut().pop_front())
        .unwrap_or_else(|| list_value(Vec::new()))
}

fn list_items(v: &Value) -> Option<&RefCell<Vec<Value>>> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<JavaList>().map(|l| &l.items),
        _ => None,
    }
}

/// `ArrayList.equals`.
fn lists_equal(a: &Value, b: &Value) -> bool {
    match (list_items(a), list_items(b)) {
        (Some(a), Some(b)) => {
            let (a, b) = (a.borrow(), b.borrow());
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| java_equals(x, y))
        }
        _ => false,
    }
}

/// `finishedWithList(list)`.
fn finished_with_list(list: Value) {
    let Some(items) = list_items(&list) else {
        return;
    };
    let pooled = LIST_POOL.with(|p| p.borrow().iter().any(|q| lists_equal(q, &list)));
    if !pooled {
        items.borrow_mut().clear();
        LIST_POOL.with(|p| p.borrow_mut().push_back(list.clone()));
    }
}

fn fill_list(objects: Vec<Rc<GlobalObject>>) -> Value {
    let list = alloc_list();
    if let Some(items) = list_items(&list) {
        items
            .borrow_mut()
            .extend(objects.into_iter().map(|o| object_value(Some(o))));
    }
    list
}

/// Coordinates of `getObjectAt(x, y, z)` or `getObjectAt(square)`.
fn coords_arg(c: &mut NativeCall<'_>) -> LuaResult<(i32, i32, i32)> {
    if c.nargs() == 2 {
        let sq = c.arg(1);
        if sq.is_nil() {
            return Err(LuaError::Runtime {
                message: "square is null".into(),
                class: "java.lang.NullPointerException",
            });
        }
        let mut coord = |name: &str| -> LuaResult<i32> {
            let f = c.thread.table_get(&sq, &Value::str(name))?;
            let v = c.thread.call1(&f, std::slice::from_ref(&sq))?;
            Ok(pz_kahlua::exec::raw_tonumber(&v).unwrap_or(0.0) as i32)
        };
        return Ok((coord("getX")?, coord("getY")?, coord("getZ")?));
    }
    Ok((int_arg(c, 1), int_arg(c, 2), int_arg(c, 3)))
}

/// `setModDataKeys` and friends: every value must be a string.
fn key_set(v: &Value) -> LuaResult<HashSet<String>> {
    let mut out = HashSet::new();
    if let Some(t) = v.as_table() {
        for (_, value) in t.entries() {
            match value {
                Value::Str(s) => {
                    out.insert(s.to_string());
                }
                other => {
                    return Err(go::illegal_argument(format!(
                        "expected string but got \"{}\"",
                        pz_kahlua::exec::java_value_of(&other)
                    )))
                }
            }
        }
    }
    Ok(out)
}

pub fn register(mgr: &mut LuaManager) {
    register_with(&mut mgr.thread, mgr.events.clone());
}

pub fn register_with(thread: &mut Thread, events: EventManager) {
    register_objects(thread);
    register_systems(thread);
    register_statics(
        thread,
        Side::Client,
        "zombie.globalObjects.CGlobalObjects",
        events.clone(),
    );
    register_statics(
        thread,
        Side::Server,
        "zombie.globalObjects.SGlobalObjects",
        events,
    );
}

fn register_objects(thread: &mut Thread) {
    let b = expose_class(thread, GLOBAL_OBJECT, Some("java.lang.Object"));
    let obj = |c: &NativeCall<'_>| {
        object_arg(c, 0).ok_or_else(|| {
            LuaError::runtime(format!(
                "Expected a GlobalObject but got {}",
                pz_kahlua::stdlib::java_class_name(&c.arg(0))
            ))
        })
    };
    b.method("getSystem", move |c| {
        let o = obj(c)?;
        Ok(c.push(system_value(o.system())))
    });
    b.method("setLocation", |_| Ok(0));
    b.method("getX", move |c| {
        let o = obj(c)?;
        Ok(c.push(o.x))
    });
    b.method("getY", move |c| {
        let o = obj(c)?;
        Ok(c.push(o.y))
    });
    b.method("getZ", move |c| {
        let o = obj(c)?;
        Ok(c.push(o.z))
    });
    b.method("getModData", move |c| {
        let o = obj(c)?;
        Ok(c.push(o.mod_data.clone()))
    });
    b.method("Reset", move |c| {
        obj(c)?.reset();
        Ok(0)
    });
    b.method("isValidIsoObject", move |c| {
        let o = obj(c)?;
        let valid = !c.arg(1).is_nil() && o.is_valid_iso_object();
        Ok(c.push(valid))
    });
    expose_class(thread, C_OBJECT, Some(GLOBAL_OBJECT));
    expose_class(thread, S_OBJECT, Some(GLOBAL_OBJECT));
}

fn register_systems(thread: &mut Thread) {
    let b = expose_class(thread, GLOBAL_OBJECT_SYSTEM, Some("java.lang.Object"));
    b.method("getName", |c| {
        let s = this_system(c)?;
        Ok(c.push(s.name.as_str()))
    });
    b.method("getModData", |c| {
        let s = this_system(c)?;
        Ok(c.push(s.mod_data.clone()))
    });
    b.method("newObject", |c| {
        let s = this_system(c)?;
        let o = s.new_object(int_arg(c, 1), int_arg(c, 2), int_arg(c, 3))?;
        Ok(c.push(object_value(Some(o))))
    });
    b.method("removeObject", |c| {
        let s = this_system(c)?;
        let o = object_arg(c, 1).ok_or_else(|| LuaError::Runtime {
            message: "object is null".into(),
            class: "java.lang.NullPointerException",
        })?;
        s.remove_object(&o)?;
        Ok(0)
    });
    b.method("getObjectAt", |c| {
        let s = this_system(c)?;
        let (x, y, z) = coords_arg(c)?;
        let o = s.get_object_at(x, y, z)?;
        Ok(c.push(object_value(o)))
    });
    b.method("hasObjectsInChunk", |c| {
        let s = this_system(c)?;
        let r = s.has_objects_in_chunk(int_arg(c, 1), int_arg(c, 2))?;
        Ok(c.push(r))
    });
    b.method("getObjectsInChunk", |c| {
        let s = this_system(c)?;
        let mut out = Vec::new();
        s.objects_in_chunk(int_arg(c, 1), int_arg(c, 2), &mut out)?;
        Ok(c.push(fill_list(out)))
    });
    b.method("getObjectsAdjacentTo", |c| {
        let s = this_system(c)?;
        let mut out = Vec::new();
        s.objects_adjacent_to(int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), &mut out)?;
        Ok(c.push(fill_list(out)))
    });
    b.method("getObjectCount", |c| {
        let s = this_system(c)?;
        Ok(c.push(s.object_count() as i32))
    });
    b.method("getObjectByIndex", |c| {
        let s = this_system(c)?;
        Ok(c.push(object_value(s.object_by_index(int_arg(c, 1)))))
    });
    b.method("allocList", |c| Ok(c.push(alloc_list())));
    b.method("finishedWithList", |c| {
        finished_with_list(c.arg(1));
        Ok(0)
    });
    b.method("Reset", |c| {
        this_system(c)?.reset();
        Ok(0)
    });

    // CGlobalObjectSystem.
    let b = expose_class(thread, C_SYSTEM, Some(GLOBAL_OBJECT_SYSTEM));
    b.ctor(|c| {
        let s = GlobalObjectSystem::new(Side::Client, &string_arg(c, 0))?;
        Ok(c.push(system_value(Some(s))))
    });
    b.method("sendCommand", |c| {
        let s = this_system(c)?;
        go::send_client_command(&s, &string_arg(c, 1), c.arg(2), &c.arg(3));
        Ok(0)
    });
    b.method("receiveServerCommand", |c| {
        let s = this_system(c)?;
        let (command, args) = (c.arg(1), c.arg(2));
        s.receive_server_command(c.thread, command, args)?;
        Ok(0)
    });
    b.method("receiveNewLuaObjectAt", |c| {
        let s = this_system(c)?;
        let args = c.arg(4).as_table().cloned().unwrap_or_default();
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        s.receive_new_lua_object_at(c.thread, x, y, z, &args)?;
        Ok(0)
    });
    b.method("receiveRemoveLuaObjectAt", |c| {
        let s = this_system(c)?;
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        s.receive_remove_lua_object_at(c.thread, x, y, z)?;
        Ok(0)
    });
    b.method("receiveUpdateLuaObjectAt", |c| {
        let s = this_system(c)?;
        let args = c.arg(4).as_table().cloned().unwrap_or_default();
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        s.receive_update_lua_object_at(c.thread, x, y, z, &args)?;
        Ok(0)
    });

    // SGlobalObjectSystem.
    let b = expose_class(thread, S_SYSTEM, Some(GLOBAL_OBJECT_SYSTEM));
    b.ctor(|c| {
        let s = GlobalObjectSystem::new(Side::Server, &string_arg(c, 0))?;
        Ok(c.push(system_value(Some(s))))
    });
    b.method("sendCommand", |c| {
        let s = this_system(c)?;
        go::send_server_command(&s, &string_arg(c, 1), &c.arg(2));
        Ok(0)
    });
    b.method("update", |_| Ok(0));
    b.method("chunkLoaded", |c| {
        let s = this_system(c)?;
        let (wx, wy) = (int_arg(c, 1), int_arg(c, 2));
        s.chunk_loaded(c.thread, wx, wy)?;
        Ok(0)
    });
    b.method("loadedWorldVersion", |c| {
        let s = this_system(c)?;
        Ok(c.push(s.loaded_world_version.get()))
    });
    type Keys = fn(&GlobalObjectSystem) -> &RefCell<HashSet<String>>;
    let key_setters: [(&str, Keys); 3] = [
        ("setModDataKeys", |s| &s.mod_data_keys),
        ("setObjectModDataKeys", |s| &s.object_mod_data_keys),
        ("setObjectSyncKeys", |s| &s.object_sync_keys),
    ];
    for (name, field) in key_setters {
        b.method(name, move |c| {
            let s = this_system(c)?;
            let cell = field(&s);
            cell.borrow_mut().clear();
            let keys = key_set(&c.arg(1))?;
            *cell.borrow_mut() = keys;
            Ok(0)
        });
    }
    b.method("receiveClientCommand", |c| {
        let s = this_system(c)?;
        let (command, player, args) = (c.arg(1), c.arg(2), c.arg(3));
        s.receive_client_command(c.thread, command, player, args)?;
        Ok(0)
    });
    b.method("getInitialStateForClient", |c| {
        let s = this_system(c)?;
        let t = s.initial_state_for_client(c.thread)?;
        Ok(c.push(t.map_or(Value::Nil, Value::Table)))
    });
    for (name, kind) in [
        ("addGlobalObjectOnClient", ObjectSync::Add),
        ("removeGlobalObjectOnClient", ObjectSync::Remove),
        ("updateGlobalObjectOnClient", ObjectSync::Update),
    ] {
        b.method(name, move |c| {
            let s = this_system(c)?;
            go::sync_object_to_client(&s, object_arg(c, 1).as_ref(), kind)?;
            Ok(0)
        });
    }
}

fn register_statics(thread: &mut Thread, side: Side, fqn: &'static str, events: EventManager) {
    let b = expose_class(thread, fqn, Some("java.lang.Object"));
    b.static_fn("noise", move |c| {
        go::noise(side, &pz_kahlua::exec::java_value_of(&c.arg(0)));
        Ok(0)
    });
    b.static_fn("getSystemCount", move |c| {
        Ok(c.push(go::system_count(side) as i32))
    });
    b.static_fn("getSystemByIndex", move |c| {
        Ok(c.push(system_value(go::system_by_index(side, int_arg(c, 0)))))
    });
    b.static_fn("getSystemByName", move |c| {
        let name = string_arg(c, 0);
        Ok(c.push(system_value(go::system_by_name(side, &name))))
    });
    b.static_fn("newSystem", move |c| {
        let s = go::new_system(side, &string_arg(c, 0))?;
        Ok(c.push(system_value(Some(s))))
    });
    b.static_fn("registerSystem", move |c| {
        let s = go::register_system(side, &string_arg(c, 0))?;
        Ok(c.push(system_value(Some(s))))
    });
    b.static_fn("Reset", move |_| {
        go::reset(side);
        Ok(0)
    });
    b.static_fn("initSystems", move |c| {
        match side {
            Side::Client => go::init_client_systems(c.thread, &events),
            Side::Server => go::init_server_systems(c.thread, &events),
        }
        Ok(0)
    });
    match side {
        Side::Client => {
            b.static_fn("receiveServerCommand", |c| {
                let (system, command, args) = (string_arg(c, 0), string_arg(c, 1), c.arg(2));
                let r = go::client_receive_server_command(c.thread, &system, &command, args)?;
                Ok(c.push(r))
            });
        }
        Side::Server => {
            b.static_fn("receiveClientCommand", |c| {
                let (system, command, player, args) =
                    (string_arg(c, 0), string_arg(c, 1), c.arg(2), c.arg(3));
                let r =
                    go::server_receive_client_command(c.thread, &system, &command, player, args)?;
                Ok(c.push(r))
            });
            b.static_fn("chunkLoaded", |c| {
                let (wx, wy) = (int_arg(c, 0), int_arg(c, 1));
                go::chunk_loaded(c.thread, wx, wy)?;
                Ok(0)
            });
            b.static_fn("update", |_| Ok(0));
            b.static_fn("load", |_| Ok(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventManager, EVENT_NAMES};
    use crate::global_objects::GridBounds;
    use pz_kahlua::TableRef;

    fn thread() -> (Thread, EventManager) {
        let env = TableRef::new();
        let mut t = Thread::new(env.clone());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        let events = EventManager::register(&env, "Events", EVENT_NAMES);
        crate::bind_java_util::register(&mut t);
        register_with(&mut t, events.clone());
        (t, events)
    }

    fn run(thread: &mut Thread, src: &str) -> Vec<Value> {
        let proto = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(proto),
            thread.globals().clone(),
        ));
        thread.call(&f, &[]).unwrap_or_else(|e| panic!("{e}"))
    }

    fn s(v: &Value) -> String {
        pz_kahlua::exec::java_value_of(v)
    }

    #[test]
    fn registry_counts_and_lookups() {
        let (mut t, _) = thread();
        let r = run(
            &mut t,
            "local n0 = CGlobalObjects.getSystemCount()
             local a = CGlobalObjects.registerSystem('alpha')
             local again = CGlobalObjects.registerSystem('alpha')
             CGlobalObjects.newSystem('beta')
             local ok = pcall(CGlobalObjects.newSystem, 'beta')
             return n0, CGlobalObjects.getSystemCount(), a == again, CGlobalObjects.getSystemByIndex(1):getName(),
                    CGlobalObjects.getSystemByIndex(2), ok, SGlobalObjects.getSystemCount(),
                    CGlobalObjects.getSystemByName('alpha') == a, pcall(CGlobalObjects.newSystem, 'a..b')",
        );
        let got: Vec<String> = r.iter().take(8).map(s).collect();
        assert_eq!(
            got,
            ["0.0", "2.0", "true", "beta", "null", "false", "0.0", "true"]
        );
        assert!(!r[8].truthy());
    }

    #[test]
    fn objects_need_lookup_and_follow_it() {
        let (mut t, _) = thread();
        go::reset(Side::Server);
        let r = run(
            &mut t,
            "local s = SGlobalObjects.registerSystem('x'); return pcall(s.newObject, s, 1, 1, 0)",
        );
        assert!(!r[0].truthy(), "no meta grid yet: NullPointerException");
        go::init_lookup(GridBounds {
            min_x: 0,
            min_y: 0,
            width: 2,
            height: 2,
        });
        let r = run(
            &mut t,
            "local s = SGlobalObjects.getSystemByName('x')
             local o = s:newObject(10, 20, 0)
             o:getModData().state = 'grown'
             local dup = pcall(s.newObject, s, 10, 20, 0)
             local adj = s:getObjectsAdjacentTo(11, 21, 0)
             local inChunk = s:hasObjectsInChunk(1, 2)
             local outside = s:getObjectAt(9999, 0, 0)
             local n = s:getObjectCount()
             s:removeObject(o)
             return dup, adj:size(), inChunk, outside, n, s:getObjectCount(), o:getSystem(), s:getObjectAt(10, 20, 0)",
        );
        let got: Vec<String> = r.iter().map(s).collect();
        assert_eq!(
            got,
            ["false", "1.0", "true", "null", "1.0", "0.0", "null", "null"]
        );
        go::reset(Side::Server);
    }

    #[test]
    fn single_player_handover_and_commands() {
        let (mut t, events) = thread();
        go::reset(Side::Server);
        go::reset(Side::Client);
        go::init_lookup(GridBounds {
            min_x: 0,
            min_y: 0,
            width: 1,
            height: 1,
        });
        run(
            &mut t,
            "local s = SGlobalObjects.registerSystem('farm')
             s:setObjectSyncKeys({'state'})
             local md = s:getModData()
             md.getInitialStateForClient = function(self) return { season = 'spring' } end
             md.OnClientCommand = function(self, cmd, player, args) self.got = cmd .. args.n end
             local o = s:newObject(3, 4, 0)
             o:getModData().state = 'seeded'
             o:getModData().secret = 'x'
             SERVER = s",
        );
        go::init_server_systems(&mut t, &events);
        let r = run(
            &mut t,
            "local c = CGlobalObjects.registerSystem('farm')
             local md = c:getModData()
             local o = c:getObjectAt(3, 4, 0)
             md.OnServerCommand = function(self, cmd, args) self.heard = cmd end
             c:sendCommand('water', {}, { n = 2 })
             CLIENT = c
             return md.season, o:getModData().state, o:getModData().secret, c:getObjectCount()",
        );
        let got: Vec<String> = r.iter().map(s).collect();
        assert_eq!(got, ["spring", "seeded", "null", "1.0"]);
        go::update_server(&mut t);
        run(&mut t, "SERVER:sendCommand('ack', { a = 1 })");
        go::update_client(&mut t);
        let r = run(
            &mut t,
            "return SERVER:getModData().got, CLIENT:getModData().heard",
        );
        let got: Vec<String> = r.iter().map(s).collect();
        assert_eq!(got, ["water2", "ack"]);
        go::reset(Side::Server);
        go::reset(Side::Client);
    }

    #[test]
    fn list_pool_reuses_lists() {
        let (mut t, _) = thread();
        let r = run(
            &mut t,
            "local s = CGlobalObjectSystem.new('p')
             local l = s:allocList()
             l:add(1)
             s:finishedWithList(l)
             return s:allocList() == l, l:size()",
        );
        assert_eq!(r.iter().map(s).collect::<Vec<_>>(), ["true", "0.0"]);
    }
}
