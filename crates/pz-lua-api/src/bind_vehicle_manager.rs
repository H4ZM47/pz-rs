//! Build 42.22 `zombie.vehicles.VehicleIDMap`,
//! `zombie.vehicles.VehicleManager`, and `zombie.vehicles.VehicleCache`.

use crate::bind_java_util::JavaList;
use crate::expose::{expose_class, int_arg, num_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

pub const VEHICLE_ID_MAP_CLASS: &str = "zombie.vehicles.VehicleIDMap";
pub const VEHICLE_MANAGER_CLASS: &str = "zombie.vehicles.VehicleManager";
pub const VEHICLE_CACHE_CLASS: &str = "zombie.vehicles.VehicleCache";

const MAX_VEHICLE_ID: i16 = i16::MAX;

#[derive(Default)]
struct VehicleIdMapState {
    by_id: BTreeMap<i16, Value>,
    next_id: i16,
}

impl VehicleIdMapState {
    fn reset(&mut self) {
        self.by_id.clear();
        self.next_id = 0;
    }

    fn allocate_id(&mut self) -> i16 {
        let mut tries = 0usize;
        while tries < 32767 {
            self.next_id = if self.next_id >= MAX_VEHICLE_ID || self.next_id <= 0 {
                1
            } else {
                self.next_id + 1
            };
            if !self.by_id.contains_key(&self.next_id) {
                return self.next_id;
            }
            tries += 1;
        }
        -1
    }
}

#[derive(Default)]
struct VehicleManagerState {
    /// `towedVehicleMap`: towing vehicle ID -> towed vehicle ID (bidirectional
    /// lookup via helper).
    towed: HashMap<i16, i16>,
    initialized: bool,
}

impl VehicleManagerState {
    fn reset(&mut self) {
        self.towed.clear();
        self.initialized = true;
    }
}

#[derive(Clone, Copy, Debug)]
struct CachedVehicle {
    id: i16,
    x: f32,
    y: f32,
}

#[derive(Default)]
struct VehicleCacheState {
    by_id: HashMap<i16, CachedVehicle>,
    by_chunk: HashMap<(i32, i32), Vec<i16>>,
}

impl VehicleCacheState {
    fn reset(&mut self) {
        self.by_id.clear();
        self.by_chunk.clear();
    }

    fn remove(&mut self, id: i16) {
        if let Some(old) = self.by_id.remove(&id) {
            let key = (
                (old.x.floor() as i32).div_euclid(8),
                (old.y.floor() as i32).div_euclid(8),
            );
            if let Some(list) = self.by_chunk.get_mut(&key) {
                list.retain(|&v| v != id);
                if list.is_empty() {
                    self.by_chunk.remove(&key);
                }
            }
        }
    }

    fn update(&mut self, id: i16, x: f32, y: f32) {
        self.remove(id);
        let key = (
            (x.floor() as i32).div_euclid(8),
            (y.floor() as i32).div_euclid(8),
        );
        self.by_id.insert(id, CachedVehicle { id, x, y });
        self.by_chunk.entry(key).or_default().push(id);
    }

    fn get_chunk(&self, cx: i32, cy: i32) -> Vec<CachedVehicle> {
        self.by_chunk
            .get(&(cx, cy))
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.by_id.get(id).copied())
                    .collect()
            })
            .unwrap_or_default()
    }
}

