//! Build 42.22 Lua bindings for `zombie.buildingRooms`:
//! - `BuildingRoomsEditor`
//! - `BREBuilding`
//! - `BRERoom`

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, TableRef, Thread, Value};

use crate::expose::expose_class;

const EDITOR_CLASS: &str = "zombie.buildingRooms.BuildingRoomsEditor";
const BRE_BUILDING_CLASS: &str = "zombie.buildingRooms.BREBuilding";
const BRE_ROOM_CLASS: &str = "zombie.buildingRooms.BRERoom";
const BRE_ROOM_RECT_CLASS: &str = "zombie.buildingRooms.BRERoomRect";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RectData {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl RectData {
    fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    fn intersects(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        !(x + w <= self.x || x >= self.x + self.w || y + h <= self.y || y >= self.y + self.h)
    }

    fn is_adjacent(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        !self.intersects(x, y, w, h)
            && self.intersects(x - 1, y - 1, w + 2, h + 2)
    }
}

#[derive(Clone, Debug)]
struct BreRoomData {
    level: i32,
    name: String,
    rects: Vec<RectData>,
}

impl BreRoomData {
    fn new(level: i32) -> Self {
        Self {
            level,
            name: "room".to_string(),
            rects: Vec::new(),
        }
    }

    fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        self.level == z && self.rects.iter().any(|r| r.contains(x, y))
    }

    fn hit_test(&self, x: i32, y: i32) -> i32 {
        self.rects
            .iter()
            .position(|r| r.contains(x, y))
            .map(|i| i as i32)
            .unwrap_or(-1)
    }

    fn intersects(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.rects.iter().any(|r| r.intersects(x, y, w, h))
    }

    fn is_adjacent(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.rects.iter().any(|r| r.is_adjacent(x, y, w, h))
    }

    fn is_valid(&self) -> bool {
        !self.name.trim().is_empty() && !self.rects.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
struct BreBuildingData {
    edited: bool,
    rooms: Vec<Rc<RefCell<BreRoomData>>>,
}

impl BreBuildingData {
    fn room_index_at(&self, x: i32, y: i32, z: i32) -> i32 {
        self.rooms
            .iter()
            .position(|r| r.borrow().contains(x, y, z))
            .map(|i| i as i32)
            .unwrap_or(-1)
    }

    fn intersects(&self, x: i32, y: i32, w: i32, h: i32, z: i32) -> bool {
        self.rooms
            .iter()
            .any(|r| r.borrow().level == z && r.borrow().intersects(x, y, w, h))
    }

    fn is_adjacent(&self, x: i32, y: i32, w: i32, h: i32, z: i32) -> bool {
        self.rooms
            .iter()
            .any(|r| r.borrow().level == z && r.borrow().is_adjacent(x, y, w, h))
    }

    fn has_non_empty_rooms_on_level(&self, z: i32) -> bool {
        self.rooms
            .iter()
            .any(|r| r.borrow().level == z && !r.borrow().rects.is_empty())
    }

    fn is_valid(&self) -> bool {
        !self.rooms.is_empty() && self.rooms.iter().all(|r| r.borrow().is_valid())
    }
}

#[derive(Default)]
struct EditorState {
    lua_editor: Option<TableRef>,
    buildings: Vec<Rc<RefCell<BreBuildingData>>>,
    current_building: Option<Rc<RefCell<BreBuildingData>>>,
    current_room: Option<Rc<RefCell<BreRoomData>>>,
    highlight_rect_for_deletion: i32,
    current_level: i32,
    invalid_string: Option<String>,
}

thread_local! {
    static EDITOR: RefCell<EditorState> = RefCell::new(EditorState::default());
}

/// `BuildingRoomsEditor.getInstance().load()` called from `IsoWorld::init`.
pub fn load() {
    EDITOR.with(|e| {
        let mut s = e.borrow_mut();
        s.buildings.clear();
        s.current_building = None;
        s.current_room = None;
        s.invalid_string = None;
    });
}

/// `BuildingRoomsEditor.Reset()` called on world teardown.
pub fn reset() {
    EDITOR.with(|e| *e.borrow_mut() = EditorState::default());
}

#[derive(Debug)]
struct EditorHandle;
impl LuaObject for EditorHandle {
    fn class_name(&self) -> &'static str {
        EDITOR_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct BreBuildingObj(Rc<RefCell<BreBuildingData>>);
impl LuaObject for BreBuildingObj {
    fn class_name(&self) -> &'static str {
        BRE_BUILDING_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct BreRoomObj(Rc<RefCell<BreRoomData>>);
impl LuaObject for BreRoomObj {
    fn class_name(&self) -> &'static str {
        BRE_ROOM_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct RoomRectObj(RectData);
impl LuaObject for RoomRectObj {
    fn class_name(&self) -> &'static str {
        BRE_ROOM_RECT_CLASS
    }
    fn super_classes(&self) -> &'static [&'static str] {
        &["zombie.iso.RoomDef$RoomRect"]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn i32_arg(c: &NativeCall<'_>, idx: usize) -> i32 {
    c.arg(idx).as_num().unwrap_or(0.0) as i32
}

fn with_building<R>(c: &NativeCall<'_>, f: impl FnOnce(&BreBuildingData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let b = o.as_any().downcast_ref::<BreBuildingObj>()?;
    let res = Some(f(&b.0.borrow()));
    res
}

fn with_building_mut<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut BreBuildingData) -> R,
) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let b = o.as_any().downcast_ref::<BreBuildingObj>()?;
    let res = Some(f(&mut b.0.borrow_mut()));
    res
}

fn with_room<R>(c: &NativeCall<'_>, f: impl FnOnce(&BreRoomData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let r = o.as_any().downcast_ref::<BreRoomObj>()?;
    let res = Some(f(&r.0.borrow()));
    res
}

fn with_room_mut<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut BreRoomData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let r = o.as_any().downcast_ref::<BreRoomObj>()?;
    let res = Some(f(&mut r.0.borrow_mut()));
    res
}

/// Register `BuildingRoomsEditor`, `BREBuilding`, and `BRERoom`.
pub fn register(thread: &mut Thread) {
    // BRERoomRect (subtype of RoomDef$RoomRect; does not overwrite bind_world's metatable)
    let rr = expose_class(thread, BRE_ROOM_RECT_CLASS, Some("java.lang.Object"));
    rr.method("getX", |c| {
        let v = match c.arg(0) {
            Value::Object(o) => o.as_any().downcast_ref::<RoomRectObj>().map(|r| r.0.x).unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(v as f64))
    });
    rr.method("getY", |c| {
        let v = match c.arg(0) {
            Value::Object(o) => o.as_any().downcast_ref::<RoomRectObj>().map(|r| r.0.y).unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(v as f64))
    });
    rr.method("getW", |c| {
        let v = match c.arg(0) {
            Value::Object(o) => o.as_any().downcast_ref::<RoomRectObj>().map(|r| r.0.w).unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(v as f64))
    });
    rr.method("getH", |c| {
        let v = match c.arg(0) {
            Value::Object(o) => o.as_any().downcast_ref::<RoomRectObj>().map(|r| r.0.h).unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(v as f64))
    });

    // BRERoom
    let rm = expose_class(thread, BRE_ROOM_CLASS, Some("java.lang.Object"));
    rm.method("getLevel", |c| {
        Ok(c.push(with_room(c, |r| r.level).unwrap_or(0) as f64))
    });
    rm.method("getName", |c| {
        let s = with_room(c, |r| r.name.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    rm.method("setName", |c| {
        let arg1 = c.arg(1);
        if let Some(s) = arg1.as_str() {
            let owned = s.to_string();
            with_room_mut(c, |r| r.name = owned);
        }
        Ok(0)
    });
    rm.method("addRectangle", |c| {
        let rect = RectData {
            x: i32_arg(c, 1),
            y: i32_arg(c, 2),
            w: i32_arg(c, 3),
            h: i32_arg(c, 4),
        };
        with_room_mut(c, |r| r.rects.push(rect));
        Ok(0)
    });
    rm.method("removeRectangle", |c| {
        let idx = i32_arg(c, 1);
        with_room_mut(c, |r| {
            if idx >= 0 && (idx as usize) < r.rects.len() {
                r.rects.remove(idx as usize);
            }
        });
        Ok(0)
    });
    rm.method("getRectangleCount", |c| {
        Ok(c.push(with_room(c, |r| r.rects.len()).unwrap_or(0) as f64))
    });
    rm.method("getRectangle", |c| {
        let idx = i32_arg(c, 1);
        let rect = if idx >= 0 {
            with_room(c, |r| r.rects.get(idx as usize).copied()).flatten()
        } else {
            None
        };
        Ok(c.push(rect.map_or(Value::Nil, |r| Value::Object(Rc::new(RoomRectObj(r))))))
    });
    rm.method("contains", |c| {
        let ok = with_room(c, |r| r.contains(i32_arg(c, 1), i32_arg(c, 2), i32_arg(c, 3)))
            .unwrap_or(false);
        Ok(c.push(ok))
    });
    rm.method("hitTest", |c| {
        let idx = with_room(c, |r| r.hit_test(i32_arg(c, 1), i32_arg(c, 2))).unwrap_or(-1);
        Ok(c.push(idx as f64))
    });
    rm.method("intersects", |c| {
        let ok = with_room(c, |r| {
            r.intersects(i32_arg(c, 1), i32_arg(c, 2), i32_arg(c, 3), i32_arg(c, 4))
        })
        .unwrap_or(false);
        Ok(c.push(ok))
    });
    rm.method("isAdjacent", |c| {
        let ok = with_room(c, |r| {
            r.is_adjacent(i32_arg(c, 1), i32_arg(c, 2), i32_arg(c, 3), i32_arg(c, 4))
        })
        .unwrap_or(false);
        Ok(c.push(ok))
    });
    rm.method("copyFrom", |c| Ok(c.push(c.arg(0))));
    rm.method("isValid", |c| {
        Ok(c.push(with_room(c, |r| r.is_valid()).unwrap_or(false)))
    });

    // BREBuilding
    let bd = expose_class(thread, BRE_BUILDING_CLASS, Some("java.lang.Object"));
    bd.ctor(|c| {
        Ok(c.push(Value::Object(Rc::new(BreBuildingObj(Rc::new(
            RefCell::new(BreBuildingData::default()),
        ))))))
    });
    bd.method("getRoomCount", |c| {
        Ok(c.push(with_building(c, |b| b.rooms.len()).unwrap_or(0) as f64))
    });
    bd.method("getRoomByIndex", |c| {
        let idx = i32_arg(c, 1);
        let out = if idx >= 0 {
            with_building(c, |b| b.rooms.get(idx as usize).cloned()).flatten()
        } else {
            None
        };
        Ok(c.push(out.map_or(Value::Nil, |r| Value::Object(Rc::new(BreRoomObj(r))))))
    });
    bd.method("isEdited", |c| {
        Ok(c.push(with_building(c, |b| b.edited).unwrap_or(false)))
    });
    bd.method("setEdited", |c| {
        let v = matches!(c.arg(1), Value::Bool(true));
        with_building_mut(c, |b| b.edited = v);
        Ok(0)
    });
    bd.method("getRoomIndexAt", |c| {
        let idx = with_building(c, |b| {
            b.room_index_at(i32_arg(c, 1), i32_arg(c, 2), i32_arg(c, 3))
        })
        .unwrap_or(-1);
        Ok(c.push(idx as f64))
    });
    bd.method("createRoom", |c| {
        let level = i32_arg(c, 1);
        let room = Rc::new(RefCell::new(BreRoomData::new(level)));
        with_building_mut(c, |b| {
            b.rooms.push(room.clone());
            b.edited = true;
        });
        Ok(c.push(Value::Object(Rc::new(BreRoomObj(room)))))
    });
    bd.method("removeRoom", |c| {
        if let Value::Object(ro) = c.arg(1) {
            if let Some(target) = ro.as_any().downcast_ref::<BreRoomObj>() {
                let ptr = target.0.clone();
                with_building_mut(c, |b| {
                    b.rooms.retain(|r| !Rc::ptr_eq(r, &ptr));
                    b.edited = true;
                });
            }
        }
        Ok(0)
    });
    bd.method("isAdjacent", |c| {
        let ok = with_building(c, |b| {
            b.is_adjacent(
                i32_arg(c, 1),
                i32_arg(c, 2),
                i32_arg(c, 3),
                i32_arg(c, 4),
                i32_arg(c, 5),
            )
        })
        .unwrap_or(false);
        Ok(c.push(ok))
    });
    bd.method("intersects", |c| {
        let ok = with_building(c, |b| {
            b.intersects(
                i32_arg(c, 1),
                i32_arg(c, 2),
                i32_arg(c, 3),
                i32_arg(c, 4),
                i32_arg(c, 5),
            )
        })
        .unwrap_or(false);
        Ok(c.push(ok))
    });
    bd.method("hasNonEmptyRoomsOnLevel", |c| {
        let ok = with_building(c, |b| b.has_non_empty_rooms_on_level(i32_arg(c, 1)))
            .unwrap_or(false);
        Ok(c.push(ok))
    });
    bd.method("copyFrom", |c| Ok(c.push(c.arg(0))));
    bd.method("isValid", |c| {
        Ok(c.push(with_building(c, |b| b.is_valid()).unwrap_or(false)))
    });
    bd.method("applyChanges", |_| Ok(0));

    // BuildingRoomsEditor
    let ed = expose_class(thread, EDITOR_CLASS, Some("java.lang.Object"));
    ed.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(EditorHandle))))
    });
    ed.static_fn("Reset", |_| {
        reset();
        Ok(0)
    });
    ed.method("setLuaEditor", |c| {
        let tbl = c.arg(1).as_table().cloned();
        EDITOR.with(|e| e.borrow_mut().lua_editor = tbl);
        Ok(0)
    });
    ed.method("getBuildingCount", |c| {
        let n = EDITOR.with(|e| e.borrow().buildings.len());
        Ok(c.push(n as f64))
    });
    ed.method("getBuildingByIndex", |c| {
        let idx = i32_arg(c, 1);
        let out = if idx >= 0 {
            EDITOR.with(|e| e.borrow().buildings.get(idx as usize).cloned())
        } else {
            None
        };
        Ok(c.push(out.map_or(Value::Nil, |b| Value::Object(Rc::new(BreBuildingObj(b))))))
    });
    ed.method("createBuilding", |c| {
        let b = Rc::new(RefCell::new(BreBuildingData::default()));
        EDITOR.with(|e| e.borrow_mut().buildings.push(b.clone()));
        Ok(c.push(Value::Object(Rc::new(BreBuildingObj(b)))))
    });
    ed.method("init", |_| Ok(0));
    ed.method("copyExistingBuilding", |c| {
        let b = Rc::new(RefCell::new(BreBuildingData::default()));
        EDITOR.with(|e| e.borrow_mut().buildings.push(b.clone()));
        Ok(c.push(Value::Object(Rc::new(BreBuildingObj(b)))))
    });
    ed.method("removeBuilding", |c| {
        if let Value::Object(bo) = c.arg(1) {
            if let Some(target) = bo.as_any().downcast_ref::<BreBuildingObj>() {
                let ptr = target.0.clone();
                EDITOR.with(|e| e.borrow_mut().buildings.retain(|b| !Rc::ptr_eq(b, &ptr)));
            }
        }
        Ok(0)
    });
    ed.method("canAddRoomRectangle", |c| {
        let (x, y, w, h, z) = (
            i32_arg(c, 2),
            i32_arg(c, 3),
            i32_arg(c, 4),
            i32_arg(c, 5),
            i32_arg(c, 6),
        );
        let ok = w > 0
            && h > 0
            && !EDITOR.with(|e| {
                e.borrow()
                    .buildings
                    .iter()
                    .any(|b| b.borrow().intersects(x, y, w, h, z))
            });
        Ok(c.push(ok))
    });
    ed.method("setCurrentBuilding", |c| {
        let b = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<BreBuildingObj>().map(|x| x.0.clone()),
            _ => None,
        };
        EDITOR.with(|e| e.borrow_mut().current_building = b);
        Ok(0)
    });
    ed.method("setCurrentRoom", |c| {
        let r = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<BreRoomObj>().map(|x| x.0.clone()),
            _ => None,
        };
        EDITOR.with(|e| e.borrow_mut().current_room = r);
        Ok(0)
    });
    ed.method("setHighlightRectForDeletion", |c| {
        let idx = i32_arg(c, 1);
        EDITOR.with(|e| e.borrow_mut().highlight_rect_for_deletion = idx);
        Ok(0)
    });
    ed.method("setCurrentLevel", |c| {
        let z = i32_arg(c, 1);
        EDITOR.with(|e| e.borrow_mut().current_level = z);
        Ok(0)
    });
    ed.method("isValid", |c| {
        let ok = EDITOR.with(|e| e.borrow().buildings.iter().all(|b| b.borrow().is_valid()));
        Ok(c.push(ok))
    });
    ed.method("getInvalidString", |c| {
        let s = EDITOR.with(|e| e.borrow().invalid_string.clone());
        Ok(c.push(s.map_or(Value::Nil, |s| Value::str(&s))))
    });
    ed.method("applyChanges", |_| Ok(0));
    ed.method("checkBuildingAndRoomIDs", |_| Ok(0));
    ed.method("renderMain", |_| Ok(0));
    ed.method("load", |_| {
        load();
        Ok(0)
    });
    ed.method("callLua", |_| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn building_rooms_editor_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            local ed = BuildingRoomsEditor.getInstance()
            ed:load()
            assert(ed:getBuildingCount() == 0)
            local b = ed:createBuilding()
            assert(ed:getBuildingCount() == 1 and not b:isValid())
            local r = b:createRoom(0)
            r:setName("kitchen")
            assert(ed:canAddRoomRectangle(r, 10, 10, 5, 5, 0) == true)
            r:addRectangle(10, 10, 5, 5)
            assert(r:getRectangleCount() == 1 and r:contains(12, 12, 0))
            assert(r:hitTest(12, 12) == 0 and r:isValid() and b:isValid())
            assert(ed:canAddRoomRectangle(r, 12, 12, 4, 4, 0) == false)
            assert(b:getRoomIndexAt(12, 12, 0) == 0)
            assert(b:hasNonEmptyRoomsOnLevel(0) == true)
            assert(ed:isValid() == true)
            ed:removeBuilding(b)
            assert(ed:getBuildingCount() == 0)
            "#,
            Some("building_rooms_editor_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
