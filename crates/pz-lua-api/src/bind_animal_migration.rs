//! Lua bindings for Build 42.22 animal migration & zone classes:
//! - `zombie.characters.animals.MigrationGroupDefinitions`
//! - `zombie.characters.animals.AnimalZone`
//! - `zombie.characters.animals.AnimalPopulationManager`

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_characters::pz_animals::population::zone;
use pz_characters::pz_animals::{JHashMap, MigrationGroupDefinitions};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_world::zones::{AnimalZoneData, Zone, ZoneGeometryType, ZoneKind};

use crate::bind_java_util::JavaMap;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, this};

pub const MIGRATION_GROUP_DEFS_CLASS: &str =
    "zombie.characters.animals.MigrationGroupDefinitions";
pub const ANIMAL_ZONE_CLASS: &str = "zombie.characters.animals.AnimalZone";
pub const ANIMAL_POPULATION_MANAGER_CLASS: &str =
    "zombie.characters.animals.AnimalPopulationManager";

thread_local! {
    static MIGRATION_DEFS: RefCell<Option<Rc<JHashMap<MigrationGroupDefinitions>>>> =
        const { RefCell::new(None) };
    static RNG_STATE: RefCell<u64> = const { RefCell::new(0x6d69_6772_6174_6501) };
}

pub fn reset() {
    MIGRATION_DEFS.with(|slot| *slot.borrow_mut() = None);
}

struct MigrationRand;

impl pz_world::GameRand for MigrationRand {
    fn next(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let n = RNG_STATE.with(|s| {
            let mut st = s.borrow_mut();
            let mut x = *st;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *st = x;
            x
        });
        (n % (bound as u64)) as i32
    }

    fn next_float(&mut self) -> f32 {
        (self.next(1_000_000) as f32) / 1_000_000.0
    }
}

fn reload_migration_defs(env: &TableRef) -> LuaResult<Rc<JHashMap<MigrationGroupDefinitions>>> {
    let map = if env.get_str("MigrationGroupDefinitions").as_table().is_some() {
        // Skip Java static methods when scanning the global table: build a filtered
        // shadow table containing only table-valued group entries.
        let src = env.get_str("MigrationGroupDefinitions").as_table().cloned().unwrap();
        let filtered = TableRef::new();
        for (k, v) in src.entries() {
            if matches!(v, Value::Table(_)) {
                filtered.rawset(k, v);
            }
        }
        let shadow_env = TableRef::new();
        shadow_env.set_str("MigrationGroupDefinitions", Value::Table(filtered));
        MigrationGroupDefinitions::load_all(&shadow_env)
            .map_err(|e| LuaError::runtime(format!("MigrationGroupDefinitions: {e}")))?
    } else {
        JHashMap::new()
    };
    let rc = Rc::new(map);
    MIGRATION_DEFS.with(|slot| *slot.borrow_mut() = Some(rc.clone()));
    Ok(rc)
}

fn ensure_migration_defs(env: &TableRef) -> LuaResult<Rc<JHashMap<MigrationGroupDefinitions>>> {
    if let Some(existing) = MIGRATION_DEFS.with(|slot| slot.borrow().clone()) {
        return Ok(existing);
    }
    reload_migration_defs(env)
}

fn opt_str(s: &Option<String>) -> Value {
    match s {
        Some(v) => Value::str(v.as_str()),
        None => Value::Nil,
    }
}

fn world_age_hours() -> f64 {
    crate::bind_climate::shared_game_time()
        .borrow()
        .world_age_hours()
}

fn write_vec2_out(c: &mut NativeCall<'_>, idx: usize, x: f32, y: f32) -> LuaResult<()> {
    let target = c.arg(idx);
    match &target {
        Value::Table(t) => {
            t.set_str("x", Value::Num(x as f64));
            t.set_str("y", Value::Num(y as f64));
        }
        Value::Object(_) => {
            let set_fn = c.thread.table_get(&target, &Value::str("set"))?;
            if !set_fn.is_nil() {
                let _ = c
                    .thread
                    .pcall(&set_fn, &[target.clone(), Value::Num(x as f64), Value::Num(y as f64)]);
            }
        }
        _ => {}
    }
    Ok(())
}

