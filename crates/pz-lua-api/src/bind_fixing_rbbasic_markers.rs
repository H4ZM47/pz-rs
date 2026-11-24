//! Build 42.22 Lua bindings for:
//! - `zombie.inventory.FixingManager`
//! - `zombie.randomizedWorld.randomizedBuilding.RBBasic`
//! - `zombie.iso.IsoMarkers` & `zombie.iso.IsoMarkers$IsoMarker`

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Thread, Value};

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;

const FIXING_MANAGER_CLASS: &str = "zombie.inventory.FixingManager";
const RB_BASIC_CLASS: &str = "zombie.randomizedWorld.randomizedBuilding.RBBasic";
const ISO_MARKERS_CLASS: &str = "zombie.iso.IsoMarkers";
const ISO_MARKER_CLASS: &str = "zombie.iso.IsoMarkers$IsoMarker";

#[derive(Clone, Debug)]
struct IsoMarkerData {
    id: i32,
    x: f32,
    y: f32,
    z: f32,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    rotation: f32,
    circle_size: f32,
    active: bool,
    removed: bool,
    square: Option<Value>,
}

impl Default for IsoMarkerData {
    fn default() -> Self {
        Self {
            id: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
            rotation: 0.0,
            circle_size: 1.0,
            active: true,
            removed: false,
            square: None,
        }
    }
}

#[derive(Default)]
struct IsoMarkersState {
    next_id: i32,
    markers: Vec<Rc<RefCell<IsoMarkerData>>>,
}

#[derive(Default)]
struct RbBasicState {
    unique_rds_spawned: Vec<String>,
    survivor_stories: Vec<Value>,
    survivor_professions: Vec<String>,
}

thread_local! {
    static ISO_MARKERS: RefCell<IsoMarkersState> = RefCell::new(IsoMarkersState::default());
    static RB_BASIC: RefCell<RbBasicState> = RefCell::new(RbBasicState::default());
}

/// `RBBasic.getUniqueRDSSpawned().clear()` called from `IsoWorld::init`.
pub fn clear_unique_rds_spawned() {
    RB_BASIC.with(|s| s.borrow_mut().unique_rds_spawned.clear());
}

/// Reset session state on world teardown.
pub fn reset() {
    ISO_MARKERS.with(|s| *s.borrow_mut() = IsoMarkersState::default());
    RB_BASIC.with(|s| *s.borrow_mut() = RbBasicState::default());
}

#[derive(Debug)]
struct RbBasicObj;

