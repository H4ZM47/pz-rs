//! Build 42.22 Lua bindings for:
//! - `zombie.iso.objects.IsoFireManager`
//! - `zombie.iso.objects.GridSquareEdgeFacingDirection`
//! - `zombie.core.stash.StashSystem` & `zombie.core.stash.Stash`

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, TableRef, Thread, Value};

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;

const ISO_FIRE_MANAGER_CLASS: &str = "zombie.iso.objects.IsoFireManager";
const EDGE_FACING_CLASS: &str = "zombie.iso.objects.GridSquareEdgeFacingDirection";
const STASH_SYSTEM_CLASS: &str = "zombie.core.stash.StashSystem";
const STASH_CLASS: &str = "zombie.core.stash.Stash";

#[derive(Default)]
struct FireManagerState {
    fire_stack: Vec<Value>,
    characters_on_fire: Vec<Value>,
    max_fire_objects: i32,
}

#[derive(Clone, Debug, Default)]
struct StashData {
    name: String,
    item: String,
    stash_type: String,
    building_x: i32,
    building_y: i32,
    spawn_table: String,
    zombies: i32,
    barricades: i32,
}

#[derive(Default)]
struct StashSystemState {
    all_stashes: Vec<Rc<RefCell<StashData>>>,
    possible_stashes: Vec<Rc<RefCell<StashData>>>,
    already_read_map: Vec<String>,
    buildings_to_do: HashMap<(i32, i32), String>,
    visited_buildings: HashSet<(i32, i32)>,
}

thread_local! {
    static FIRE_MGR: RefCell<FireManagerState> = RefCell::new(FireManagerState {
        fire_stack: Vec::new(),
        characters_on_fire: Vec::new(),
        max_fire_objects: 75,
    });
    static STASH_SYS: RefCell<StashSystemState> = RefCell::new(StashSystemState::default());
}