struct LuaVehicleIdMap;
impl LuaObject for LuaVehicleIdMap {
    fn class_name(&self) -> &str {
        VEHICLE_ID_MAP_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaVehicleManager;
impl LuaObject for LuaVehicleManager {
    fn class_name(&self) -> &str {
        VEHICLE_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaVehicleCacheEntry(CachedVehicle);
impl LuaObject for LuaVehicleCacheEntry {
    fn class_name(&self) -> &str {
        VEHICLE_CACHE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static ID_MAP: RefCell<VehicleIdMapState> = RefCell::new(VehicleIdMapState::default());
    static MANAGER: RefCell<VehicleManagerState> = RefCell::new(VehicleManagerState::default());
    static CACHE: RefCell<VehicleCacheState> = RefCell::new(VehicleCacheState::default());
}

/// `VehicleIDMap.instance.Reset()`.
pub fn reset_vehicle_id_map() {
    ID_MAP.with(|m| m.borrow_mut().reset());
}

/// `new VehicleManager()` (`VehicleManager.instance` initialization on world load).
pub fn init_vehicle_manager() {
    MANAGER.with(|m| m.borrow_mut().reset());
}

/// `VehicleCache.Reset()`.
pub fn reset_vehicle_cache() {
    CACHE.with(|c| c.borrow_mut().reset());
}

/// Reset all vehicle manager/id/cache state on session teardown.
pub(crate) fn reset_session_handles() {
    reset_vehicle_id_map();
    init_vehicle_manager();
    reset_vehicle_cache();
}

fn vehicle_id_of(c: &mut pz_kahlua::NativeCall<'_>, v: &Value) -> Option<i16> {
    if let Value::Num(n) = v {
        return Some(*n as i16);
    }
    if v.is_nil() {
        return None;
    }
    let f = c.thread.table_get(v, &Value::str("getId")).ok()?;
    if f.is_nil() {
        return None;
    }
    let res = c.thread.call1(&f, &[v.clone()]).ok()?;
    pz_kahlua::exec::raw_tonumber(&res).map(|n| n as i16)
}

pub fn register(thread: &mut Thread) {
    register_vehicle_id_map(thread);
    register_vehicle_manager(thread);
    register_vehicle_cache(thread);
}

fn register_vehicle_id_map(thread: &mut Thread) {
    let cls = expose_class(thread, VEHICLE_ID_MAP_CLASS, Some("java.lang.Object"));
    cls.field("instance", Value::Object(Rc::new(LuaVehicleIdMap)));

    cls.method("put", |c| {
        let id = int_arg(c, 1) as i16;
        let val = c.arg(2);
        ID_MAP.with(|m| {
            let mut m = m.borrow_mut();
            if val.is_nil() {
                m.by_id.remove(&id);
            } else {
                m.by_id.insert(id, val);
            }
        });
        Ok(0)
    });
    cls.method("remove", |c| {
        let id = int_arg(c, 1) as i16;
        ID_MAP.with(|m| m.borrow_mut().by_id.remove(&id));
        Ok(0)
    });
    cls.method("get", |c| {
        let id = int_arg(c, 1) as i16;
        let val = ID_MAP.with(|m| m.borrow().by_id.get(&id).cloned().unwrap_or(Value::Nil));
        Ok(c.push(val))
    });
    cls.method("containsKey", |c| {
        let id = int_arg(c, 1) as i16;
        let has = ID_MAP.with(|m| m.borrow().by_id.contains_key(&id));
        Ok(c.push(has))
    });
    cls.method("toArrayList", |c| {
        let items: Vec<Value> = ID_MAP.with(|m| m.borrow().by_id.values().cloned().collect());
        if let Value::Object(o) = c.arg(1) {
            if let Some(list) = o.as_any().downcast_ref::<JavaList>() {
                for item in items {
                    list.push(item)?;
                }
                return Ok(0);
            }
        }
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    cls.method("Reset", |_c| {
        reset_vehicle_id_map();
        Ok(0)
    });
    cls.method("allocateID", |c| {
        let id = ID_MAP.with(|m| m.borrow_mut().allocate_id());
        Ok(c.push(id as i32))
    });
}

fn register_vehicle_manager(thread: &mut Thread) {
    let cls = expose_class(thread, VEHICLE_MANAGER_CLASS, Some("java.lang.Object"));
    let inst = Value::Object(Rc::new(LuaVehicleManager));
    cls.field("instance", inst.clone());
    cls.ctor(move |c| {
        init_vehicle_manager();
        Ok(c.push(inst.clone()))
    });

    cls.method("registerVehicle", |c| {
        let veh = c.arg(1);
        if let Some(id) = vehicle_id_of(c, &veh) {
            ID_MAP.with(|m| m.borrow_mut().by_id.insert(id, veh));
        }
        Ok(0)
    });
    cls.method("unregisterVehicle", |c| {
        let veh = c.arg(1);
        if let Some(id) = vehicle_id_of(c, &veh) {
            ID_MAP.with(|m| m.borrow_mut().by_id.remove(&id));
            MANAGER.with(|m| {
                let mut m = m.borrow_mut();
                m.towed.remove(&id);
                m.towed.retain(|_, v| *v != id);
            });
            CACHE.with(|ca| ca.borrow_mut().remove(id));
        }
        Ok(0)
    });
    cls.method("getVehicleByID", |c| {
        let id = int_arg(c, 1) as i16;
        let val = ID_MAP.with(|m| m.borrow().by_id.get(&id).cloned().unwrap_or(Value::Nil));
        Ok(c.push(val))
    });
    cls.method("getVehicles", |c| {
        let items: Vec<Value> = ID_MAP.with(|m| m.borrow().by_id.values().cloned().collect());
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    cls.method("removeFromWorld", |c| {
        let veh = c.arg(1);
        if let Some(id) = vehicle_id_of(c, &veh) {
            ID_MAP.with(|m| m.borrow_mut().by_id.remove(&id));
            CACHE.with(|ca| ca.borrow_mut().remove(id));
        }
        Ok(0)
    });
    cls.method("attachTowing", |c| {
        let v1 = c.arg(1);
        let v2 = c.arg(2);
        if let (Some(id1), Some(id2)) = (vehicle_id_of(c, &v1), vehicle_id_of(c, &v2)) {
            MANAGER.with(|m| m.borrow_mut().towed.insert(id1, id2));
        }
        Ok(0)
    });
    cls.method("detachTowing", |c| {
        let v1 = c.arg(1);
        let v2 = c.arg(2);
        let id1 = vehicle_id_of(c, &v1);
        let id2 = vehicle_id_of(c, &v2);
        MANAGER.with(|m| {
            let mut m = m.borrow_mut();
            if let Some(a) = id1 {
                m.towed.remove(&a);
            }
            if let Some(b) = id2 {
                m.towed.retain(|_, v| *v != b);
            }
        });
        Ok(0)
    });
    cls.method("getTowedVehicleID", |c| {
        let id = int_arg(c, 1) as i16;
        let towed = MANAGER.with(|m| m.borrow().towed.get(&id).copied().unwrap_or(-1));
        Ok(c.push(towed as i32))
    });
    cls.method("serverUpdate", |_c| Ok(0));
    cls.method("clientUpdate", |_c| Ok(0));
}

fn register_vehicle_cache(thread: &mut Thread) {
    let cls = expose_class(thread, VEHICLE_CACHE_CLASS, Some("java.lang.Object"));

    cls.static_fn("vehicleUpdate", |c| {
        let id = int_arg(c, 0) as i16;
        let x = num_arg(c, 1) as f32;
        let y = num_arg(c, 2) as f32;
        CACHE.with(|ca| ca.borrow_mut().update(id, x, y));
        Ok(0)
    });
    cls.static_fn("vehicleGet", |c| {
        let x = num_arg(c, 0);
        let y = num_arg(c, 1);
        let cx = (x.floor() as i32).div_euclid(8);
        let cy = (y.floor() as i32).div_euclid(8);
        let items: Vec<Value> = CACHE.with(|ca| {
            ca.borrow()
                .get_chunk(cx, cy)
                .into_iter()
                .map(|v| Value::Object(Rc::new(LuaVehicleCacheEntry(v))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    cls.static_fn("remove", |c| {
        let id = int_arg(c, 0) as i16;
        CACHE.with(|ca| ca.borrow_mut().remove(id));
        Ok(0)
    });
    cls.static_fn("Reset", |_c| {
        reset_vehicle_cache();
        Ok(0)
    });

    cls.method("getId", |c| {
        let id = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaVehicleCacheEntry>()
                .map(|e| e.0.id as i32)
                .unwrap_or(-1),
            _ => -1,
        };
        Ok(c.push(id))
    });
    cls.method("getX", |c| {
        let x = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaVehicleCacheEntry>()
                .map(|e| e.0.x as f64)
                .unwrap_or(0.0),
            _ => 0.0,
        };
        Ok(c.push(x))
    });
    cls.method("getY", |c| {
        let y = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaVehicleCacheEntry>()
                .map(|e| e.0.y as f64)
                .unwrap_or(0.0),
            _ => 0.0,
        };
        Ok(c.push(y))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib;

    #[test]
    fn vehicle_id_map_manager_and_cache_lua() {
        reset_session_handles();
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let src = r#"
            local function assert(v, msg)
                if not v then error(msg or "assertion failed") end
            end

            -- VehicleIDMap
            local id1 = VehicleIDMap.instance:allocateID()
            local id2 = VehicleIDMap.instance:allocateID()
            assert(id1 == 1 and id2 == 2)
            local v1 = { getId = function() return id1 end }
            local v2 = { getId = function() return id2 end }
            VehicleIDMap.instance:put(id1, v1)
            assert(VehicleIDMap.instance:containsKey(id1) == true)
            assert(VehicleIDMap.instance:get(id1) == v1)

            -- VehicleManager
            VehicleManager.instance:registerVehicle(v2)
            assert(VehicleManager.instance:getVehicleByID(id2) == v2)
            assert(VehicleManager.instance:getVehicles():size() == 2)
            VehicleManager.instance:attachTowing(v1, v2, "trailer", "trailer")
            assert(VehicleManager.instance:getTowedVehicleID(id1) == id2)
            VehicleManager.instance:detachTowing(v1, v2)
            assert(VehicleManager.instance:getTowedVehicleID(id1) == -1)

            -- VehicleCache
            VehicleCache.vehicleUpdate(id1, 16.5, 24.2, 0.0)
            local found = VehicleCache.vehicleGet(17.0, 25.0)
            assert(found:size() == 1)
            assert(found:get(0):getId() == id1)
            VehicleCache.remove(id1)
            assert(VehicleCache.vehicleGet(17.0, 25.0):size() == 0)

            VehicleIDMap.instance:Reset()
            assert(VehicleIDMap.instance:containsKey(id1) == false)
            assert(VehicleManager.instance:getVehicles():size() == 0)
        "#;
        let f = stdlib::load_string(src, Some("vehicle_manager"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();
    }
}