impl LuaObject for RbBasicObj {
    fn class_name(&self) -> &'static str {
        RB_BASIC_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct IsoMarkersHandle;

impl LuaObject for IsoMarkersHandle {
    fn class_name(&self) -> &'static str {
        ISO_MARKERS_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct IsoMarkerObj(Rc<RefCell<IsoMarkerData>>);

impl LuaObject for IsoMarkerObj {
    fn class_name(&self) -> &'static str {
        ISO_MARKER_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn f32_arg(c: &NativeCall<'_>, idx: usize) -> f32 {
    c.arg(idx).as_num().unwrap_or(0.0) as f32
}

fn i32_arg(c: &NativeCall<'_>, idx: usize) -> i32 {
    c.arg(idx).as_num().unwrap_or(0.0) as i32
}

fn call_method(thread: &mut Thread, obj: &Value, name: &str, extra: &[Value]) -> Option<Value> {
    if obj.is_nil() {
        return None;
    }
    let f = thread.table_get(obj, &Value::str(name)).ok()?;
    if f.is_nil() {
        return None;
    }
    let mut args = Vec::with_capacity(1 + extra.len());
    args.push(obj.clone());
    args.extend_from_slice(extra);
    thread.pcall(&f, &args).ok()?.into_iter().next()
}

fn with_marker<R>(c: &NativeCall<'_>, f: impl FnOnce(&IsoMarkerData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let m = o.as_any().downcast_ref::<IsoMarkerObj>()?;
    let res = Some(f(&m.0.borrow()));
    res
}

fn with_marker_mut<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut IsoMarkerData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let m = o.as_any().downcast_ref::<IsoMarkerObj>()?;
    let res = Some(f(&mut m.0.borrow_mut()));
    res
}

fn compute_cond_repaired(
    thread: &mut Thread,
    item: &Value,
    chr: &Value,
    fixing: &Value,
    fixer: &Value,
) -> f64 {
    let cond_max = call_method(thread, item, "getConditionMax", &[])
        .and_then(|v| v.as_num())
        .unwrap_or(10.0);
    let cond = call_method(thread, item, "getCondition", &[])
        .and_then(|v| v.as_num())
        .unwrap_or(0.0);
    let missing = (cond_max - cond).max(0.0);
    let factor = call_method(thread, fixing, "getConditionModifier", &[])
        .and_then(|v| v.as_num())
        .unwrap_or(1.0);
    let base_rep = call_method(thread, fixer, "getCondRepaired", &[])
        .and_then(|v| v.as_num())
        .unwrap_or(50.0);
    let _ = chr;
    ((missing * (base_rep / 100.0) * factor).round()).clamp(0.0, missing)
}

fn compute_fail_chance(
    thread: &mut Thread,
    item: &Value,
    _chr: &Value,
    _fixing: &Value,
    _fixer: &Value,
) -> f64 {
    let have_repaired = call_method(thread, item, "getHaveBeenRepaired", &[])
        .and_then(|v| v.as_num())
        .unwrap_or(1.0);
    ((have_repaired - 1.0) * 10.0).clamp(0.0, 100.0)
}

/// Register `FixingManager`, `RBBasic`, `IsoMarkers`, and `IsoMarkers$IsoMarker`.
pub fn register(thread: &mut Thread) {
    // zombie.inventory.FixingManager
    let fm = expose_class(thread, FIXING_MANAGER_CLASS, Some("java.lang.Object"));
    fm.static_fn("getFixes", |c| {
        let item = c.arg(0);
        let item_type = call_method(c.thread, &item, "getType", &[])
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let mut fixes = Vec::new();
        if !item_type.is_empty() {
            let sm = c.thread.globals().get_str("getScriptManager");
            if !sm.is_nil() {
                if let Ok(res) = c.thread.pcall(&sm, &[]) {
                    if let Some(sm_obj) = res.into_iter().next() {
                        if let Some(Value::Object(list_obj)) =
                            call_method(c.thread, &sm_obj, "getAllFixing", &[])
                        {
                            if let Some(list) = list_obj.as_any().downcast_ref::<JavaList>() {
                                let snapshot = list.items.borrow().clone();
                                for fx in snapshot {
                                    if let Some(Value::Object(req_obj)) =
                                        call_method(c.thread, &fx, "getRequiredItem", &[])
                                    {
                                        if let Some(req_list) =
                                            req_obj.as_any().downcast_ref::<JavaList>()
                                        {
                                            let matches = req_list
                                                .items
                                                .borrow()
                                                .iter()
                                                .any(|v| v.as_str() == Some(item_type.as_str()));
                                            if matches {
                                                fixes.push(fx);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(c.push(Value::Object(Rc::new(JavaList::new(fixes)))))
    });
    fm.static_fn("getCondRepaired", |c| {
        let (item, chr, fixing, fixer) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
        let v = compute_cond_repaired(c.thread, &item, &chr, &fixing, &fixer);
        Ok(c.push(v))
    });
    fm.static_fn("getChanceOfFail", |c| {
        let (item, chr, fixing, fixer) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
        let v = compute_fail_chance(c.thread, &item, &chr, &fixing, &fixer);
        Ok(c.push(v))
    });
    fm.static_fn("useFixer", |_, | Ok(0));
    fm.static_fn("fixItem", |c| {
        let (item, chr, fixing, fixer) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
        if !item.is_nil() {
            let cond = call_method(c.thread, &item, "getCondition", &[])
                .and_then(|v| v.as_num())
                .unwrap_or(0.0);
            let cond_max = call_method(c.thread, &item, "getConditionMax", &[])
                .and_then(|v| v.as_num())
                .unwrap_or(10.0);
            let add = compute_cond_repaired(c.thread, &item, &chr, &fixing, &fixer);
            let new_cond = (cond + add).min(cond_max);
            let _ = call_method(c.thread, &item, "setCondition", &[Value::Num(new_cond)]);
            let rep = call_method(c.thread, &item, "getHaveBeenRepaired", &[])
                .and_then(|v| v.as_num())
                .unwrap_or(1.0);
            let _ = call_method(
                c.thread,
                &item,
                "setHaveBeenRepaired",
                &[Value::Num(rep + 1.0)],
            );
        }
        Ok(c.push(item))
    });

    // zombie.randomizedWorld.randomizedBuilding.RBBasic
    let rb = expose_class(thread, RB_BASIC_CLASS, Some("java.lang.Object"));
    rb.ctor(|c| Ok(c.push(Value::Object(Rc::new(RbBasicObj)))));
    rb.static_fn("getUniqueRDSSpawned", |c| {
        let list = JavaList::live(
            || {
                RB_BASIC.with(|s| {
                    s.borrow()
                        .unique_rds_spawned
                        .iter()
                        .map(|name| Value::str(name))
                        .collect()
                })
            },
            Some(Box::new(|items| {
                RB_BASIC.with(|s| {
                    s.borrow_mut().unique_rds_spawned = items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect();
                });
                Ok(())
            })),
        );
        Ok(c.push(Value::Object(Rc::new(list))))
    });
    for static_helper in [
        "doOfficeStuff",
        "doNolansOfficeStuff",
        "doCafeStuff",
        "doGigamartStuff",
        "doGroceryStuff",
        "doGeneralRoom",
        "doJudgeStuff",
        "doTwiggyStuff",
        "doWoodcraftStuff",
    ] {
        rb.static_fn(static_helper, |_| Ok(0));
    }
    rb.method("getSurvivorStories", |c| {
        let items = RB_BASIC.with(|s| s.borrow().survivor_stories.clone());
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    rb.method("getSurvivorProfession", |c| {
        let items = RB_BASIC.with(|s| {
            s.borrow()
                .survivor_professions
                .iter()
                .map(|p| Value::str(p))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    rb.method("randomizeBuilding", |_| Ok(0));
    rb.method("forceVehicleDistribution", |_| Ok(0));
    rb.method("doProfessionStory", |_| Ok(0));
    rb.method("doRandomDeadSurvivorStory", |_| Ok(0));
    rb.method("doProfessionBuilding", |_| Ok(0));

    // zombie.iso.IsoMarkers$IsoMarker
    let mk = expose_class(thread, ISO_MARKER_CLASS, Some("java.lang.Object"));
    mk.ctor(|c| {
        Ok(c.push(Value::Object(Rc::new(IsoMarkerObj(Rc::new(
            RefCell::new(IsoMarkerData::default()),
        ))))))
    });
    mk.method("getID", |c| Ok(c.push(with_marker(c, |m| m.id).unwrap_or(0) as f64)));
    mk.method("remove", |c| {
        with_marker_mut(c, |m| m.removed = true);
        Ok(0)
    });
    mk.method("isRemoved", |c| {
        Ok(c.push(with_marker(c, |m| m.removed).unwrap_or(false)))
    });
    mk.method("getX", |c| Ok(c.push(f64::from(with_marker(c, |m| m.x).unwrap_or(0.0)))));
    mk.method("getY", |c| Ok(c.push(f64::from(with_marker(c, |m| m.y).unwrap_or(0.0)))));
    mk.method("getZ", |c| Ok(c.push(f64::from(with_marker(c, |m| m.z).unwrap_or(0.0)))));
    mk.method("getR", |c| Ok(c.push(f64::from(with_marker(c, |m| m.r).unwrap_or(1.0)))));
    mk.method("getG", |c| Ok(c.push(f64::from(with_marker(c, |m| m.g).unwrap_or(1.0)))));
    mk.method("getB", |c| Ok(c.push(f64::from(with_marker(c, |m| m.b).unwrap_or(1.0)))));
    mk.method("getA", |c| Ok(c.push(f64::from(with_marker(c, |m| m.a).unwrap_or(1.0)))));
    mk.method("setR", |c| {
        let v = f32_arg(c, 1);
        with_marker_mut(c, |m| m.r = v);
        Ok(0)
    });
    mk.method("setG", |c| {
        let v = f32_arg(c, 1);
        with_marker_mut(c, |m| m.g = v);
        Ok(0)
    });
    mk.method("setB", |c| {
        let v = f32_arg(c, 1);
        with_marker_mut(c, |m| m.b = v);
        Ok(0)
    });
    mk.method("setA", |c| {
        let v = f32_arg(c, 1);
        with_marker_mut(c, |m| m.a = v);
        Ok(0)
    });
    mk.method("setAlpha", |c| {
        let v = f32_arg(c, 1);
        with_marker_mut(c, |m| m.a = v);
        Ok(0)
    });
    mk.method("setColor", |c| {
        let (r, g, b, a) = (f32_arg(c, 1), f32_arg(c, 2), f32_arg(c, 3), f32_arg(c, 4));
        with_marker_mut(c, |m| {
            m.r = r;
            m.g = g;
            m.b = b;
            m.a = a;
        });
        Ok(0)
    });
    mk.method("getSquare", |c| {
        let sq = with_marker(c, |m| m.square.clone()).flatten();
        Ok(c.push(sq.unwrap_or(Value::Nil)))
    });
    mk.method("setSquare", |c| {
        let sq = c.arg(1);
        with_marker_mut(c, |m| m.square = (!sq.is_nil()).then_some(sq));
        Ok(0)
    });
    mk.method("setPos", |c| {
        let (x, y, z) = (i32_arg(c, 1) as f32, i32_arg(c, 2) as f32, i32_arg(c, 3) as f32);
        with_marker_mut(c, |m| {
            m.x = x;
            m.y = y;
            m.z = z;
        });
        Ok(0)
    });
    mk.method("isActive", |c| {
        Ok(c.push(with_marker(c, |m| m.active).unwrap_or(false)))
    });
    mk.method("setActive", |c| {
        let v = matches!(c.arg(1), Value::Bool(true));
        with_marker_mut(c, |m| m.active = v);
        Ok(0)
    });
    mk.method("setRotation", |c| {
        let v = f32_arg(c, 1);
        with_marker_mut(c, |m| m.rotation = v);
        Ok(0)
    });
    mk.method("getCircleSize", |c| {
        Ok(c.push(f64::from(with_marker(c, |m| m.circle_size).unwrap_or(1.0))))
    });
    mk.method("setCircleSize", |c| {
        let v = f32_arg(c, 1);
        with_marker_mut(c, |m| m.circle_size = v);
        Ok(0)
    });
    mk.method("init", |c| {
        let (x, y, z) = (i32_arg(c, 2) as f32, i32_arg(c, 3) as f32, i32_arg(c, 4) as f32);
        let sq = c.arg(5);
        with_marker_mut(c, |m| {
            m.x = x;
            m.y = y;
            m.z = z;
            m.square = (!sq.is_nil()).then_some(sq);
        });
        Ok(0)
    });

    // zombie.iso.IsoMarkers
    let im = expose_class(thread, ISO_MARKERS_CLASS, Some("java.lang.Object"));
    im.field("instance", Value::Object(Rc::new(IsoMarkersHandle)));
    im.method("reset", |_| {
        ISO_MARKERS.with(|s| *s.borrow_mut() = IsoMarkersState::default());
        Ok(0)
    });
    im.method("update", |_| {
        ISO_MARKERS.with(|s| {
            s.borrow_mut().markers.retain(|m| !m.borrow().removed);
        });
        Ok(0)
    });
    im.method("render", |_| Ok(0));
    im.method("renderIsoMarkers", |_| Ok(0));
    im.method("addIsoMarker", |c| {
        let sq = c.arg(2);
        let (x, y, z) = if !sq.is_nil() {
            let sx = call_method(c.thread, &sq, "getX", &[])
                .and_then(|v| v.as_num())
                .unwrap_or(0.0) as f32;
            let sy = call_method(c.thread, &sq, "getY", &[])
                .and_then(|v| v.as_num())
                .unwrap_or(0.0) as f32;
            let sz = call_method(c.thread, &sq, "getZ", &[])
                .and_then(|v| v.as_num())
                .unwrap_or(0.0) as f32;
            (sx, sy, sz)
        } else {
            (0.0, 0.0, 0.0)
        };
        let (r, g, b, a) = (f32_arg(c, 3), f32_arg(c, 4), f32_arg(c, 5), f32_arg(c, 6));
        let rc = ISO_MARKERS.with(|s| {
            let mut st = s.borrow_mut();
            st.next_id += 1;
            let id = st.next_id;
            let m = Rc::new(RefCell::new(IsoMarkerData {
                id,
                x,
                y,
                z,
                r,
                g,
                b,
                a,
                square: (!sq.is_nil()).then_some(sq),
                ..Default::default()
            }));
            st.markers.push(m.clone());
            m
        });
        Ok(c.push(Value::Object(Rc::new(IsoMarkerObj(rc)))))
    });
    im.method("getIsoMarker", |c| {
        let id = i32_arg(c, 1);
        let found = ISO_MARKERS.with(|s| {
            s.borrow()
                .markers
                .iter()
                .find(|m| m.borrow().id == id && !m.borrow().removed)
                .cloned()
                .map(|rc| Value::Object(Rc::new(IsoMarkerObj(rc))))
        });
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });
    im.method("removeIsoMarker", |c| {
        let removed = match c.arg(1) {
            Value::Num(n) => {
                let id = n as i32;
                ISO_MARKERS.with(|s| {
                    let mut st = s.borrow_mut();
                    let before = st.markers.len();
                    st.markers.retain(|m| {
                        if m.borrow().id == id {
                            m.borrow_mut().removed = true;
                            false
                        } else {
                            true
                        }
                    });
                    st.markers.len() < before
                })
            }
            Value::Object(o) => {
                let target_id = o
                    .as_any()
                    .downcast_ref::<IsoMarkerObj>()
                    .map(|m| m.0.borrow().id);
                if let Some(id) = target_id {
                    ISO_MARKERS.with(|s| {
                        let mut st = s.borrow_mut();
                        let before = st.markers.len();
                        st.markers.retain(|m| {
                            if m.borrow().id == id {
                                m.borrow_mut().removed = true;
                                false
                            } else {
                                true
                            }
                        });
                        st.markers.len() < before
                    })
                } else {
                    false
                }
            }
            _ => false,
        };
        Ok(c.push(removed))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn fixing_manager_rbbasic_and_iso_markers_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            local cond = 4
            local reps = 1
            local fake_item = {
                getType = function() return "Axe" end,
                getCondition = function() return cond end,
                getConditionMax = function() return 10 end,
                setCondition = function(_, v) cond = v end,
                getHaveBeenRepaired = function() return reps end,
                setHaveBeenRepaired = function(_, v) reps = v end,
            }
            local fake_fixing = { getConditionModifier = function() return 1.0 end }
            local fake_fixer = { getCondRepaired = function() return 50.0 end }
            assert(FixingManager.getCondRepaired(fake_item, nil, fake_fixing, fake_fixer) == 3)
            assert(FixingManager.getChanceOfFail(fake_item, nil, fake_fixing, fake_fixer) == 0)
            FixingManager.fixItem(fake_item, nil, fake_fixing, fake_fixer)
            assert(cond == 7 and reps == 2)
            assert(FixingManager.getFixes(fake_item):size() == 0)

            local rds = RBBasic.getUniqueRDSSpawned()
            rds:add("UniqueStory1")
            assert(RBBasic.getUniqueRDSSpawned():size() == 1)
            rds:clear()
            assert(RBBasic.getUniqueRDSSpawned():size() == 0)

            local fake_sq = { getX = function() return 100 end, getY = function() return 200 end, getZ = function() return 0 end }
            local mk = IsoMarkers.instance:addIsoMarker("sprite_0", fake_sq, 1.0, 0.5, 0.25, 0.8)
            assert(mk ~= nil and mk:getX() == 100 and mk:getY() == 200 and mk:isActive())
            assert(IsoMarkers.instance:getIsoMarker(mk:getID()) ~= nil)
            assert(IsoMarkers.instance:removeIsoMarker(mk) == true)
            assert(mk:isRemoved() and IsoMarkers.instance:getIsoMarker(mk:getID()) == nil)
            "#,
            Some("fixing_rbbasic_markers_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
