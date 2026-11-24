//! Build 42.22 `LuaManager$GlobalObject` entity-spawning and Kahlua
//! debugger/reflection globals:
//! - Spawning: `createRandomDeadBody`, `createZombie`, `addZombiesInOutfit`,
//!   `spawnHorde`, `addVehicle`, `addVehicleDebug`
//! - Debugger breakpoints: `hasDataBreakpoint`, `hasDataReadBreakpoint`,
//!   `toggleBreakOnChange`, `toggleBreakOnRead`
//! - Class reflection: `getNumClassFields`, `getClassField`,
//!   `getClassFieldVal`, `getNumClassFunctions`, `getClassFunction`,
//!   `getMethodParameterCount`, `getMethodParameter`
//! - Coroutine stack inspection: `getCoroutineTop`, `getCoroutineObjStack`,
//!   `getCoroutineCallframeStack`

use crate::bind_character_sim::{character_value, LuaGameCharacterRef};
use crate::bind_characters::{desc_arg, shared_characters, LuaItem};
use crate::bind_dead_body::dead_body_value;
use crate::bind_java_util::JavaList;
use crate::bind_registry::shared_script_manager;
use crate::bind_world::{square_coords, with_cell_square_mut};
use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, str_arg, JavaEnum};
use pz_characters::character::zombie::ZombieSpawn;
use pz_characters::character::{DefaultSimContext, GameCharacter};
use pz_characters::survivor::SurvivorDesc;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, TableRef, Thread, Value};
use pz_vehicles::lua::vehicle_value;
use pz_vehicles::vehicle::{BaseVehicle, VehicleRef};
use pz_world::save::visual::{CorpseVisual, HumanVisual, IsoDeadBody, MovingBase};
use pz_world::IsoDirections;
use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub const REFLECT_FIELD_CLASS: &str = "java.lang.reflect.Field";
pub const REFLECT_METHOD_CLASS: &str = "java.lang.reflect.Method";
pub const LUA_CALL_FRAME_CLASS: &str = "se.krka.kahlua.vm.LuaCallFrame";

#[derive(Default)]
struct SpawnDebuggerState {
    next_body_id: i16,
    spawned_zombies: Vec<LuaGameCharacterRef>,
    spawned_vehicles: Vec<VehicleRef>,
    write_breakpoints: HashMap<usize, HashSet<String>>,
    read_breakpoints: HashMap<usize, HashSet<String>>,
}

thread_local! {
    static STATE: RefCell<SpawnDebuggerState> = RefCell::new(SpawnDebuggerState::default());
}

pub(crate) fn reset_session_handles() {
    STATE.with(|s| *s.borrow_mut() = SpawnDebuggerState::default());
}

/// Drain zombies created via `createZombie` / `addZombiesInOutfit` / `spawnHorde`.
pub fn take_spawned_zombies() -> Vec<LuaGameCharacterRef> {
    STATE.with(|s| std::mem::take(&mut s.borrow_mut().spawned_zombies))
}

/// Drain vehicles created via `addVehicle` / `addVehicleDebug`.
pub fn take_spawned_vehicles() -> Vec<VehicleRef> {
    STATE.with(|s| std::mem::take(&mut s.borrow_mut().spawned_vehicles))
}

struct ReflectField {
    name: String,
    owner: String,
}