// ---- userdata wrappers ------------------------------------------------------

#[derive(Clone)]
pub struct LuaMigrationGroupDef(pub Rc<MigrationGroupDefinitions>);

impl LuaObject for LuaMigrationGroupDef {
    fn class_name(&self) -> &str {
        MIGRATION_GROUP_DEFS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub struct LuaAnimalZone(pub Rc<RefCell<Zone>>);

impl LuaObject for LuaAnimalZone {
    fn class_name(&self) -> &str {
        ANIMAL_ZONE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaAnimalPopManager;

impl LuaObject for LuaAnimalPopManager {
    fn class_name(&self) -> &str {
        ANIMAL_POPULATION_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn animal_zone_value(z: Zone) -> Value {
    Value::Object(Rc::new(LuaAnimalZone(Rc::new(RefCell::new(z)))))
}

/// Registers `MigrationGroupDefinitions`, `AnimalZone`, and
/// `AnimalPopulationManager` on `thread`.
pub fn register(thread: &mut Thread) {
    register_migration_group_defs(thread);
    register_animal_zone(thread);
    register_animal_pop_manager(thread);
}

fn register_migration_group_defs(thread: &mut Thread) {
    if let Value::Table(meta) = thread.class_metatables.get_str(MIGRATION_GROUP_DEFS_CLASS) {
        if let Value::Table(methods) = meta.get_str("__pz_methods") {
            meta.set_str("__index", methods);
        }
    }
    let b = expose_class(thread, MIGRATION_GROUP_DEFS_CLASS, None);
    b.static_fn("loadMigrationsDefinitions", |c| {
        let env = c.thread.globals().clone();
        reload_migration_defs(&env)?;
        Ok(0)
    });
    b.static_fn("Reset", |_c| {
        reset();
        Ok(0)
    });
    b.static_fn("getMigrationDefs", |c| {
        let env = c.thread.globals().clone();
        let defs = ensure_migration_defs(&env)?;
        let map = JavaMap::hash_map();
        for (k, v) in defs.iter() {
            map.put(
                Value::str(k.as_str()),
                Value::Object(Rc::new(LuaMigrationGroupDef(Rc::new(v.clone())))),
            );
        }
        Ok(c.push(Value::Object(Rc::new(map))))
    });
    b.static_fn("getNextEatTime", |c| {
        let Some(group) = str_arg(c, 0) else {
            return Ok(c.push(Value::Num(0.0)));
        };
        let env = c.thread.globals().clone();
        let defs = ensure_migration_defs(&env)?;
        let mut rng = MigrationRand;
        let v = defs
            .get(&group)
            .map(|d| d.next_eat_time(world_age_hours(), &mut rng))
            .unwrap_or(0.0);
        Ok(c.push(Value::Num(v)))
    });
    b.static_fn("getNextSleepTime", |c| {
        let Some(group) = str_arg(c, 0) else {
            return Ok(c.push(Value::Num(0.0)));
        };
        let env = c.thread.globals().clone();
        let defs = ensure_migration_defs(&env)?;
        let mut rng = MigrationRand;
        let v = defs
            .get(&group)
            .map(|d| d.next_sleep_time(world_age_hours(), &mut rng))
            .unwrap_or(0.0);
        Ok(c.push(Value::Num(v)))
    });

    b.method("getRandBreed", |c| {
        let o = this::<LuaMigrationGroupDef>(c)?;
        let d = &o.as_any().downcast_ref::<LuaMigrationGroupDef>().unwrap().0;
        let mut rng = MigrationRand;
        let s = d
            .rand_breed(&mut rng)
            .map_err(|e| LuaError::runtime(format!("{e}")))?;
        Ok(c.push(Value::str(&s)))
    });

    let methods = b.methods.clone();
    if let Value::Table(meta) = thread.class_metatables.get_str(MIGRATION_GROUP_DEFS_CLASS) {
        meta.set_str("__pz_methods", methods.clone());
        meta.set_str(
            "__index",
            Value::Native(pz_kahlua::NativeFunction::new("__index", move |c| {
                let key = c.arg(1);
                let found = methods.rawget(&key);
                if !found.is_nil() {
                    return Ok(c.push(found));
                }
                let Some(name) = key.as_str() else {
                    return Ok(c.push(Value::Nil));
                };
                let o = this::<LuaMigrationGroupDef>(c)?;
                let d = &o.as_any().downcast_ref::<LuaMigrationGroupDef>().unwrap().0;
                let val = match name {
                    "type" => opt_str(&d.type_),
                    "minTimeBeforeSleep" => Value::Num(d.min_time_before_sleep as f64),
                    "maxTimeBeforeSleep" => Value::Num(d.max_time_before_sleep as f64),
                    "minTimeBeforeEat" => Value::Num(d.min_time_before_eat as f64),
                    "maxTimeBeforeEat" => Value::Num(d.max_time_before_eat as f64),
                    "timeToEat" => Value::Num(d.time_to_eat as f64),
                    "timeToSleep" => Value::Num(d.time_to_sleep as f64),
                    "minAnimal" => Value::Num(d.min_animal as f64),
                    "maxAnimal" => Value::Num(d.max_animal as f64),
                    "babyChance" => Value::Num(d.baby_chance as f64),
                    "speed" => Value::Num(d.speed as f64),
                    _ => Value::Nil,
                };
                Ok(c.push(val))
            })),
        );
    }
}

fn register_animal_zone(thread: &mut Thread) {
    let b = expose_class(thread, ANIMAL_ZONE_CLASS, None);
    b.ctor(|c| {
        let mut z = Zone::default();
        if c.nargs() >= 7 {
            z.name = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
            z.zone_type = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            z.x = int_arg(c, 2);
            z.y = int_arg(c, 3);
            z.z = int_arg(c, 4);
            z.w = int_arg(c, 5);
            z.h = int_arg(c, 6);
            z.geometry_type = ZoneGeometryType::Polyline;
            z.points = vec![z.x, z.y, z.x + z.w, z.y + z.h];
        }
        let mut data = AnimalZoneData::default();
        if c.nargs() >= 10 {
            data.action = str_arg(c, 7).map(|s| s.to_string());
            data.animal_type = str_arg(c, 8).map(|s| s.to_string());
            data.spawn_animal = bool_arg(c, 9);
        }
        z.kind = ZoneKind::Animal(data);
        Ok(c.push(animal_zone_value(z)))
    });
    b.method("getAction", |c| {
        let o = this::<LuaAnimalZone>(c)?;
        let z = o.as_any().downcast_ref::<LuaAnimalZone>().unwrap().0.borrow();
        let out = zone::action(&z).map(Value::str).unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    b.method("getClosedPolylineLength", |c| {
        let o = this::<LuaAnimalZone>(c)?;
        let z = o.as_any().downcast_ref::<LuaAnimalZone>().unwrap().0.borrow();
        Ok(c.push(Value::Num(zone::polyline_length(&z) as f64)))
    });
    b.method("getPointOnPolyline", |c| {
        let o = this::<LuaAnimalZone>(c)?;
        let t = num_arg(c, 1) as f32;
        let (ok, (px, py)) = {
            let z = o.as_any().downcast_ref::<LuaAnimalZone>().unwrap().0.borrow();
            zone::point_on_polyline(&z, t)
        };
        if ok {
            write_vec2_out(c, 2, px, py)?;
        }
        Ok(c.push(Value::Bool(ok)))
    });
    b.method("getClosestPointOnPolyline", |c| {
        let o = this::<LuaAnimalZone>(c)?;
        let px = num_arg(c, 1) as f32;
        let py = num_arg(c, 2) as f32;
        let mut out = (0.0, 0.0);
        let t = {
            let z = o.as_any().downcast_ref::<LuaAnimalZone>().unwrap().0.borrow();
            zone::closest_point_on_polyline(&z, px, py, &mut out)
        };
        write_vec2_out(c, 3, out.0, out.1)?;
        Ok(c.push(Value::Num(t as f64)))
    });
    b.method("getDistanceOfPointFromStart", |c| {
        let o = this::<LuaAnimalZone>(c)?;
        let idx = int_arg(c, 1);
        let z = o.as_any().downcast_ref::<LuaAnimalZone>().unwrap().0.borrow();
        Ok(c.push(Value::Num(zone::distance_of_point_from_start(&z, idx) as f64)))
    });
    b.method("getDirectionOnPolyline", |c| {
        let o = this::<LuaAnimalZone>(c)?;
        let t = num_arg(c, 1) as f32;
        let dir = {
            let z = o.as_any().downcast_ref::<LuaAnimalZone>().unwrap().0.borrow();
            zone::direction_on_polyline(&z, t)
        };
        if let Some((dx, dy)) = dir {
            write_vec2_out(c, 2, dx, dy)?;
            Ok(c.push(Value::Bool(true)))
        } else {
            Ok(c.push(Value::Bool(false)))
        }
    });
    b.method("getIndexOfPoint", |c| {
        let o = this::<LuaAnimalZone>(c)?;
        let x = int_arg(c, 1);
        let y = int_arg(c, 2);
        let z = o.as_any().downcast_ref::<LuaAnimalZone>().unwrap().0.borrow();
        Ok(c.push(Value::Num(zone::index_of_point(&z, x, y) as f64)))
    });
    b.method("Dispose", |_c| Ok(0));
    b.method("toString", |c| {
        let o = this::<LuaAnimalZone>(c)?;
        let z = o.as_any().downcast_ref::<LuaAnimalZone>().unwrap().0.borrow();
        Ok(c.push(Value::str(&format!("AnimalZone({}:{})", z.name, z.zone_type))))
    });
}

fn register_animal_pop_manager(thread: &mut Thread) {
    let b = expose_class(thread, ANIMAL_POPULATION_MANAGER_CLASS, None);
    b.static_fn("getInstance", |c| Ok(c.push(Value::Object(Rc::new(LuaAnimalPopManager)))));
    b.method("init", |_c| Ok(0));
    b.method("addChunkToWorld", |_c| Ok(0));
    b.method("removeChunkFromWorld", |_c| Ok(0));
    b.method("virtualizeAnimal", |_c| Ok(0));
    b.method("addToRecentlyRemoved", |_c| Ok(0));
    b.method("update", |_c| Ok(0));
    b.method("save", |_c| Ok(0));
    b.method("saveToBufferMap", |_c| Ok(0));
    b.method("stop", |_c| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_group_defs_and_animal_zone_round_trip_from_lua() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            MigrationGroupDefinitions = MigrationGroupDefinitions or {}
            MigrationGroupDefinitions["deer"] = {
                minTimeBeforeSleep = 200,
                maxTimeBeforeSleep = 400,
                minTimeBeforeEat = 100,
                maxTimeBeforeEat = 200,
                timeToMove = 30,
                minAnimal = 2,
                maxAnimal = 5,
                babyChance = 25,
                speed = 1.2,
                possibleBreed = "whitetailed",
            }

            local defs = MigrationGroupDefinitions.getMigrationDefs()
            assert(defs:size() == 1, "one migration group loaded")
            local deer = defs:get("deer")
            assert(deer ~= nil and deer.minAnimal == 2 and deer.maxAnimal == 5)
            assert(deer:getRandBreed() == "whitetailed")
            assert(MigrationGroupDefinitions.getNextEatTime("deer") >= 0)
            assert(MigrationGroupDefinitions.getNextSleepTime("deer") >= 0)

            local az = AnimalZone.new("trail1", "Animal", 0, 0, 0, 10, 0, "Follow", "deer", true)
            assert(az ~= nil and az:getAction() == "Follow")
            assert(az:getClosedPolylineLength() == 10)
            local out = {}
            assert(az:getPointOnPolyline(0.5, out) == true)
            assert(out.x == 5.5 and out.y == 0.5)
            local dir = {}
            assert(az:getDirectionOnPolyline(0.5, dir) == true)
            assert(dir.x == 1 and dir.y == 0)
            assert(az:getIndexOfPoint(0, 0) == 0)
            assert(az:getIndexOfPoint(10, 0) == 1)

            local mgr = AnimalPopulationManager.getInstance()
            assert(mgr ~= nil)
            mgr:update()

            MigrationGroupDefinitions.Reset()
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(src, Some("bind_animal_migration_test"), t.globals().clone())
            .unwrap();
        let res = t
            .pcall(&cl, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
        reset();
    }
}