/// Reset session state on world teardown.
pub fn reset() {
    FIRE_MGR.with(|f| {
        let mut s = f.borrow_mut();
        s.fire_stack.clear();
        s.characters_on_fire.clear();
    });
    STASH_SYS.with(|s| *s.borrow_mut() = StashSystemState::default());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EdgeFacingVal(&'static str, usize);

impl LuaObject for EdgeFacingVal {
    fn class_name(&self) -> &'static str {
        EDGE_FACING_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct StashObj(Rc<RefCell<StashData>>);

impl LuaObject for StashObj {
    fn class_name(&self) -> &'static str {
        STASH_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn call_zero(thread: &mut Thread, obj: &Value, method: &str) -> Option<Value> {
    let f = thread.table_get(obj, &Value::str(method)).ok()?;
    if f.is_nil() {
        return None;
    }
    thread
        .pcall(&f, std::slice::from_ref(obj))
        .ok()?
        .into_iter()
        .next()
}

fn building_xy(thread: &mut Thread, val: &Value) -> Option<(i32, i32)> {
    if val.is_nil() {
        return None;
    }
    let x = call_zero(thread, val, "getX")?.as_num()? as i32;
    let y = call_zero(thread, val, "getY")?.as_num()? as i32;
    Some((x, y))
}

fn with_stash<R>(c: &NativeCall<'_>, f: impl FnOnce(&StashData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let s = o.as_any().downcast_ref::<StashObj>()?;
    let res = Some(f(&s.0.borrow()));
    res
}

/// Register `IsoFireManager`, `GridSquareEdgeFacingDirection`, `StashSystem`,
/// and `Stash`.
pub fn register(thread: &mut Thread) {
    // zombie.iso.objects.IsoFireManager
    let fm = expose_class(thread, ISO_FIRE_MANAGER_CLASS, Some("java.lang.Object"));
    fm.field("FIRE_ANIM_DELAY", Value::Num(0.2));
    fm.field("maxFireObjects", Value::Num(75.0));
    fm.field("fireAlpha", Value::Num(1.0));
    fm.field("smokeAlpha", Value::Num(0.3));
    fm.static_fn("Add", |c| {
        let v = c.arg(0);
        if !v.is_nil() {
            FIRE_MGR.with(|m| {
                let mut s = m.borrow_mut();
                if !s.fire_stack.iter().any(|x| x.raw_equals(&v)) {
                    s.fire_stack.push(v);
                }
            });
        }
        Ok(0)
    });
    fm.static_fn("Remove", |c| {
        let v = c.arg(0);
        FIRE_MGR.with(|m| m.borrow_mut().fire_stack.retain(|x| !x.raw_equals(&v)));
        Ok(0)
    });
    fm.static_fn("AddBurningCharacter", |c| {
        let v = c.arg(0);
        if !v.is_nil() {
            FIRE_MGR.with(|m| {
                let mut s = m.borrow_mut();
                if !s.characters_on_fire.iter().any(|x| x.raw_equals(&v)) {
                    s.characters_on_fire.push(v);
                }
            });
        }
        Ok(0)
    });
    fm.static_fn("addCharacterOnFire", |c| {
        let v = c.arg(0);
        if !v.is_nil() {
            FIRE_MGR.with(|m| {
                let mut s = m.borrow_mut();
                if !s.characters_on_fire.iter().any(|x| x.raw_equals(&v)) {
                    s.characters_on_fire.push(v);
                }
            });
        }
        Ok(0)
    });
    fm.static_fn("RemoveBurningCharacter", |c| {
        let v = c.arg(0);
        FIRE_MGR.with(|m| {
            m.borrow_mut()
                .characters_on_fire
                .retain(|x| !x.raw_equals(&v));
        });
        Ok(0)
    });
    fm.static_fn("deleteCharacterOnFire", |c| {
        let v = c.arg(0);
        FIRE_MGR.with(|m| {
            m.borrow_mut()
                .characters_on_fire
                .retain(|x| !x.raw_equals(&v));
        });
        Ok(0)
    });
    fm.static_fn("StartFire", |_| Ok(0));
    fm.static_fn("StartSmoke", |_| Ok(0));
    fm.static_fn("explode", |_| Ok(0));
    fm.static_fn("MolotovSmash", |_| Ok(0));
    fm.static_fn("Fire_LightCalc", |_| Ok(0));
    fm.static_fn("LightTileWithFire", |_| Ok(0));
    fm.static_fn("RemoveAllOn", |_| Ok(0));
    fm.static_fn("Update", |_| Ok(0));
    fm.static_fn("updateSound", |_| Ok(0));
    fm.static_fn("stopSound", |_| Ok(0));
    fm.static_fn("Reset", |_| {
        FIRE_MGR.with(|m| {
            let mut s = m.borrow_mut();
            s.fire_stack.clear();
            s.characters_on_fire.clear();
            let _ = s.max_fire_objects;
        });
        Ok(0)
    });

    // zombie.iso.objects.GridSquareEdgeFacingDirection
    let edges = [
        EdgeFacingVal("NORTH_SOUTH", 0),
        EdgeFacingVal("EAST_WEST", 1),
    ];
    let ef = expose_class(thread, EDGE_FACING_CLASS, Some("java.lang.Object"));
    for e in edges {
        ef.field(e.0, Value::Object(Rc::new(e)));
    }
    ef.method("name", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<EdgeFacingVal>()
                .map(|v| v.0)
                .unwrap_or(""),
            _ => "",
        };
        Ok(c.push(Value::str(s)))
    });
    ef.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<EdgeFacingVal>()
                .map(|v| v.0)
                .unwrap_or(""),
            _ => "",
        };
        Ok(c.push(Value::str(s)))
    });
    ef.method("ordinal", |c| {
        let ord = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<EdgeFacingVal>()
                .map(|v| v.1)
                .unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(ord as f64))
    });
    ef.static_fn("values", move |c| {
        let tbl = TableRef::new();
        for (idx, e) in edges.iter().enumerate() {
            tbl.rawset(Value::Num((idx + 1) as f64), Value::Object(Rc::new(*e)));
        }
        Ok(c.push(Value::Table(tbl)))
    });
    ef.static_fn("valueOf", move |c| {
        let arg0 = c.arg(0);
        let found = arg0.as_str().and_then(|name| {
            edges
                .iter()
                .find(|v| v.0 == name)
                .map(|v| Value::Object(Rc::new(*v)))
        });
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });

    // zombie.core.stash.Stash
    let st = expose_class(thread, STASH_CLASS, Some("java.lang.Object"));
    st.ctor(|c| {
        let name = c.arg(0).as_str().unwrap_or("").to_string();
        Ok(c.push(Value::Object(Rc::new(StashObj(Rc::new(RefCell::new(
            StashData {
                name,
                ..Default::default()
            },
        )))))))
    });
    st.method("getName", |c| {
        let s = with_stash(c, |st| st.name.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    st.method("getItem", |c| {
        let s = with_stash(c, |st| st.item.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    st.method("getBuildingX", |c| {
        Ok(c.push(with_stash(c, |st| st.building_x).unwrap_or(0) as f64))
    });
    st.method("getBuildingY", |c| {
        Ok(c.push(with_stash(c, |st| st.building_y).unwrap_or(0) as f64))
    });

    // zombie.core.stash.StashSystem
    let ss = expose_class(thread, STASH_SYSTEM_CLASS, Some("java.lang.Object"));
    ss.static_fn("init", |c| {
        load_stash_descriptions_from_lua(c.thread);
        Ok(0)
    });
    ss.static_fn("initAllStashes", |c| {
        load_stash_descriptions_from_lua(c.thread);
        Ok(0)
    });
    ss.static_fn("reinit", |c| {
        STASH_SYS.with(|s| *s.borrow_mut() = StashSystemState::default());
        load_stash_descriptions_from_lua(c.thread);
        Ok(0)
    });
    ss.static_fn("Reset", |_| {
        STASH_SYS.with(|s| *s.borrow_mut() = StashSystemState::default());
        Ok(0)
    });
    ss.static_fn("getAllStashes", |c| {
        let items = STASH_SYS.with(|s| {
            s.borrow()
                .all_stashes
                .iter()
                .map(|rc| Value::Object(Rc::new(StashObj(rc.clone()))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    ss.static_fn("getPossibleStashes", |c| {
        let items = STASH_SYS.with(|s| {
            s.borrow()
                .possible_stashes
                .iter()
                .map(|rc| Value::Object(Rc::new(StashObj(rc.clone()))))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    ss.static_fn("getAlreadyReadMap", |c| {
        let items = STASH_SYS.with(|s| {
            s.borrow()
                .already_read_map
                .iter()
                .map(|name| Value::str(name))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    ss.static_fn("getStash", |c| {
        let arg0 = c.arg(0);
        let found = arg0.as_str().and_then(|name| {
            STASH_SYS.with(|s| {
                s.borrow()
                    .all_stashes
                    .iter()
                    .find(|st| st.borrow().name == name)
                    .cloned()
                    .map(|rc| Value::Object(Rc::new(StashObj(rc))))
            })
        });
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });
    ss.static_fn("prepareBuildingStash", |c| {
        let arg0 = c.arg(0);
        if let Some(name) = arg0.as_str() {
            STASH_SYS.with(|s| {
                let mut state = s.borrow_mut();
                if let Some(st) = state.all_stashes.iter().find(|st| st.borrow().name == name) {
                    let b = st.borrow();
                    let (bx, by) = (b.building_x, b.building_y);
                    drop(b);
                    state.buildings_to_do.insert((bx, by), name.to_string());
                }
            });
        }
        Ok(0)
    });
    ss.static_fn("isStashBuilding", |c| {
        let arg0 = c.arg(0);
        let ok = building_xy(c.thread, &arg0)
            .is_some_and(|xy| STASH_SYS.with(|s| s.borrow().buildings_to_do.contains_key(&xy)));
        Ok(c.push(ok))
    });
    ss.static_fn("visitedBuilding", |c| {
        let arg0 = c.arg(0);
        if let Some(xy) = building_xy(c.thread, &arg0) {
            STASH_SYS.with(|s| {
                s.borrow_mut().visited_buildings.insert(xy);
            });
        }
        Ok(0)
    });
    ss.static_fn("doBuildingStash", |c| {
        let arg0 = c.arg(0);
        if let Some(xy) = building_xy(c.thread, &arg0) {
            STASH_SYS.with(|s| {
                s.borrow_mut().buildings_to_do.remove(&xy);
            });
        }
        Ok(0)
    });
    ss.static_fn("checkStashItem", |_| Ok(0));
    ss.static_fn("doStashItem", |_| Ok(0));
}

fn load_stash_descriptions_from_lua(thread: &Thread) {
    let Value::Table(desc_tbl) = thread.globals().get_str("StashDescriptions") else {
        return;
    };
    let mut loaded = Vec::new();
    for (_, val) in desc_tbl.try_entries().unwrap_or_default() {
        if let Value::Table(entry) = val {
            let name = entry.get_str("name").as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let data = StashData {
                name,
                item: entry.get_str("item").as_str().unwrap_or("").to_string(),
                stash_type: entry.get_str("type").as_str().unwrap_or("").to_string(),
                building_x: entry.get_str("buildingX").as_num().unwrap_or(0.0) as i32,
                building_y: entry.get_str("buildingY").as_num().unwrap_or(0.0) as i32,
                spawn_table: entry
                    .get_str("spawnTable")
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                zombies: entry.get_str("zombies").as_num().unwrap_or(0.0) as i32,
                barricades: entry.get_str("barricades").as_num().unwrap_or(0.0) as i32,
            };
            loaded.push(Rc::new(RefCell::new(data)));
        }
    }
    STASH_SYS.with(|s| {
        let mut state = s.borrow_mut();
        state.all_stashes = loaded.clone();
        state.possible_stashes = loaded;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_manager_edge_facing_and_stash_system_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            assert(IsoFireManager.maxFireObjects == 75)
            IsoFireManager.Reset()
            IsoFireManager.StartFire(nil, nil, false, 10)
            IsoFireManager.StartSmoke(nil, nil, false, 5)

            assert(GridSquareEdgeFacingDirection.NORTH_SOUTH:name() == "NORTH_SOUTH")
            assert(GridSquareEdgeFacingDirection.valueOf("EAST_WEST"):ordinal() == 1)
            assert(#GridSquareEdgeFacingDirection.values() == 2)

            StashDescriptions = {
                {
                    name = "TestStash1",
                    type = "Map",
                    item = "Base.LouisvilleMap1",
                    buildingX = 10500,
                    buildingY = 9800,
                    spawnTable = "GunCache1",
                }
            }
            StashSystem.init()
            assert(StashSystem.getAllStashes():size() == 1)
            local st = StashSystem.getStash("TestStash1")
            assert(st ~= nil and st:getBuildingX() == 10500 and st:getBuildingY() == 9800)
            StashSystem.prepareBuildingStash("TestStash1")
            local bdef = { getX = function() return 10500 end, getY = function() return 9800 end }
            assert(StashSystem.isStashBuilding(bdef) == true)
            StashSystem.doBuildingStash(bdef)
            assert(StashSystem.isStashBuilding(bdef) == false)
            StashSystem.Reset()
            assert(StashSystem.getAllStashes():size() == 0)
            "#,
            Some("fire_stash_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