impl LuaObject for ReflectField {
    fn class_name(&self) -> &str {
        REFLECT_FIELD_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct ReflectMethod {
    name: String,
    owner: String,
}

impl LuaObject for ReflectMethod {
    fn class_name(&self) -> &str {
        REFLECT_METHOD_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaCallFrameObj {
    filename: String,
    line: i32,
    description: String,
}

impl LuaObject for LuaCallFrameObj {
    fn class_name(&self) -> &str {
        LUA_CALL_FRAME_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn dir_from_value(v: &Value) -> Option<IsoDirections> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == crate::bind_iso_enums::ISO_DIRECTIONS)
            .and_then(|e| IsoDirections::from_name(e.name)),
        Value::Str(s) => IsoDirections::from_name(s),
        Value::Num(n) => IsoDirections::ALL.get((*n as usize) & 7).copied(),
        _ => None,
    }
}

fn spawn_one_zombie(
    x: f32,
    y: f32,
    z: f32,
    desc: Option<Rc<RefCell<SurvivorDesc<LuaItem>>>>,
    palette: i32,
    dir: Option<IsoDirections>,
    female_override: Option<bool>,
) -> LuaResult<LuaGameCharacterRef> {
    let sm = shared_script_manager();
    let chars = shared_characters();
    let ctx = DefaultSimContext;
    let mut create_survivor = || {
        let mut ch = chars.borrow_mut();
        let mut d = SurvivorDesc::<LuaItem>::new(&mut ch, &sm)?;
        if let Some(f) = female_override {
            d.set_female(f);
        }
        Ok(Rc::new(RefCell::new(d)))
    };
    let descriptor = desc.map(|d| (d, palette.max(1)));
    let mut zombie = GameCharacter::new_zombie(
        &sm,
        &ctx,
        ZombieSpawn {
            descriptor,
            create_survivor: &mut create_survivor,
        },
    )
    .map_err(|e| LuaError::runtime(format!("createZombie failed: {e}")))?;
    zombie.set_x(x);
    zombie.set_y(y);
    zombie.set_z(z);
    if let Some(d) = dir {
        zombie.set_forward_iso_direction(d);
    }
    if let Some(f) = female_override {
        zombie.zombie.female = f;
    }
    let z_ref = Rc::new(RefCell::new(zombie));
    STATE.with(|s| s.borrow_mut().spawned_zombies.push(z_ref.clone()));
    Ok(z_ref)
}

fn spawn_vehicle(
    script_name: &str,
    x: f32,
    y: f32,
    z: f32,
    skin_idx: Option<i32>,
) -> VehicleRef {
    let v = BaseVehicle::new();
    if !script_name.trim().is_empty() {
        v.set_script_name(script_name);
        v.set_script(script_name);
    }
    if let Some(skin) = skin_idx {
        v.set_skin_index(skin);
    }
    v.set_position(x, y, z);
    pz_vehicles::manager::add_to_world(&v);
    STATE.with(|s| s.borrow_mut().spawned_vehicles.push(v.clone()));
    v
}

fn breakpoint_key(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => format!("{n}"),
        Value::Str(s) => s.to_string(),
        other => pz_kahlua::exec::java_value_of(other),
    }
}

fn table_of_value(thread: &Thread, v: &Value) -> Option<(TableRef, String)> {
    match v {
        Value::Table(t) => Some((t.clone(), "table".to_string())),
        Value::Object(o) => {
            let cls = o.class_name().to_string();
            let mt = thread.raw_metatable(v).ok().flatten()?;
            let idx = mt.get_str("__index");
            match idx {
                Value::Table(t) => Some((t, cls)),
                _ => Some((mt, cls)),
            }
        }
        _ => None,
    }
}

fn is_callable(v: &Value) -> bool {
    matches!(v, Value::Closure(_) | Value::Native(_))
}

fn collect_members(thread: &Thread, v: &Value, want_functions: bool) -> (String, Vec<String>) {
    let Some((tbl, owner)) = table_of_value(thread, v) else {
        return (String::new(), Vec::new());
    };
    let entries = tbl.try_entries().unwrap_or_default();
    let mut names = Vec::new();
    for (k, val) in entries {
        if let Value::Str(s) = k {
            if s.starts_with("__") {
                continue;
            }
            if is_callable(&val) == want_functions {
                names.push(s.to_string());
            }
        }
    }
    (owner, names)
}

pub fn register(thread: &mut Thread) {
    let field_cls = expose_class(thread, REFLECT_FIELD_CLASS, Some("java.lang.Object"));
    field_cls.method("getName", |c| {
        let name = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<ReflectField>()
                .map(|f| f.name.clone()),
            _ => None,
        };
        Ok(c.push(name.map_or(Value::Nil, |s| Value::str(&s))))
    });
    field_cls.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<ReflectField>()
                .map(|f| format!("{}.{}", f.owner, f.name)),
            _ => None,
        };
        Ok(c.push(s.as_deref().map_or(Value::Nil, Value::str)))
    });

    let method_cls = expose_class(thread, REFLECT_METHOD_CLASS, Some("java.lang.Object"));
    method_cls.method("getName", |c| {
        let name = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<ReflectMethod>()
                .map(|m| m.name.clone()),
            _ => None,
        };
        Ok(c.push(name.as_deref().map_or(Value::Nil, Value::str)))
    });
    method_cls.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<ReflectMethod>()
                .map(|m| format!("{}.{}()", m.owner, m.name)),
            _ => None,
        };
        Ok(c.push(s.as_deref().map_or(Value::Nil, Value::str)))
    });

    let frame_cls = expose_class(thread, LUA_CALL_FRAME_CLASS, Some("java.lang.Object"));
    frame_cls.method("getFilename", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaCallFrameObj>()
                .map(|f| f.filename.clone()),
            _ => None,
        };
        Ok(c.push(s.as_deref().map_or(Value::Nil, Value::str)))
    });
    frame_cls.method("getLine", |c| {
        let line = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaCallFrameObj>()
                .map(|f| f.line)
                .unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(line as f64))
    });
    frame_cls.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaCallFrameObj>()
                .map(|f| f.description.clone()),
            _ => None,
        };
        Ok(c.push(s.as_deref().map_or(Value::Nil, Value::str)))
    });

    // ---- Spawning globals --------------------------------------------------

    global(thread, "createRandomDeadBody", |c| {
        let (sx, sy, sz) = match square_coords(&c.arg(0)) {
            Some(pos) => pos,
            None if matches!(c.arg(0), Value::Num(_)) => {
                (int_arg(c, 0), int_arg(c, 1), int_arg(c, 2))
            }
            None => return Ok(c.push(Value::Nil)),
        };
        let id = STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.next_body_id = st.next_body_id.wrapping_add(1).max(1);
            st.next_body_id
        });
        let body = IsoDeadBody {
            base: MovingBase {
                offset_x: 0.0,
                offset_y: 0.0,
                x: sx as f32 + 0.5,
                y: sy as f32 + 0.5,
                z: sz as f32,
                dir: 0,
                mod_data: None,
            },
            female: (id & 1) == 0,
            was_zombie: true,
            animal: None,
            object_id: (id, 0),
            persistent_outfit_id: 0,
            desc: None,
            visual: CorpseVisual::Human(Box::new(HumanVisual::new(0))),
            container: None,
            worn_items: Vec::new(),
            attached_items: Vec::new(),
            death_time: 0.0,
            reanimate_time: -1.0,
            fall_on_front: false,
            killed_by_fall: false,
            was_skeleton: false,
            angle: 0.0,
            zombie_rot_stage_at_death: 0,
            animal_rot_stage_at_death: 0,
            rotten_texture: String::new(),
            skel_inv_icon: String::new(),
            crawling: false,
            fake_dead: false,
            ragdoll: None,
            burn_timer: 0.0,
            runtime: None,
        };
        let body_ref = Rc::new(RefCell::new(body));
        with_cell_square_mut(sx, sy, sz, |sq| {
            sq.static_moving.push(body_ref.clone());
        });
        Ok(c.push(dead_body_value(&body_ref)))
    });

    global(thread, "createZombie", |c| {
        let x = num_arg(c, 0) as f32;
        let y = num_arg(c, 1) as f32;
        let z = num_arg(c, 2) as f32;
        let desc = if c.arg(3).is_nil() {
            None
        } else {
            Some(desc_arg(c, 3)?)
        };
        let palette = int_arg(c, 4);
        let dir = dir_from_value(&c.arg(5));
        let z_ref = spawn_one_zombie(x, y, z, desc, palette, dir, None)?;
        Ok(c.push(character_value(&z_ref)))
    });

    global(thread, "addZombiesInOutfit", |c| {
        let x = num_arg(c, 0) as f32 + 0.5;
        let y = num_arg(c, 1) as f32 + 0.5;
        let z = num_arg(c, 2) as f32;
        let total = int_arg(c, 3).max(0) as usize;
        let _outfit = str_arg(c, 4);
        let female_chance = match c.arg(5) {
            Value::Num(n) => Some((n as i32).clamp(0, 100)),
            _ => None,
        };
        let is_fake_dead = c.nargs() > 8 && bool_arg(c, 8);
        let health_opt = if c.nargs() > 12 {
            match c.arg(12) {
                Value::Num(n) if n > 0.0 => Some(n as f32),
                _ => None,
            }
        } else {
            None
        };

        let list = JavaList::new(Vec::with_capacity(total));
        for i in 0..total {
            let female = female_chance.map(|pct| ((i * 37) % 100) < pct as usize);
            let z_ref = spawn_one_zombie(x, y, z, None, 1, None, female)?;
            {
                let mut zb = z_ref.borrow_mut();
                if is_fake_dead {
                    zb.set_fake_dead(&DefaultSimContext, true);
                }
                if let Some(h) = health_opt {
                    zb.health = h;
                }
            }
            list.push(character_value(&z_ref))?;
        }
        Ok(c.push(Value::Object(Rc::new(list))))
    });

    global(thread, "spawnHorde", |c| {
        let x1 = num_arg(c, 0) as f32;
        let y1 = num_arg(c, 1) as f32;
        let x2 = num_arg(c, 2) as f32;
        let y2 = num_arg(c, 3) as f32;
        let z = num_arg(c, 4) as f32;
        let count = int_arg(c, 5).max(0) as usize;
        let (min_x, max_x) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
        let (min_y, max_y) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
        let span_x = (max_x - min_x).max(0.0);
        let span_y = (max_y - min_y).max(0.0);
        for i in 0..count {
            let t = if count <= 1 {
                0.5
            } else {
                i as f32 / (count - 1) as f32
            };
            let px = min_x + span_x * t;
            let py = min_y + span_y * t;
            let _ = spawn_one_zombie(px, py, z, None, 1, None, None)?;
        }
        Ok(0)
    });

    global(thread, "addVehicle", |c| {
        let Some(script) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let x = int_arg(c, 1) as f32 + 0.5;
        let y = int_arg(c, 2) as f32 + 0.5;
        let z = int_arg(c, 3) as f32;
        let v = spawn_vehicle(&script, x, y, z, None);
        Ok(c.push(vehicle_value(&v)))
    });

    global(thread, "addVehicleDebug", |c| {
        let Some(script) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let _dir = dir_from_value(&c.arg(1));
        let skin = match c.arg(2) {
            Value::Num(n) => Some(n as i32),
            _ => None,
        };
        let (sx, sy, sz) = square_coords(&c.arg(3)).unwrap_or((0, 0, 0));
        let v = spawn_vehicle(&script, sx as f32 + 0.5, sy as f32 + 0.5, sz as f32, skin);
        Ok(c.push(vehicle_value(&v)))
    });

    // ---- Debugger breakpoints ----------------------------------------------

    global(thread, "hasDataBreakpoint", |c| {
        let has = match c.arg(0) {
            Value::Table(t) => {
                let k = breakpoint_key(&c.arg(1));
                STATE.with(|s| {
                    s.borrow()
                        .write_breakpoints
                        .get(&t.ptr_id())
                        .is_some_and(|set| set.contains(&k))
                })
            }
            _ => false,
        };
        Ok(c.push(has))
    });

    global(thread, "hasDataReadBreakpoint", |c| {
        let has = match c.arg(0) {
            Value::Table(t) => {
                let k = breakpoint_key(&c.arg(1));
                STATE.with(|s| {
                    s.borrow()
                        .read_breakpoints
                        .get(&t.ptr_id())
                        .is_some_and(|set| set.contains(&k))
                })
            }
            _ => false,
        };
        Ok(c.push(has))
    });

    global(thread, "toggleBreakOnChange", |c| {
        if let Value::Table(t) = c.arg(0) {
            let k = breakpoint_key(&c.arg(1));
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                let entry = st.write_breakpoints.entry(t.ptr_id()).or_default();
                if !entry.remove(&k) {
                    entry.insert(k);
                }
            });
        }
        Ok(0)
    });

    global(thread, "toggleBreakOnRead", |c| {
        if let Value::Table(t) = c.arg(0) {
            let k = breakpoint_key(&c.arg(1));
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                let entry = st.read_breakpoints.entry(t.ptr_id()).or_default();
                if !entry.remove(&k) {
                    entry.insert(k);
                }
            });
        }
        Ok(0)
    });

    // ---- Class reflection globals ------------------------------------------

    global(thread, "getNumClassFields", |c| {
        let (_, fields) = collect_members(c.thread, &c.arg(0), false);
        Ok(c.push(fields.len() as f64))
    });

    global(thread, "getClassField", |c| {
        let idx = int_arg(c, 1);
        if idx < 0 {
            return Ok(c.push(Value::Nil));
        }
        let (owner, fields) = collect_members(c.thread, &c.arg(0), false);
        match fields.into_iter().nth(idx as usize) {
            Some(name) => Ok(c.push(Value::Object(Rc::new(ReflectField { name, owner })))),
            None => Ok(c.push(Value::Nil)),
        }
    });

    global(thread, "getClassFieldVal", |c| {
        let obj = c.arg(0);
        let field_name = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<ReflectField>()
                .map(|f| f.name.clone()),
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        };
        let Some(name) = field_name else {
            return Ok(c.push(Value::Nil));
        };
        let val = c.thread.table_get(&obj, &Value::str(&name))?;
        Ok(c.push(val))
    });

    global(thread, "getNumClassFunctions", |c| {
        let (_, funcs) = collect_members(c.thread, &c.arg(0), true);
        Ok(c.push(funcs.len() as f64))
    });

    global(thread, "getClassFunction", |c| {
        let idx = int_arg(c, 1);
        if idx < 0 {
            return Ok(c.push(Value::Nil));
        }
        let (owner, funcs) = collect_members(c.thread, &c.arg(0), true);
        match funcs.into_iter().nth(idx as usize) {
            Some(name) => Ok(c.push(Value::Object(Rc::new(ReflectMethod { name, owner })))),
            None => Ok(c.push(Value::Nil)),
        }
    });

    global(thread, "getMethodParameterCount", |c| {
        let count = match c.arg(0) {
            Value::Closure(cl) => cl.prototype.num_params as f64,
            _ => 0.0,
        };
        Ok(c.push(count))
    });

    global(thread, "getMethodParameter", |c| {
        let idx = int_arg(c, 1);
        if idx < 0 {
            return Ok(c.push(Value::Nil));
        }
        let param = match c.arg(0) {
            Value::Closure(cl) => cl
                .prototype
                .locvars
                .get(idx as usize)
                .map(|s| Value::str(s))
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        Ok(c.push(param))
    });

    // ---- Coroutine stack inspection globals --------------------------------

    global(thread, "getCoroutineTop", |c| {
        let top = match c.arg(0) {
            Value::Coroutine(co) => co
                .borrow()
                .stack_trace
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count() as f64,
            _ => 0.0,
        };
        Ok(c.push(top))
    });

    global(thread, "getCoroutineObjStack", |c| {
        let idx = int_arg(c, 1);
        if idx < 0 {
            return Ok(c.push(Value::Nil));
        }
        let val = match c.arg(0) {
            Value::Coroutine(co) => co
                .borrow()
                .stack_trace
                .lines()
                .filter(|l| !l.trim().is_empty())
                .nth(idx as usize)
                .map(Value::str)
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        Ok(c.push(val))
    });

    global(thread, "getCoroutineCallframeStack", |c| {
        let idx = int_arg(c, 1);
        if idx < 0 {
            return Ok(c.push(Value::Nil));
        }
        let frame = match c.arg(0) {
            Value::Coroutine(co) => co
                .borrow()
                .stack_trace
                .lines()
                .filter(|l| !l.trim().is_empty())
                .nth(idx as usize)
                .map(|line| {
                    Value::Object(Rc::new(LuaCallFrameObj {
                        filename: String::new(),
                        line: 0,
                        description: line.to_string(),
                    }))
                })
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        Ok(c.push(frame))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib;

    #[test]
    fn spawn_and_debugger_globals_lua() {
        reset_session_handles();
        pz_vehicles::manager::reset();

        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_dead_body::register(&mut t);
        crate::bind_characters::register_shared(&mut t);
        crate::bind_character_sim::register_shared(&mut t);
        crate::bind_vehicles::register(&mut t);
        register(&mut t);

        let src = r#"
            local function assert(v, msg)
                if not v then error(msg or "assertion failed") end
            end

            -- Dead body spawning (coordinate overload without a live cell)
            local body = createRandomDeadBody(10, 20, 0)
            assert(body ~= nil)
            assert(body:isZombie() == true)
            assert(body:getX() == 10.5 and body:getY() == 20.5 and body:getZ() == 0)

            -- Zombie spawning
            local z = createZombie(12.25, 34.5, 0, nil, 1, nil)
            assert(z ~= nil)
            assert(z:getX() == 12.25 and z:getY() == 34.5)

            local list = addZombiesInOutfit(15, 25, 0, 3, "Generic", 50)
            assert(list:size() == 3)
            assert(list:get(0):getX() == 15.5)

            spawnHorde(10, 10, 14, 14, 0, 2)

            -- Vehicle spawning
            local v1 = addVehicle("Base.PickUpTruck", 50, 60, 0)
            assert(v1 ~= nil)
            assert(v1:getScriptName() == "Base.PickUpTruck")
            assert(v1:getX() == 50.5 and v1:getY() == 60.5)

            local v2 = addVehicleDebug("Base.SportsCar", nil, 2, nil)
            assert(v2 ~= nil)
            assert(v2:getScriptName() == "Base.SportsCar")

            -- Data breakpoints
            local tbl = { hp = 100, name = "test", greet = function(a, b) return a end }
            assert(hasDataBreakpoint(tbl, "hp") == false)
            toggleBreakOnChange(tbl, "hp")
            assert(hasDataBreakpoint(tbl, "hp") == true)
            toggleBreakOnChange(tbl, "hp")
            assert(hasDataBreakpoint(tbl, "hp") == false)

            assert(hasDataReadBreakpoint(tbl, "name") == false)
            toggleBreakOnRead(tbl, "name")
            assert(hasDataReadBreakpoint(tbl, "name") == true)

            -- Reflection
            assert(getNumClassFields(tbl) == 2)
            local f0 = getClassField(tbl, 0)
            assert(f0 ~= nil)
            assert(getClassFieldVal(tbl, f0) ~= nil)
            assert(getNumClassFunctions(tbl) == 1)
            local m0 = getClassFunction(tbl, 0)
            assert(m0:getName() == "greet")
            assert(getMethodParameterCount(tbl.greet) == 2)

            -- Coroutine helpers
            local co = coroutine.create(function() coroutine.yield(1) end)
            assert(getCoroutineTop(co) == 0)
            assert(getCoroutineObjStack(co, 0) == nil)
            assert(getCoroutineCallframeStack(co, 0) == nil)
        "#;
        let f = stdlib::load_string(src, Some("spawn_debugger"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();

        let zombies = take_spawned_zombies();
        assert_eq!(zombies.len(), 6); // 1 createZombie + 3 addZombiesInOutfit + 2 spawnHorde
        let vehicles = take_spawned_vehicles();
        assert_eq!(vehicles.len(), 2);
    }
}
