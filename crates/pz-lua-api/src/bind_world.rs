//! World and map classes as `LuaManager.Exposer` publishes them:
//! `zombie.iso.RoomDef`, `RoomDef$RoomRect`, `BuildingDef`, `MetaObject`,
//! `IsoMetaGrid`, `IsoMetaCell`, `IsoMetaChunk`, `IsoCell`, `IsoChunk`,
//! `IsoGridSquare` and `IsoWorld`, backed by `pz-world`.
//!
//! The world is a per-thread singleton like `IsoWorld.instance`: the engine
//! installs the meta grid and the current cell with [`set_meta_grid`] and
//! [`set_cell`]. Lua sees handles into that state. A handle is cached weakly
//! per object, so the same room, square, ... is the same Lua value while Lua
//! holds it (Java identity). Replacing the meta grid (or the cell) makes the
//! old handles into it stale; using one raises an error.
//!
//! Only methods `pz-world` can back are registered; a method whose Java body
//! needs an unported system (IsoRoom, zones, Rand, SandboxOptions, rendering,
//! inventory, ...) is left unbound. Public static constants are not
//! registered here: their values come from the game's classes at runtime.

use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, ClassBuilder};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, NativeFunction, TableRef, Thread, Value};
use pz_world::defs::room_id;
use pz_world::java::fastfloor;
use pz_world::{BuildingDef, BuildingIdx, IsoCell, IsoMetaGrid, RoomDef, RoomIdx, RoomRect};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

mod cell_drag;
pub mod lua_world;
mod reachability;
mod square_queries;
pub mod power;

/// `IsoWorld.instance.metaGrid` and `IsoWorld.instance.currentCell`.
#[derive(Default)]
pub struct WorldState {
    pub meta_grid: Option<IsoMetaGrid>,
    pub cell: Option<IsoCell>,
    /// `currentCell.weatherFx`, initialized by `initWeatherFx` and retired
    /// with the cell. The simulation exists independently of GPU drawing.
    pub weather_fx: Option<Rc<RefCell<pz_iso_render::weather_fx::IsoWeatherFx>>>,
}

thread_local! {
    static WORLD: RefCell<WorldState> = RefCell::default();
    /// Generations of the meta grid and of the cell, for stale handles.
    static META_GEN: Cell<u64> = const { Cell::new(0) };
    static CELL_GEN: Cell<u64> = const { Cell::new(0) };
    static HANDLES: RefCell<HashMap<Key, Weak<Handle>>> = RefCell::default();
}

/// Install (or clear) the meta grid Lua sees through `IsoWorld`.
pub fn set_meta_grid(grid: Option<IsoMetaGrid>) {
    WORLD.with(|w| w.borrow_mut().meta_grid = grid);
    META_GEN.with(|g| g.set(g.get() + 1));
    HANDLES.with(|h| h.borrow_mut().retain(|k, _| !k.is_meta()));
}

/// Install (or clear) `IsoWorld.instance.currentCell`.
pub fn set_cell(cell: Option<IsoCell>) {
    let retired = WORLD.with(|w| {
        let mut w = w.borrow_mut();
        let weather = w.weather_fx.take();
        let cell = std::mem::replace(&mut w.cell, cell);
        (weather, cell)
    });
    CELL_GEN.with(|g| g.set(g.get() + 1));
    HANDLES.with(|h| {
        h.borrow_mut()
            .retain(|k, _| k.is_meta() || *k == Key::World)
    });
    // Live process queues can be the final owners of items carrying Lua
    // parents. Their destructors must observe the new cell without an
    // outstanding WORLD borrow.
    drop(retired);
}

/// `IsoWorld.instance.getMetaGrid()`'s `(minX, minY, maxX, maxY)` in cells,
/// `None` without a meta grid.
pub fn meta_grid_cell_bounds() -> Option<(i32, i32, i32, i32)> {
    WORLD.with(|w| {
        w.borrow()
            .meta_grid
            .as_ref()
            .map(|g| (g.min_x, g.min_y, g.max_x, g.max_y))
    })
}

/// Access the world state (e.g. to load chunks into the cell). Must not be
/// called from inside a bound method.
pub fn with_world<R>(f: impl FnOnce(&mut WorldState) -> R) -> R {
    WORLD.with(|w| f(&mut w.borrow_mut()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    World,
    MetaGrid,
    Cell,
    /// Absolute cell coordinates.
    MetaCell(i32, i32),
    /// Absolute chunk coordinates.
    MetaChunk(i32, i32),
    Room(u32),
    Building(u32),
    /// Room, index in `objects`.
    MetaObject(u32, u32),
    /// Room, index in `rects`.
    RoomRect(u32, u32),
    Chunk(i32, i32),
    Square(i32, i32, i32),
    /// A meta-grid zone (arena index) and its Java class.
    Zone(u32, ZoneClass),
}

/// The class of a zone object (`Zone` or the subclass map loading made).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ZoneClass {
    Zone,
    Vehicle,
    Mannequin,
    WorldGen,
    Animal,
}

impl ZoneClass {
    fn of(kind: &pz_world::zones::ZoneKind) -> ZoneClass {
        use pz_world::zones::ZoneKind as K;
        match kind {
            K::Zone => ZoneClass::Zone,
            K::Vehicle(_) => ZoneClass::Vehicle,
            K::Mannequin(_) => ZoneClass::Mannequin,
            K::WorldGen { .. } => ZoneClass::WorldGen,
            K::Animal(_) => ZoneClass::Animal,
        }
    }
    fn name(self) -> &'static str {
        match self {
            ZoneClass::Zone => "zombie.iso.zones.Zone",
            ZoneClass::Vehicle => "zombie.iso.zones.VehicleZone",
            ZoneClass::Mannequin => "zombie.iso.objects.IsoMannequin$MannequinZone",
            ZoneClass::WorldGen => "zombie.iso.worldgen.zones.WorldGenZone",
            ZoneClass::Animal => "zombie.characters.animals.AnimalZone",
        }
    }
}

impl Key {
    fn is_meta(&self) -> bool {
        matches!(
            self,
            Key::MetaGrid
                | Key::MetaCell(..)
                | Key::MetaChunk(..)
                | Key::Room(_)
                | Key::Building(_)
                | Key::MetaObject(..)
                | Key::RoomRect(..)
                | Key::Zone(..)
        )
    }
    fn generation(&self) -> u64 {
        match self {
            Key::World => 0,
            k if k.is_meta() => META_GEN.with(Cell::get),
            _ => CELL_GEN.with(Cell::get),
        }
    }
    fn class(&self) -> &'static str {
        match self {
            Key::World => "zombie.iso.IsoWorld",
            Key::MetaGrid => "zombie.iso.IsoMetaGrid",
            Key::Cell => "zombie.iso.IsoCell",
            Key::MetaCell(..) => "zombie.iso.IsoMetaCell",
            Key::MetaChunk(..) => "zombie.iso.IsoMetaChunk",
            Key::Room(_) => "zombie.iso.RoomDef",
            Key::Building(_) => "zombie.iso.BuildingDef",
            Key::MetaObject(..) => "zombie.iso.MetaObject",
            Key::RoomRect(..) => RECT_CLASS,
            Key::Chunk(..) => "zombie.iso.IsoChunk",
            Key::Square(..) => "zombie.iso.IsoGridSquare",
            Key::Zone(_, c) => c.name(),
        }
    }
}

const RECT_CLASS: &str = "zombie.iso.RoomDef$RoomRect";

/// A Lua-visible reference to a world object.
struct Handle {
    key: Key,
    generation: u64,
}

impl LuaObject for Handle {
    fn class_name(&self) -> &str {
        self.key.class()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `RoomRect` created by `RoomRect.new`, not owned by a room.
struct FreeRect(RefCell<RoomRect>);

impl LuaObject for FreeRect {
    fn class_name(&self) -> &str {
        RECT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `java.util.ArrayList` returned by a getter (a snapshot of the list).
struct JavaList(Vec<Value>);

impl LuaObject for JavaList {
    fn class_name(&self) -> &str {
        "java.util.ArrayList"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn obj(key: Key) -> Value {
    HANDLES.with(|h| {
        let mut h = h.borrow_mut();
        if let Some(rc) = h.get(&key).and_then(Weak::upgrade) {
            return Value::Object(rc);
        }
        if h.len() > 4096 {
            h.retain(|_, w| w.strong_count() > 0);
        }
        let rc = Rc::new(Handle {
            key,
            generation: key.generation(),
        });
        h.insert(key, Rc::downgrade(&rc));
        Value::Object(rc)
    })
}

fn opt(key: Option<Key>) -> Value {
    key.map_or(Value::Nil, obj)
}

fn list(items: impl IntoIterator<Item = Value>) -> Value {
    Value::Object(Rc::new(JavaList(items.into_iter().collect())))
}

fn long(v: i64) -> Value {
    Value::Num(v as f64)
}

fn java_err(class: &'static str, message: impl Into<String>) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

fn npe() -> LuaError {
    java_err("java.lang.NullPointerException", "")
}

fn index_oob(i: i64, len: usize) -> LuaError {
    java_err(
        "java.lang.IndexOutOfBoundsException",
        format!("Index {i} out of bounds for length {len}"),
    )
}

/// Key of a handle argument; `nil` is a Java `null` (NPE when used).
fn key_of(v: &Value) -> LuaResult<Key> {
    match v {
        Value::Nil => Err(npe()),
        Value::Object(o) => match o.as_any().downcast_ref::<Handle>() {
            Some(h) if h.generation == h.key.generation() => Ok(h.key),
            Some(h) => Err(LuaError::runtime(format!(
                "stale {} (world data was replaced)",
                h.key.class()
            ))),
            None => Err(LuaError::runtime(format!("unexpected {}", o.class_name()))),
        },
        other => Err(LuaError::runtime(format!(
            "unexpected {}",
            other.type_name()
        ))),
    }
}

fn wrong(c: &NativeCall<'_>, i: usize, want: &str) -> LuaError {
    LuaError::runtime(format!(
        "Expected {want} but got {}",
        pz_kahlua::stdlib::java_class_name(&c.arg(i))
    ))
}

fn room_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<usize> {
    match key_of(&c.arg(i))? {
        Key::Room(r) => Ok(r as usize),
        _ => Err(wrong(c, i, "zombie.iso.RoomDef")),
    }
}

fn building_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<usize> {
    match key_of(&c.arg(i))? {
        Key::Building(b) => Ok(b as usize),
        _ => Err(wrong(c, i, "zombie.iso.BuildingDef")),
    }
}

/// The x, y, z of an `IsoGridSquare` Lua object; `None` for anything else
/// (or a square from replaced world data).
pub fn square_coords(v: &Value) -> Option<(i32, i32, i32)> {
    match key_of(v) {
        Ok(Key::Square(x, y, z)) => Some((x, y, z)),
        _ => None,
    }
}

fn square_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<(i32, i32, i32)> {
    match key_of(&c.arg(i))? {
        Key::Square(x, y, z) => Ok((x, y, z)),
        _ => Err(wrong(c, i, "zombie.iso.IsoGridSquare")),
    }
}

/// `(long)` conversion of a Lua number (`Double.longValue()`).
fn long_arg(c: &NativeCall<'_>, i: usize) -> i64 {
    num_arg(c, i) as i64
}

/// `PZMath.fastfloor(double)`.
fn fastfloor_d(v: f64) -> i32 {
    let xi = v as i32;
    if v < xi as f64 {
        xi - 1
    } else {
        xi
    }
}

fn grid<R>(f: impl FnOnce(&mut IsoMetaGrid) -> LuaResult<R>) -> LuaResult<R> {
    WORLD.with(|w| match w.borrow_mut().meta_grid.as_mut() {
        Some(g) => f(g),
        None => Err(LuaError::runtime("IsoMetaGrid is not loaded")),
    })
}

/// `IsoWorld.instance.currentCell`, NPE when there is none.
fn cell<R>(f: impl FnOnce(&IsoCell) -> LuaResult<R>) -> LuaResult<R> {
    WORLD.with(|w| match w.borrow().cell.as_ref() {
        Some(c) => f(c),
        None => Err(npe()),
    })
}

fn square_exists(x: i32, y: i32, z: i32) -> bool {
    WORLD.with(|w| {
        w.borrow()
            .cell
            .as_ref()
            .is_some_and(|c| c.get_grid_square(x, y, z).is_some())
    })
}

fn square_key(x: i32, y: i32, z: i32) -> Option<Key> {
    square_exists(x, y, z).then_some(Key::Square(x, y, z))
}

/// The `IsoGridSquare` Lua object at `(x, y, z)` of the current cell
/// (`nil` when the cell has no square there), e.g. for event arguments.
pub fn square_value(x: i32, y: i32, z: i32) -> Value {
    opt(square_key(x, y, z))
}

/// Method returning one value.
fn get(
    b: &ClassBuilder,
    name: &str,
    f: impl Fn(&mut NativeCall<'_>) -> LuaResult<Value> + 'static,
) {
    b.method(name, move |c| {
        let v = f(c)?;
        Ok(c.push(v))
    });
}

/// `void` method.
fn void(b: &ClassBuilder, name: &str, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<()> + 'static) {
    b.method(name, move |c| {
        f(c)?;
        Ok(0)
    });
}

fn static_get(
    b: &ClassBuilder,
    name: &str,
    f: impl Fn(&mut NativeCall<'_>) -> LuaResult<Value> + 'static,
) {
    b.static_fn(name, move |c| {
        let v = f(c)?;
        Ok(c.push(v))
    });
}

fn self_room(c: &NativeCall<'_>) -> LuaResult<usize> {
    room_arg(c, 0)
}

fn self_building(c: &NativeCall<'_>) -> LuaResult<usize> {
    building_arg(c, 0)
}

fn room_get(b: &ClassBuilder, name: &str, f: impl Fn(&RoomDef) -> Value + 'static) {
    get(b, name, move |c| {
        let r = self_room(c)?;
        grid(|g| Ok(f(&g.room_defs[r])))
    });
}

fn building_get(b: &ClassBuilder, name: &str, f: impl Fn(&BuildingDef) -> Value + 'static) {
    get(b, name, move |c| {
        let i = self_building(c)?;
        grid(|g| Ok(f(&g.building_defs[i])))
    });
}

pub fn register(thread: &mut Thread) {
    register_world(thread, None);
}

/// Expose the world and connect cursor mutation to the actual Lua session's
/// registered SetDragItem event. Standalone read-only exposure needs no sink.
pub fn register_with_events(thread: &mut Thread, events: crate::events::EventManager) {
    register_world(thread, Some(events.add_event("SetDragItem")));
}

fn register_world(thread: &mut Thread, drag_event: Option<Rc<RefCell<crate::events::Event>>>) {
    register_array_list(thread);
    register_room_def(thread);
    register_room_rect(thread);
    register_meta_object(thread);
    register_building_def(thread);
    register_meta_chunk(thread);
    register_meta_cell(thread);
    register_meta_grid(thread);
    register_iso_cell(thread, drag_event);
    register_iso_chunk(thread);
    register_grid_square(thread);
    register_iso_world(thread);
    crate::expose::global(thread, "getCell", |c| Ok(c.push(current_cell())));
    lua_world::register_zone(thread);
}

/// Instance metatable for the lists getters return, unless another module
/// already exposes `java.util.ArrayList`.
fn register_array_list(thread: &mut Thread) {
    if !thread
        .class_metatables
        .get_str("java.util.ArrayList")
        .is_nil()
    {
        return;
    }
    let meta = TableRef::new();
    let methods = TableRef::new();
    meta.set_str("__index", methods.clone());
    thread.class_metatables.set_str("java.util.ArrayList", meta);
    let this =
        |c: &NativeCall<'_>| -> LuaResult<Rc<dyn LuaObject>> { crate::expose::this::<JavaList>(c) };
    let items = |o: &Rc<dyn LuaObject>| o.as_any().downcast_ref::<JavaList>().unwrap().0.clone();
    type Body = Rc<dyn Fn(&mut NativeCall<'_>) -> LuaResult<usize>>;
    let add = |name: &str, f: Body| {
        methods.set_str(
            name,
            Value::Native(NativeFunction::new(name, move |c| f(c))),
        );
    };
    add(
        "size",
        Rc::new(move |c| {
            let n = items(&this(c)?).len() as i32;
            Ok(c.push(n))
        }),
    );
    add(
        "isEmpty",
        Rc::new(move |c| {
            let e = items(&this(c)?).is_empty();
            Ok(c.push(e))
        }),
    );
    add(
        "get",
        Rc::new(move |c| {
            let v = items(&this(c)?);
            let i = int_arg(c, 1);
            let r = usize::try_from(i).ok().and_then(|i| v.get(i).cloned());
            match r {
                Some(x) => Ok(c.push(x)),
                None => Err(index_oob(i as i64, v.len())),
            }
        }),
    );
    add(
        "contains",
        Rc::new(move |c| {
            let x = c.arg(1);
            let r = items(&this(c)?).iter().any(|v| v.raw_equals(&x));
            Ok(c.push(r))
        }),
    );
    add(
        "indexOf",
        Rc::new(move |c| {
            let x = c.arg(1);
            let r = items(&this(c)?)
                .iter()
                .position(|v| v.raw_equals(&x))
                .map_or(-1, |i| i as i32);
            Ok(c.push(r))
        }),
    );
}

// ---------------------------------------------------------------- RoomDef

fn room_is_adjacent_rect(r: &RoomDef, x: i32, y: i32, w: i32, h: i32) -> bool {
    r.intersects(x - 1, y - 1, w + 2, h + 2)
}

fn room_overlaps(a: &RoomDef, b: &RoomDef) -> bool {
    b.rects
        .iter()
        .any(|o| a.get_area_overlapping(o.x, o.y, o.w, o.h) > 0.0)
}

fn room_is_adjacent(a: &RoomDef, b: &RoomDef) -> bool {
    b.rects
        .iter()
        .any(|o| room_is_adjacent_rect(a, o.x, o.y, o.w, o.h))
}

fn room_dispose(r: &mut RoomDef) {
    r.building = None;
    r.rects.clear();
    r.objects.clear();
}

fn register_room_def(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.RoomDef", Some("java.lang.Object"));
    room_get(&b, "getID", |r| long(r.id));
    room_get(&b, "getIDString", |r| Value::from(r.id.to_string()));
    room_get(&b, "isExplored", |r| r.explored.into());
    room_get(&b, "getName", |r| Value::from(r.name.as_str()));
    room_get(&b, "getX", |r| r.x.into());
    room_get(&b, "getY", |r| r.y.into());
    room_get(&b, "getX2", |r| r.x2.into());
    room_get(&b, "getY2", |r| r.y2.into());
    room_get(&b, "getW", |r| (r.x2 - r.x).into());
    room_get(&b, "getH", |r| (r.y2 - r.y).into());
    room_get(&b, "getZ", |r| r.level.into());
    room_get(&b, "getArea", |r| r.area.into());
    room_get(&b, "isEmptyOutside", |r| r.is_empty_outside().into());
    room_get(&b, "isRoofFixed", |r| r.roof_fixed.into());
    room_get(&b, "isUserDefined", |r| r.user_defined.into());
    room_get(&b, "getBuilding", |r| {
        opt(r.building.map(|b| Key::Building(b.0)))
    });
    get(&b, "getRects", |c| {
        let r = self_room(c)?;
        let n = grid(|g| Ok(g.room_defs[r].rects.len()))?;
        Ok(list((0..n).map(|i| obj(Key::RoomRect(r as u32, i as u32)))))
    });
    for name in ["getObjects", "getMetaObjects"] {
        get(&b, name, |c| {
            let r = self_room(c)?;
            let n = grid(|g| Ok(g.room_defs[r].objects.len()))?;
            Ok(list(
                (0..n).map(|i| obj(Key::MetaObject(r as u32, i as u32))),
            ))
        });
    }
    void(&b, "setExplored", |c| {
        let (r, v) = (self_room(c)?, bool_arg(c, 1));
        grid(|g| {
            g.room_defs[r].explored = v;
            Ok(())
        })
    });
    void(&b, "setRoofFixed", |c| {
        let (r, v) = (self_room(c)?, bool_arg(c, 1));
        grid(|g| {
            g.room_defs[r].roof_fixed = v;
            Ok(())
        })
    });
    void(&b, "setName", |c| {
        let r = self_room(c)?;
        // `name` is never null here, so the new name is always taken.
        let v = str_arg(c, 1)
            .ok_or_else(|| LuaError::runtime("setName: null name is not supported"))?;
        grid(|g| {
            g.room_defs[r].name = v.to_string();
            Ok(())
        })
    });
    void(&b, "setBuilding", |c| {
        let r = self_room(c)?;
        let def = match c.arg(1) {
            Value::Nil => None,
            _ => Some(BuildingIdx(building_arg(c, 1)? as u32)),
        };
        grid(|g| {
            g.room_defs[r].building = def;
            Ok(())
        })
    });
    get(&b, "isInside", |c| {
        let r = self_room(c)?;
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        grid(|g| {
            let room = &g.room_defs[r];
            // The original reads `building.x` first.
            room.building.ok_or_else(npe)?;
            Ok(room.is_inside(x, y, z).into())
        })
    });
    get(&b, "contains", |c| {
        let r = self_room(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        grid(|g| Ok(g.room_defs[r].contains(x, y).into()))
    });
    get(&b, "intersects", |c| {
        let r = self_room(c)?;
        let (x, y, w, h) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        grid(|g| Ok(g.room_defs[r].intersects(x, y, w, h).into()))
    });
    get(&b, "isAdjacent", |c| {
        let r = self_room(c)?;
        if let Value::Object(_) = c.arg(1) {
            let o = room_arg(c, 1)?;
            grid(|g| Ok(room_is_adjacent(&g.room_defs[r], &g.room_defs[o]).into()))
        } else {
            let (x, y, w, h) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
            grid(|g| Ok(room_is_adjacent_rect(&g.room_defs[r], x, y, w, h).into()))
        }
    });
    get(&b, "overlaps", |c| {
        let (r, o) = (self_room(c)?, room_arg(c, 1)?);
        grid(|g| Ok(room_overlaps(&g.room_defs[r], &g.room_defs[o]).into()))
    });
    get(&b, "getAreaOverlapping", |c| {
        let r = self_room(c)?;
        let (x, y, w, h) = match c.arg(1) {
            Value::Object(_) => match key_of(&c.arg(1))? {
                Key::Chunk(wx, wy) => (wx * 8, wy * 8, 8, 8),
                _ => return Err(wrong(c, 1, "zombie.iso.IsoChunk")),
            },
            _ => (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4)),
        };
        grid(|g| {
            Ok(Value::Num(
                g.room_defs[r].get_area_overlapping(x, y, w, h) as f64
            ))
        })
    });
    void(&b, "CalculateBounds", |c| {
        let r = self_room(c)?;
        grid(|g| {
            g.room_defs[r].CalculateBounds();
            Ok(())
        })
    });
    get(&b, "calculateMetaID", |c| {
        let r = self_room(c)?;
        let (cx, cy) = (int_arg(c, 1), int_arg(c, 2));
        grid(|g| Ok(long(g.room_defs[r].calculate_meta_id(cx, cy))))
    });
    void(&b, "offset", |c| {
        let r = self_room(c)?;
        let (dx, dy) = (int_arg(c, 1), int_arg(c, 2));
        grid(|g| {
            let room = &mut g.room_defs[r];
            room.x += dx;
            room.y += dy;
            room.x2 += dx;
            room.y2 += dy;
            for rect in &mut room.rects {
                rect.x += dx;
                rect.y += dy;
            }
            Ok(())
        })
    });
    get(&b, "getRoomRect", |c| {
        let r = self_room(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let i = grid(|g| {
            Ok(g.room_defs[r]
                .rects
                .iter()
                .position(|rc| !(rc.x > x || rc.y > y || rc.get_x2() < x || rc.get_y2() < y)))
        })?;
        Ok(opt(i.map(|i| Key::RoomRect(r as u32, i as u32))))
    });
    void(&b, "Dispose", |c| {
        let r = self_room(c)?;
        grid(|g| {
            room_dispose(&mut g.room_defs[r]);
            Ok(())
        })
    });
}

// ---------------------------------------------------------------- RoomRect

fn rect_value(v: &Value) -> LuaResult<RoomRect> {
    if let Value::Object(o) = v {
        if let Some(f) = o.as_any().downcast_ref::<FreeRect>() {
            return Ok(*f.0.borrow());
        }
    }
    match key_of(v)? {
        Key::RoomRect(r, i) => grid(|g| {
            let rects = &g.room_defs[r as usize].rects;
            rects
                .get(i as usize)
                .copied()
                .ok_or_else(|| index_oob(i as i64, rects.len()))
        }),
        _ => Err(LuaError::runtime(format!("Expected {RECT_CLASS}"))),
    }
}

fn register_room_rect(thread: &mut Thread) {
    let b = expose_class(thread, RECT_CLASS, Some("java.lang.Object"));
    b.ctor(|c| {
        let r = if c.nargs() >= 4 {
            RoomRect::new(int_arg(c, 0), int_arg(c, 1), int_arg(c, 2), int_arg(c, 3))
        } else {
            RoomRect::new(0, 0, 0, 0)
        };
        Ok(c.push(Value::Object(Rc::new(FreeRect(RefCell::new(r))))))
    });
    let field = |b: &ClassBuilder, name: &str, f: fn(&RoomRect) -> i32| {
        get(b, name, move |c| Ok(f(&rect_value(&c.arg(0))?).into()));
    };
    field(&b, "getX", |r| r.x);
    field(&b, "getY", |r| r.y);
    field(&b, "getW", |r| r.w);
    field(&b, "getH", |r| r.h);
    field(&b, "getX2", |r| r.get_x2());
    field(&b, "getY2", |r| r.get_y2());
    get(&b, "contains", |c| {
        let r = rect_value(&c.arg(0))?;
        Ok(r.contains(num_arg(c, 1) as f32, num_arg(c, 2) as f32)
            .into())
    });
    get(&b, "set", |c| {
        let this = c.arg(0);
        let new = RoomRect::new(int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        if let Value::Object(o) = &this {
            if let Some(f) = o.as_any().downcast_ref::<FreeRect>() {
                *f.0.borrow_mut() = new;
                return Ok(this);
            }
        }
        match key_of(&this)? {
            Key::RoomRect(r, i) => grid(|g| {
                let rects = &mut g.room_defs[r as usize].rects;
                let len = rects.len();
                *rects
                    .get_mut(i as usize)
                    .ok_or_else(|| index_oob(i as i64, len))? = new;
                Ok(())
            })?,
            _ => return Err(wrong(c, 0, RECT_CLASS)),
        }
        Ok(this)
    });
}

// ---------------------------------------------------------------- MetaObject

fn meta_object<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut pz_world::MetaObject) -> R,
) -> LuaResult<R> {
    match key_of(&c.arg(0))? {
        Key::MetaObject(r, i) => grid(|g| {
            let objs = &mut g.room_defs[r as usize].objects;
            let len = objs.len();
            Ok(f(objs
                .get_mut(i as usize)
                .ok_or_else(|| index_oob(i as i64, len))?))
        }),
        _ => Err(wrong(c, 0, "zombie.iso.MetaObject")),
    }
}

fn register_meta_object(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.MetaObject", Some("java.lang.Object"));
    get(&b, "getRoom", |c| match key_of(&c.arg(0))? {
        Key::MetaObject(r, _) => Ok(obj(Key::Room(r))),
        _ => Err(wrong(c, 0, "zombie.iso.MetaObject")),
    });
    get(&b, "getUsed", |c| Ok(meta_object(c, |o| o.used)?.into()));
    get(&b, "getX", |c| Ok(meta_object(c, |o| o.x)?.into()));
    get(&b, "getY", |c| Ok(meta_object(c, |o| o.y)?.into()));
    get(&b, "getType", |c| Ok(meta_object(c, |o| o.kind)?.into()));
    void(&b, "setUsed", |c| {
        let v = bool_arg(c, 1);
        meta_object(c, |o| o.used = v)
    });
}

// ---------------------------------------------------------------- BuildingDef

fn building_is_adjacent(g: &IsoMetaGrid, a: usize, b: usize, ignore_z: bool) -> bool {
    let (a, b) = (&g.building_defs[a], &g.building_defs[b]);
    a.rooms.iter().any(|r| {
        let r = &g.room_defs[r.0 as usize];
        b.rooms.iter().any(|o| {
            let o = &g.room_defs[o.0 as usize];
            (ignore_z || r.level == o.level) && room_is_adjacent(r, o)
        })
    })
}

fn register_building_def(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.BuildingDef", Some("java.lang.Object"));
    building_get(&b, "getX", |d| d.x.into());
    building_get(&b, "getY", |d| d.y.into());
    building_get(&b, "getX2", |d| d.x2.into());
    building_get(&b, "getY2", |d| d.y2.into());
    building_get(&b, "getW", |d| (d.x2 - d.x).into());
    building_get(&b, "getH", |d| (d.y2 - d.y).into());
    building_get(&b, "getID", |d| long(d.id));
    building_get(&b, "getIDString", |d| Value::from(d.id.to_string()));
    building_get(&b, "getCellX", |d| fastfloor(d.x as f32 / 256.0).into());
    building_get(&b, "getCellY", |d| fastfloor(d.y as f32 / 256.0).into());
    building_get(&b, "getCellX2", |d| {
        fastfloor((d.x2 - 1) as f32 / 256.0).into()
    });
    building_get(&b, "getCellY2", |d| {
        fastfloor((d.y2 - 1) as f32 / 256.0).into()
    });
    building_get(&b, "getChunkX", |d| fastfloor(d.x as f32 / 8.0).into());
    building_get(&b, "getChunkY", |d| fastfloor(d.y as f32 / 8.0).into());
    building_get(&b, "getKeyId", |d| d.key_id.into());
    building_get(&b, "getKeySpawned", |d| d.key_spawned.into());
    building_get(&b, "isHasBeenVisited", |d| d.has_been_visited.into());
    building_get(&b, "isAlarmed", |d| d.alarmed.into());
    building_get(&b, "isUserDefined", |d| d.user_defined.into());
    building_get(&b, "getRoomsNumber", |d| (d.rooms.len() as i32).into());
    building_get(&b, "isEntirelyEmptyOutside", |d| {
        (d.rooms.is_empty() && !d.emptyoutside.is_empty()).into()
    });
    building_get(&b, "getRooms", |d| {
        list(d.rooms.iter().map(|r| obj(Key::Room(r.0))))
    });
    building_get(&b, "getEmptyOutside", |d| {
        list(d.emptyoutside.iter().map(|r| obj(Key::Room(r.0))))
    });
    get(&b, "getFirstRoom", |c| {
        let i = self_building(c)?;
        let r = grid(|g| {
            g.building_defs[i]
                .rooms
                .first()
                .copied()
                .ok_or_else(|| index_oob(0, 0))
        })?;
        Ok(obj(Key::Room(r.0)))
    });
    get(&b, "getMinLevel", |c| {
        let i = self_building(c)?;
        grid(|g| Ok(g.building_defs[i].get_min_level(&g.room_defs).into()))
    });
    get(&b, "getMaxLevel", |c| {
        let i = self_building(c)?;
        grid(|g| Ok(g.building_defs[i].get_max_level(&g.room_defs).into()))
    });
    void(&b, "resetMinMaxLevel", |c| {
        let i = self_building(c)?;
        grid(|g| {
            g.building_defs[i].reset_min_max_level();
            Ok(())
        })
    });
    let setter = |b: &ClassBuilder, name: &str, f: fn(&mut BuildingDef, &NativeCall<'_>)| {
        void(b, name, move |c| {
            let i = self_building(c)?;
            grid(|g| {
                f(&mut g.building_defs[i], c);
                Ok(())
            })
        });
    };
    setter(&b, "setKeyId", |d, c| d.key_id = int_arg(c, 1));
    setter(&b, "setKeySpawned", |d, c| d.key_spawned = int_arg(c, 1));
    setter(&b, "setAlarmed", |d, c| d.alarmed = bool_arg(c, 1));
    setter(&b, "setUserDefined", |d, c| d.user_defined = bool_arg(c, 1));
    get(&b, "getRoom", |c| {
        let i = self_building(c)?;
        if bool_arg(c, 2) {
            return Err(LuaError::runtime(
                "BuildingDef.getRoom(name, true) needs RoomDef.isKidsRoom, which is not ported",
            ));
        }
        let Some(name) = str_arg(c, 1) else {
            return Ok(Value::Nil);
        };
        let name = name.to_lowercase();
        let r = grid(|g| {
            Ok(g.building_defs[i]
                .rooms
                .iter()
                .copied()
                .find(|r| g.room_defs[r.0 as usize].name.to_lowercase() == name))
        })?;
        Ok(opt(r.map(|r| Key::Room(r.0))))
    });
    get(&b, "containsRoom", |c| {
        let i = self_building(c)?;
        let name = str_arg(c, 1);
        grid(|g| {
            let d = &g.building_defs[i];
            Ok(d.rooms
                .iter()
                .any(|r| name.as_deref() == Some(g.room_defs[r.0 as usize].name.as_str()))
                .into())
        })
    });
    get(&b, "isResidential", |c| {
        let i = self_building(c)?;
        grid(|g| {
            let d = &g.building_defs[i];
            Ok(d.rooms
                .iter()
                .any(|r| g.room_defs[r.0 as usize].name == "bedroom")
                .into())
        })
    });
    get(&b, "isBasement", |c| {
        let i = self_building(c)?;
        grid(|g| {
            let d = &g.building_defs[i];
            Ok(d.rooms
                .first()
                .is_some_and(|r| g.room_defs[r.0 as usize].level < 0)
                .into())
        })
    });
    get(&b, "isAllExplored", |c| {
        let i = self_building(c)?;
        grid(|g| {
            Ok(g.building_defs[i]
                .rooms
                .iter()
                .all(|r| g.room_defs[r.0 as usize].explored)
                .into())
        })
    });
    void(&b, "setAllExplored", |c| {
        let (i, v) = (self_building(c)?, bool_arg(c, 1));
        grid(|g| {
            for r in g.building_defs[i].rooms.clone() {
                g.room_defs[r.0 as usize].explored = v;
            }
            Ok(())
        })
    });
    get(&b, "getArea", |c| {
        let i = self_building(c)?;
        grid(|g| {
            Ok(g.building_defs[i]
                .rooms
                .iter()
                .map(|r| g.room_defs[r.0 as usize].area)
                .sum::<i32>()
                .into())
        })
    });
    void(&b, "CalculateBounds", |c| {
        let i = self_building(c)?;
        grid(|g| {
            g.building_defs[i].CalculateBounds(&g.room_defs);
            Ok(())
        })
    });
    get(&b, "calculateMetaID", |c| {
        let i = self_building(c)?;
        let (cx, cy) = (int_arg(c, 1), int_arg(c, 2));
        grid(|g| {
            Ok(long(g.building_defs[i].calculate_meta_id(
                &g.room_defs,
                cx,
                cy,
            )))
        })
    });
    get(&b, "overlapsChunk", |c| {
        let i = self_building(c)?;
        let (wx, wy) = (int_arg(c, 1), int_arg(c, 2));
        grid(|g| {
            let d = &g.building_defs[i];
            d.overlapped_chunks.as_ref().ok_or_else(npe)?;
            Ok(d.overlaps_chunk(wx, wy).into())
        })
    });
    get(&b, "containsXYZ", |c| {
        let i = self_building(c)?;
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        grid(|g| {
            let d = &g.building_defs[i];
            let inside = x >= d.x && y >= d.y && x < d.x + (d.x2 - d.x) && y < d.y + (d.y2 - d.y);
            Ok((inside
                && d.rooms.iter().any(|r| {
                    let r = &g.room_defs[r.0 as usize];
                    r.level == z && r.intersects(x, y, 1, 1)
                }))
            .into())
        })
    });
    get(&b, "intersects", |c| {
        let i = self_building(c)?;
        let (x, y, w, h, z) = (
            int_arg(c, 1),
            int_arg(c, 2),
            int_arg(c, 3),
            int_arg(c, 4),
            int_arg(c, 5),
        );
        grid(|g| {
            Ok(g.building_defs[i]
                .rooms
                .iter()
                .any(|r| {
                    let r = &g.room_defs[r.0 as usize];
                    r.level == z && r.intersects(x, y, w, h)
                })
                .into())
        })
    });
    get(&b, "isAdjacent", |c| {
        let i = self_building(c)?;
        if let Value::Object(_) = c.arg(1) {
            let o = building_arg(c, 1)?;
            let ignore_z = bool_arg(c, 2);
            grid(|g| Ok(building_is_adjacent(g, i, o, ignore_z).into()))
        } else {
            let (x, y, w, h, z) = (
                int_arg(c, 1),
                int_arg(c, 2),
                int_arg(c, 3),
                int_arg(c, 4),
                int_arg(c, 5),
            );
            grid(|g| {
                Ok(g.building_defs[i]
                    .rooms
                    .iter()
                    .any(|r| {
                        let r = &g.room_defs[r.0 as usize];
                        r.level == z && room_is_adjacent_rect(r, x, y, w, h)
                    })
                    .into())
            })
        }
    });
    get(&b, "overlaps", |c| {
        let (i, o, ignore_z) = (self_building(c)?, building_arg(c, 1)?, bool_arg(c, 2));
        grid(|g| {
            let (a, b) = (&g.building_defs[i], &g.building_defs[o]);
            Ok(a.rooms
                .iter()
                .any(|r| {
                    let r = &g.room_defs[r.0 as usize];
                    b.rooms.iter().any(|x| {
                        let x = &g.room_defs[x.0 as usize];
                        (ignore_z || r.level == x.level) && room_overlaps(r, x)
                    })
                })
                .into())
        })
    });
    get(&b, "getRoofRoomID", |c| {
        let (i, level) = (self_building(c)?, int_arg(c, 1));
        grid(|g| {
            let d = &g.building_defs[i];
            let (cell_x, cell_y) = (d.x / 256, d.y / 256);
            let Some(cell) = g.get_cell_data(cell_x, cell_y) else {
                return Ok(long(-1));
            };
            let Some(index) = cell.buildings.iter().position(|b| b.0 as usize == i) else {
                return Ok(long(-1));
            };
            let room_index = cell.rooms.len() as i32 + index as i32 * 64 + 32 + level;
            Ok(long(room_id::make_id(cell_x, cell_y, room_index)))
        })
    });
    void(&b, "addRoomToCollapseRect", |c| {
        building_arg(c, 0)?;
        Ok(())
    });
    void(&b, "addRoomsOf", |c| {
        let (i, src) = (self_building(c)?, building_arg(c, 1)?);
        grid(|g| {
            let (rooms, outside) = (
                g.building_defs[src].rooms.clone(),
                g.building_defs[src].emptyoutside.clone(),
            );
            for r in &rooms {
                g.room_defs[r.0 as usize].building = Some(BuildingIdx(i as u32));
                g.building_defs[i].rooms.push(*r);
            }
            for r in &outside {
                g.room_defs[r.0 as usize].building = Some(BuildingIdx(i as u32));
                g.building_defs[i].emptyoutside.push(*r);
            }
            g.building_defs[i].reset_min_max_level();
            g.building_defs[i].CalculateBounds(&g.room_defs);
            g.building_defs[src].reset_min_max_level();
            Ok(())
        })
    });
    void(&b, "Dispose", |c| {
        let i = self_building(c)?;
        grid(|g| {
            for r in g.building_defs[i].rooms.clone() {
                room_dispose(&mut g.room_defs[r.0 as usize]);
            }
            let d = &mut g.building_defs[i];
            d.emptyoutside.clear();
            d.rooms.clear();
            d.reset_min_max_level();
            Ok(())
        })
    });
    get(&b, "isFullyStreamedIn", |c| {
        let i = self_building(c)?;
        let chunks = grid(|g| g.building_defs[i].overlapped_chunks.clone().ok_or_else(npe))?;
        cell(|cl| {
            Ok(chunks
                .as_chunks::<2>()
                .0
                .iter()
                .all(|p| cl.chunks.contains_key(&(p[0] as i32, p[1] as i32)))
                .into())
        })
    });
}

// ---------------------------------------------------------------- IsoMetaChunk

/// Cell coordinates and chunk index of absolute chunk `(chx, chy)`.
fn chunk_slot(chx: i32, chy: i32) -> (i32, i32, usize) {
    let cx = fastfloor(chx as f32 / 32.0);
    let cy = fastfloor(chy as f32 / 32.0);
    (cx, cy, ((chy - cy * 32) * 32 + chx - cx * 32) as usize)
}

/// `IsoMetaGrid.getChunkData(chunkX, chunkY)`: creates the chunk if its cell
/// exists.
fn chunk_data(g: &mut IsoMetaGrid, chx: i32, chy: i32) -> Option<(i32, i32)> {
    let (cx, cy, i) = chunk_slot(chx, chy);
    g.meta_chunk_mut(cx, cy, i)?;
    Some((chx, chy))
}

fn chunk_data_from_tile(g: &mut IsoMetaGrid, x: i32, y: i32) -> Option<(i32, i32)> {
    chunk_data(g, fastfloor(x as f32 / 8.0), fastfloor(y as f32 / 8.0))
}

fn self_meta_chunk(c: &NativeCall<'_>) -> LuaResult<(i32, i32)> {
    match key_of(&c.arg(0))? {
        Key::MetaChunk(x, y) => Ok((x, y)),
        _ => Err(wrong(c, 0, "zombie.iso.IsoMetaChunk")),
    }
}

fn meta_chunk_rooms(g: &IsoMetaGrid, chx: i32, chy: i32) -> Vec<RoomIdx> {
    g.get_chunk_data(chx, chy)
        .map(|m| m.rooms().to_vec())
        .unwrap_or_default()
}

/// `IsoMetaChunk.getAssociatedBuildingAt(x, y)`.
fn chunk_associated_building(
    g: &mut IsoMetaGrid,
    chx: i32,
    chy: i32,
    x: i32,
    y: i32,
) -> LuaResult<Option<BuildingIdx>> {
    for r in meta_chunk_rooms(g, chx, chy) {
        let b = g.room_defs[r.0 as usize].building.ok_or_else(npe)?;
        if g.building_defs[b.0 as usize].get_max_level(&g.room_defs) < 0 {
            continue;
        }
        let hit = g.room_defs[r.0 as usize]
            .rects
            .iter()
            .any(|rr| !(rr.x - 1 > x || rr.y - 1 > y || x > rr.get_x2() || y > rr.get_y2()));
        if hit {
            return Ok(Some(b));
        }
    }
    Ok(None)
}

/// `IsoMetaChunk.getRoomsIntersecting(x, y, w, h, result)`.
fn chunk_rooms_intersecting(
    g: &IsoMetaGrid,
    rooms: &[RoomIdx],
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    out: &mut Vec<RoomIdx>,
) -> LuaResult<()> {
    for &r in rooms {
        let room = &g.room_defs[r.0 as usize];
        if room.is_empty_outside() {
            let b = room.building.ok_or_else(npe)?;
            if !g.building_defs[b.0 as usize].rooms.is_empty() {
                continue;
            }
        }
        if out.contains(&r) || !room.intersects(x, y, w, h) {
            continue;
        }
        out.push(r);
    }
    Ok(())
}

fn register_meta_chunk(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.IsoMetaChunk", Some("java.lang.Object"));
    get(&b, "getRoomsSize", |c| {
        let (x, y) = self_meta_chunk(c)?;
        grid(|g| Ok((meta_chunk_rooms(g, x, y).len() as i32).into()))
    });
    get(&b, "getUnadjustedZombieIntensity", |c| {
        let (x, y) = self_meta_chunk(c)?;
        grid(|g| {
            Ok(g.get_chunk_data(x, y)
                .map_or(0, |m| m.get_zombie_intensity() as i32)
                .into())
        })
    });
    void(&b, "setZombieIntensity", |c| {
        let (x, y) = self_meta_chunk(c)?;
        // `byte` parameter: `Double.byteValue()`.
        let v = int_arg(c, 1) as i8 as u8;
        let (cx, cy, i) = chunk_slot(x, y);
        grid(|g| {
            if let Some(m) = g.meta_chunk_mut(cx, cy, i) {
                m.set_zombie_intensity(v);
            }
            Ok(())
        })
    });
    get(&b, "getRoomAt", |c| {
        let (x, y) = self_meta_chunk(c)?;
        let (px, py, pz) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        let r = grid(|g| {
            Ok(g.get_chunk_data(x, y)
                .and_then(|m| m.get_room_at(&g.room_defs, px, py, pz)))
        })?;
        Ok(opt(r.map(|r| Key::Room(r.0))))
    });
    get(&b, "getEmptyOutsideAt", |c| {
        let (x, y) = self_meta_chunk(c)?;
        let (px, py, pz) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        let r = grid(|g| {
            Ok(g.get_chunk_data(x, y)
                .and_then(|m| m.get_empty_outside_at(&g.room_defs, px, py, pz)))
        })?;
        Ok(opt(r.map(|r| Key::Room(r.0))))
    });
    get(&b, "getAssociatedBuildingAt", |c| {
        let (x, y) = self_meta_chunk(c)?;
        let (px, py) = (int_arg(c, 1), int_arg(c, 2));
        let b = grid(|g| chunk_associated_building(g, x, y, px, py))?;
        Ok(opt(b.map(|b| Key::Building(b.0))))
    });
    void(&b, "addRoom", |c| {
        let (x, y) = self_meta_chunk(c)?;
        let r = RoomIdx(room_arg(c, 1)? as u32);
        let (cx, cy, i) = chunk_slot(x, y);
        grid(|g| {
            if let Some(m) = g.meta_chunk_mut(cx, cy, i) {
                m.add_room(r);
            }
            Ok(())
        })
    });
    void(&b, "removeRoom", |c| {
        let (x, y) = self_meta_chunk(c)?;
        let r = RoomIdx(room_arg(c, 1)? as u32);
        let (cx, cy, i) = chunk_slot(x, y);
        grid(|g| {
            if let Some(m) = g.meta_chunk_mut(cx, cy, i) {
                m.remove_room(r);
            }
            Ok(())
        })
    });
    void(&b, "clearRooms", |c| {
        let (x, y) = self_meta_chunk(c)?;
        let (cx, cy, i) = chunk_slot(x, y);
        grid(|g| {
            if let Some(m) = g.meta_chunk_mut(cx, cy, i) {
                while let Some(&r) = m.rooms().last() {
                    m.remove_room(r);
                }
            }
            Ok(())
        })
    });
}

// ---------------------------------------------------------------- IsoMetaCell

fn self_meta_cell(c: &NativeCall<'_>) -> LuaResult<(i32, i32)> {
    match key_of(&c.arg(0))? {
        Key::MetaCell(x, y) => Ok((x, y)),
        _ => Err(wrong(c, 0, "zombie.iso.IsoMetaCell")),
    }
}

fn meta_cell_key(g: &IsoMetaGrid, cx: i32, cy: i32) -> Option<Key> {
    g.get_cell_data(cx, cy)
        .map(|m| Key::MetaCell(m.get_x(), m.get_y()))
}

/// Chunk index argument(s): `(x, y)` or `(i)`. `None` for an out-of-range
/// `(x, y)`; an out-of-range `i` is an `ArrayIndexOutOfBoundsException`.
fn chunk_index_args(c: &NativeCall<'_>) -> LuaResult<Option<(i32, i32)>> {
    if c.nargs() >= 3 {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        Ok((x >= 0 && y >= 0 && x < 32 && y < 32).then_some((x, y)))
    } else {
        let i = int_arg(c, 1);
        if !(0..1024).contains(&i) {
            return Err(java_err(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {i} out of bounds for length 1024"),
            ));
        }
        Ok(Some((i % 32, i / 32)))
    }
}

fn register_meta_cell(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.IsoMetaCell", Some("java.lang.Object"));
    get(&b, "getX", |c| Ok(self_meta_cell(c)?.0.into()));
    get(&b, "getY", |c| Ok(self_meta_cell(c)?.1.into()));
    get(&b, "hasChunk", |c| {
        let (cx, cy) = self_meta_cell(c)?;
        let Some((x, y)) = chunk_index_args(c)? else {
            return Ok(false.into());
        };
        grid(|g| {
            Ok(g.get_cell_data(cx, cy)
                .is_some_and(|m| m.has_chunk(x, y))
                .into())
        })
    });
    get(&b, "getChunk", |c| {
        let (cx, cy) = self_meta_cell(c)?;
        let Some((x, y)) = chunk_index_args(c)? else {
            return Ok(Value::Nil);
        };
        let made = grid(|g| Ok(g.meta_chunk_mut(cx, cy, (y * 32 + x) as usize).is_some()))?;
        Ok(opt(made.then_some(Key::MetaChunk(cx * 32 + x, cy * 32 + y))))
    });
    get(&b, "getBuildingCount", |c| {
        let (cx, cy) = self_meta_cell(c)?;
        let exclude = bool_arg(c, 1);
        grid(|g| {
            let m = g.get_cell_data(cx, cy).ok_or_else(npe)?;
            let n = m
                .buildings
                .iter()
                .filter(|b| !exclude || !g.building_defs[b.0 as usize].user_defined)
                .count();
            Ok((n as i32).into())
        })
    });
    get(&b, "getRoomCount", |c| {
        let (cx, cy) = self_meta_cell(c)?;
        let exclude = bool_arg(c, 1);
        grid(|g| {
            let m = g.get_cell_data(cx, cy).ok_or_else(npe)?;
            let n = m
                .room_list
                .iter()
                .filter(|r| !exclude || !g.room_defs[r.0 as usize].user_defined)
                .count();
            Ok((n as i32).into())
        })
    });
}

// ---------------------------------------------------------------- IsoMetaGrid

fn self_meta_grid(c: &NativeCall<'_>) -> LuaResult<()> {
    match key_of(&c.arg(0))? {
        Key::MetaGrid => Ok(()),
        _ => Err(wrong(c, 0, "zombie.iso.IsoMetaGrid")),
    }
}

/// `IsoDirections` offsets, by the original's search order in
/// `IsoMetaGrid.getAssociatedBuildingAt`.
const DIR_E: (i32, i32) = (1, 0);
const DIR_W: (i32, i32) = (-1, 0);
const DIR_N: (i32, i32) = (0, -1);
const DIR_NW: (i32, i32) = (-1, -1);
const DIR_SE: (i32, i32) = (1, 1);
const DIR_SW: (i32, i32) = (-1, 1);
const DIR_NE: (i32, i32) = (1, -1);

/// `IsoMetaGrid.getAssociatedBuildingAt(x, y)`.
fn grid_associated_building(g: &mut IsoMetaGrid, x: i32, y: i32) -> LuaResult<Option<BuildingIdx>> {
    let at = |g: &mut IsoMetaGrid, (dx, dy): (i32, i32)| -> LuaResult<Option<BuildingIdx>> {
        match chunk_data_from_tile(g, x + dx, y + dy) {
            Some((chx, chy)) => chunk_associated_building(g, chx, chy, x, y),
            None => Ok(None),
        }
    };
    if let Some(b) = at(g, (0, 0))? {
        return Ok(Some(b));
    }
    let xmod = pz_world::java::coordmodulo(x, 8);
    let ymod = pz_world::java::coordmodulo(y, 8);
    let tries = [
        (xmod == 7, DIR_E),
        (xmod == 0, DIR_W),
        (ymod == 0, DIR_N),
        (xmod == 0 || ymod == 0, DIR_NW),
        (xmod == 7 || ymod == 7, DIR_SE),
        (xmod == 0 || ymod == 7, DIR_SW),
        (xmod == 7 || ymod == 0, DIR_NE),
    ];
    for (cond, dir) in tries {
        if cond {
            if let Some(b) = at(g, dir)? {
                return Ok(Some(b));
            }
        }
    }
    Ok(None)
}

fn grid_building_at(g: &IsoMetaGrid, pred: impl Fn(&BuildingDef) -> bool) -> Option<Key> {
    g.buildings
        .iter()
        .find(|b| pred(&g.building_defs[b.0 as usize]))
        .map(|b| Key::Building(b.0))
}

fn register_meta_grid(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.IsoMetaGrid", Some("java.lang.Object"));
    let field = |b: &ClassBuilder, name: &str, f: fn(&IsoMetaGrid) -> i32| {
        get(b, name, move |c| {
            self_meta_grid(c)?;
            grid(|g| Ok(f(g).into()))
        });
    };
    field(&b, "getMinX", |g| g.min_x);
    field(&b, "getMinY", |g| g.min_y);
    field(&b, "getMaxX", |g| g.max_x);
    field(&b, "getMaxY", |g| g.max_y);
    field(&b, "getWidth", |g| g.width);
    field(&b, "getHeight", |g| g.height);
    field(&b, "gridX", |g| g.width);
    field(&b, "gridY", |g| g.height);
    // `grid[x][y]` with grid-relative indices.
    let relative = |c: &NativeCall<'_>, g: &IsoMetaGrid| -> LuaResult<Option<Key>> {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        if x < 0 || y < 0 || x >= g.width || y >= g.height {
            return Err(java_err(
                "java.lang.ArrayIndexOutOfBoundsException",
                "grid index out of bounds",
            ));
        }
        Ok(g.get_cell(x, y)
            .map(|m| Key::MetaCell(m.get_x(), m.get_y())))
    };
    get(&b, "hasCell", move |c| {
        self_meta_grid(c)?;
        grid(|g| Ok(relative(c, g)?.is_some().into()))
    });
    for name in ["getCell", "getCellDataAbs"] {
        get(&b, name, move |c| {
            self_meta_grid(c)?;
            Ok(opt(grid(|g| relative(c, g))?))
        });
    }
    get(&b, "getCellData", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        Ok(opt(grid(|g| Ok(meta_cell_key(g, x, y)))?))
    });
    get(&b, "getMetaGridFromTile", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1) / 256, int_arg(c, 2) / 256);
        Ok(opt(grid(|g| Ok(meta_cell_key(g, x, y)))?))
    });
    get(&b, "getChunkData", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let k = grid(|g| Ok(chunk_data(g, x, y)))?;
        Ok(opt(k.map(|(x, y)| Key::MetaChunk(x, y))))
    });
    get(&b, "getChunkDataFromTile", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let k = grid(|g| Ok(chunk_data_from_tile(g, x, y)))?;
        Ok(opt(k.map(|(x, y)| Key::MetaChunk(x, y))))
    });
    get(&b, "getRoomAt", |c| {
        self_meta_grid(c)?;
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        let r = grid(|g| {
            Ok(chunk_data_from_tile(g, x, y).and_then(|(chx, chy)| {
                g.get_chunk_data(chx, chy)?
                    .get_room_at(&g.room_defs, x, y, z)
            }))
        })?;
        Ok(opt(r.map(|r| Key::Room(r.0))))
    });
    get(&b, "getEmptyOutsideAt", |c| {
        self_meta_grid(c)?;
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        let r = grid(|g| {
            Ok(chunk_data_from_tile(g, x, y).and_then(|(chx, chy)| {
                g.get_chunk_data(chx, chy)?
                    .get_empty_outside_at(&g.room_defs, x, y, z)
            }))
        })?;
        Ok(opt(r.map(|r| Key::Room(r.0))))
    });
    get(&b, "getRoomDefByID", |c| {
        self_meta_grid(c)?;
        let id = long_arg(c, 1);
        let r = grid(|g| {
            Ok(
                g.get_cell_data(room_id::get_cell_x(id), room_id::get_cell_y(id))
                    .and_then(|m| m.rooms.get(&id).copied()),
            )
        })?;
        Ok(opt(r.map(|r| Key::Room(r.0))))
    });
    get(&b, "isValidSquare", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        grid(|g| Ok(g.is_valid_square(x, y).into()))
    });
    get(&b, "isValidChunk", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1).wrapping_mul(8), int_arg(c, 2).wrapping_mul(8));
        grid(|g| Ok(g.is_valid_square(x, y).into()))
    });
    get(&b, "getBuildings", |c| {
        self_meta_grid(c)?;
        let v = grid(|g| Ok(g.buildings.clone()))?;
        Ok(list(v.into_iter().map(|b| obj(Key::Building(b.0)))))
    });
    get(&b, "getBuildingAt", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let k = grid(|g| {
            Ok(grid_building_at(g, |d| {
                !(d.x > x || d.y > y || d.x2 - d.x <= x - d.x || d.y2 - d.y <= y - d.y)
            }))
        })?;
        Ok(opt(k))
    });
    get(&b, "getBuildingAtRelax", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let k = grid(|g| {
            Ok(grid_building_at(g, |d| {
                !(d.x > x + 1 || d.y > y + 1 || d.x2 - d.x < x - d.x || d.y2 - d.y < y - d.y)
            }))
        })?;
        Ok(opt(k))
    });
    get(&b, "getAssociatedBuildingAt", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let b = grid(|g| grid_associated_building(g, x, y))?;
        Ok(opt(b.map(|b| Key::Building(b.0))))
    });
    get(&b, "countRoomsIntersecting", |c| {
        self_meta_grid(c)?;
        let (x, y, w, h) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        grid(|g| {
            let mut out = Vec::new();
            // The original bounds the cell loop by the grid's width/height.
            for cy in y / 256..=(y + g.height) / 256 {
                for cx in x / 256..=(x + g.width) / 256 {
                    if cx < g.min_x || cx > g.max_x || cy < g.min_y || cy > g.max_y {
                        continue;
                    }
                    let Some(cell) = g.get_cell(cx - g.min_x, cy - g.min_y) else {
                        continue;
                    };
                    let mut xmax = (x + w) / 8;
                    if (x + w) % 8 == 0 {
                        xmax -= 1;
                    }
                    let mut ymax = (y + h) / 8;
                    if (y + h) % 8 == 0 {
                        ymax -= 1;
                    }
                    let (cell_x, cell_y) = (cell.get_x() * 256, cell.get_y() * 256);
                    for yy in y / 8..=ymax {
                        for xx in x / 8..=xmax {
                            if xx < cell_x / 8
                                || xx >= (cell_x + 256) / 8
                                || yy < cell_y / 8
                                || yy >= (cell_y + 256) / 8
                            {
                                continue;
                            }
                            let Some(m) = cell.chunk(xx - cell_x / 8, yy - cell_y / 8) else {
                                continue;
                            };
                            chunk_rooms_intersecting(g, m.rooms(), x, y, w, h, &mut out)?;
                        }
                    }
                }
            }
            Ok((out.len() as i32).into())
        })
    });
    get(&b, "isChunkLoaded", |c| {
        self_meta_grid(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        cell(|cl| Ok(cl.chunks.contains_key(&(x, y)).into()))
    });
    // Stubs in the original that return null.
    for name in [
        "getRandomIndoorCoord",
        "getRandomRoomBetweenRange",
        "getRandomRoomNotInRange",
    ] {
        get(&b, name, |c| {
            self_meta_grid(c)?;
            Ok(Value::Nil)
        });
    }
}

// ---------------------------------------------------------------- IsoCell

fn self_cell(c: &NativeCall<'_>) -> LuaResult<()> {
    match key_of(&c.arg(0))? {
        Key::Cell => Ok(()),
        _ => Err(wrong(c, 0, "zombie.iso.IsoCell")),
    }
}

/// The canonical Lua handle for the current world cell, or `nil` before
/// a cell is installed and after it is unloaded.
pub fn current_cell() -> Value {
    let has = WORLD.with(|w| w.borrow().cell.is_some());
    opt(has.then_some(Key::Cell))
}

fn register_iso_cell(thread: &mut Thread, drag_event: Option<Rc<RefCell<crate::events::Event>>>) {
    let b = expose_class(thread, "zombie.iso.IsoCell", Some("java.lang.Object"));
    cell_drag::register(&b, drag_event);
    for (name, remove) in [
        ("addToProcessItems", false),
        ("addToProcessItemsRemove", true),
    ] {
        void(&b, name, move |c| {
            self_cell(c)?;
            let items = crate::bind_process_items::argument_values(&c.arg(1));
            let client = crate::bind_core::shared_core().borrow().is_client;
            cell(|cl| {
                if !remove && client {
                    return Ok(());
                }
                for value in items {
                    // The original casts each ArrayList element inside its
                    // loop. A later bad element preserves earlier changes.
                    let item = crate::bind_process_items::decode(&value)?;
                    if remove {
                        cl.process_items.queue_remove(item);
                    } else {
                        cl.process_items.add(item, client);
                    }
                }
                Ok(())
            })
        });
    }
    for (name, remove) in [("getProcessItems", false), ("getProcessItemsRemove", true)] {
        get(&b, name, move |c| {
            self_cell(c)?;
            let entries = cell(|cl| {
                Ok(if remove {
                    cl.process_items.remove.clone()
                } else {
                    cl.process_items.items.clone()
                })
            })?;
            Ok(crate::bind_process_items::queue_value(&entries, remove))
        });
    }
    static_get(&b, "getInstance", |_| Ok(current_cell()));
    static_get(&b, "getCellSizeInChunks", |_| Ok(32.into()));
    static_get(&b, "getCellSizeInSquares", |_| Ok(256.into()));
    static_get(&b, "getRComponent", |c| Ok((int_arg(c, 0) & 0xFF).into()));
    static_get(&b, "getGComponent", |c| {
        Ok(((int_arg(c, 0) & 0xFF00) >> 8).into())
    });
    static_get(&b, "getBComponent", |c| {
        Ok(((int_arg(c, 0) & 0xFF0000) >> 16).into())
    });
    static_get(&b, "toIntColor", |c| {
        let ch = |i| (num_arg(c, i) as f32 * 255.0) as i32;
        Ok((ch(0) | ch(1) << 8 | ch(2) << 16 | ch(3) << 24).into())
    });
    get(&b, "getMaxFloors", |c| {
        self_cell(c)?;
        Ok(32.into())
    });
    get(&b, "getGridSquare", |c| {
        self_cell(c)?;
        let (x, y, z) = (
            fastfloor_d(num_arg(c, 1)),
            fastfloor_d(num_arg(c, 2)),
            fastfloor_d(num_arg(c, 3)),
        );
        Ok(opt(square_key(x, y, z)))
    });
    get(&b, "getChunk", |c| {
        self_cell(c)?;
        let (wx, wy) = (int_arg(c, 1), int_arg(c, 2));
        cell(|cl| {
            Ok(opt(cl
                .chunks
                .contains_key(&(wx, wy))
                .then_some(Key::Chunk(wx, wy))))
        })
    });
    get(&b, "getChunkForGridSquare", |c| {
        self_cell(c)?;
        let (wx, wy) = (int_arg(c, 1).div_euclid(8), int_arg(c, 2).div_euclid(8));
        cell(|cl| {
            Ok(opt(cl
                .chunks
                .contains_key(&(wx, wy))
                .then_some(Key::Chunk(wx, wy))))
        })
    });
}

// ---------------------------------------------------------------- IsoChunk

fn chunk<R>(c: &NativeCall<'_>, f: impl FnOnce(&pz_world::IsoChunk) -> R) -> LuaResult<R> {
    match key_of(&c.arg(0))? {
        Key::Chunk(wx, wy) => cell(|cl| {
            let ch = cl
                .chunks
                .get(&(wx, wy))
                .ok_or_else(|| LuaError::runtime("IsoChunk is no longer loaded"))?;
            Ok(f(ch))
        }),
        _ => Err(wrong(c, 0, "zombie.iso.IsoChunk")),
    }
}

fn register_iso_chunk(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.IsoChunk", Some("java.lang.Object"));
    get(&b, "getMinLevel", |c| {
        Ok(chunk(c, |ch| ch.min_level)?.into())
    });
    get(&b, "getMaxLevel", |c| {
        Ok(chunk(c, |ch| ch.max_level)?.into())
    });
    get(&b, "isValidLevel", |c| {
        let z = int_arg(c, 1);
        Ok(chunk(c, |ch| ch.is_valid_level(z))?.into())
    });
    get(&b, "squaresIndexOfLevel", |c| {
        let z = int_arg(c, 1);
        Ok(chunk(c, |ch| z - ch.min_level)?.into())
    });
    get(&b, "containsPoint", |c| {
        let (x, y) = (num_arg(c, 1) as f32, num_arg(c, 2) as f32);
        let r = chunk(c, |ch| {
            x >= (ch.wx * 8) as f32
                && x < ((ch.wx + 1) * 8) as f32
                && y >= (ch.wy * 8) as f32
                && y < ((ch.wy + 1) * 8) as f32
        })?;
        Ok(r.into())
    });
    get(&b, "getGridSquare", |c| {
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        let k = chunk(c, |ch| {
            ch.get_grid_square(x, y, z).map(|s| {
                let s = s.borrow();
                Key::Square(s.x, s.y, s.z)
            })
        })?;
        Ok(opt(k))
    });
}

// ---------------------------------------------------------------- IsoGridSquare

fn self_square(c: &NativeCall<'_>) -> LuaResult<(i32, i32, i32)> {
    square_arg(c, 0)
}

fn square_room_id(x: i32, y: i32, z: i32) -> LuaResult<i64> {
    cell(|cl| {
        cl.get_grid_square(x, y, z)
            .map(|s| s.borrow().room_id)
            .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))
    })
}

/// `getRoomDef()`: the room of `roomID` through `IsoMetaGrid.getRoomByID`.
fn square_room(x: i32, y: i32, z: i32) -> LuaResult<Option<usize>> {
    let id = square_room_id(x, y, z)?;
    if id == -1 {
        return Ok(None);
    }
    grid(|g| {
        Ok(
            g.get_cell_data(room_id::get_cell_x(id), room_id::get_cell_y(id))
                .and_then(|m| m.rooms.get(&id))
                .map(|r| r.0 as usize),
        )
    })
}

fn square_building(x: i32, y: i32, z: i32) -> LuaResult<Option<usize>> {
    match square_room(x, y, z)? {
        Some(r) => grid(|g| Ok(g.room_defs[r].building.map(|b| b.0 as usize))),
        None => Ok(None),
    }
}

fn register_grid_square(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.IsoGridSquare", Some("java.lang.Object"));
    reachability::register(&b);
    square_queries::register(&b);
    power::register(&b);
    static_get(&b, "getMatrixBit", |c| {
        let bit = |i| int_arg(c, i) as i8 as i32;
        let n = bit(1) + bit(2) * 3 + bit(3) * 9;
        Ok((int_arg(c, 0).wrapping_shr(n as u32) & 1 != 0).into())
    });
    static_get(&b, "setMatrixBit", |c| {
        let bit = |i| int_arg(c, i) as i8 as i32;
        let n = (bit(1) + bit(2) * 3 + bit(3) * 9) as u32;
        let m = int_arg(c, 0);
        Ok(if bool_arg(c, 4) {
            m | 1i32.wrapping_shl(n)
        } else {
            m & !1i32.wrapping_shl(n)
        }
        .into())
    });
    get(&b, "getX", |c| Ok(self_square(c)?.0.into()));
    get(&b, "getY", |c| Ok(self_square(c)?.1.into()));
    get(&b, "getZ", |c| Ok(self_square(c)?.2.into()));
    get(&b, "getCenterX", |c| {
        Ok(Value::Num((self_square(c)?.0 as f32 + 0.5) as f64))
    });
    get(&b, "getCenterY", |c| {
        Ok(Value::Num((self_square(c)?.1 as f32 + 0.5) as f64))
    });
    get(&b, "getRoomID", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(long(square_room_id(x, y, z)?))
    });
    get(&b, "getRoomIDString", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(square_room_id(x, y, z)?.to_string().into())
    });
    get(&b, "getRoomDef", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(opt(square_room(x, y, z)?.map(|r| Key::Room(r as u32))))
    });
    get(&b, "hasRoomDef", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(square_room(x, y, z)?.is_some().into())
    });
    get(&b, "getBuildingDef", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(opt(
            square_building(x, y, z)?.map(|b| Key::Building(b as u32))
        ))
    });
    get(&b, "isUserDefinedRoom", |c| {
        let (x, y, z) = self_square(c)?;
        match square_room(x, y, z)? {
            Some(r) => grid(|g| Ok(g.room_defs[r].user_defined.into())),
            None => Ok(false.into()),
        }
    });
    get(&b, "isUserDefinedBuilding", |c| {
        let (x, y, z) = self_square(c)?;
        match square_building(x, y, z)? {
            Some(b) => grid(|g| Ok(g.building_defs[b].user_defined.into())),
            None => Ok(false.into()),
        }
    });
    get(&b, "getChunk", |c| {
        let (x, y, _) = self_square(c)?;
        let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
        cell(|cl| {
            Ok(opt(cl
                .chunks
                .contains_key(&(wx, wy))
                .then_some(Key::Chunk(wx, wy))))
        })
    });
    get(&b, "getCell", |c| {
        self_square(c)?;
        Ok(current_cell())
    });
    // Neighbour links (`n`, `s`, `w`, `e`) and the squares above/below,
    // resolved through the current cell.
    let neighbour = |b: &ClassBuilder, name: &str, (dx, dy, dz): (i32, i32, i32)| {
        get(b, name, move |c| {
            let (x, y, z) = self_square(c)?;
            Ok(opt(square_key(x + dx, y + dy, z + dz)))
        });
    };
    neighbour(&b, "getN", (0, -1, 0));
    neighbour(&b, "getS", (0, 1, 0));
    neighbour(&b, "getW", (-1, 0, 0));
    neighbour(&b, "getE", (1, 0, 0));
    neighbour(&b, "getSquareAbove", (0, 0, 1));
    neighbour(&b, "getSquareBelow", (0, 0, -1));
    get(&b, "isInsideRectangle", |c| {
        let (sx, sy, _) = self_square(c)?;
        let (x, y, w, h) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        Ok((sx >= x && sy >= y && sx < x + w && sy < y + h).into())
    });
    let compare = |b: &ClassBuilder, name: &str, f: fn((i32, i32), (i32, i32)) -> bool| {
        get(b, name, move |c| {
            let (x, y, _) = self_square(c)?;
            let (ox, oy, _) = square_arg(c, 1)?;
            Ok(f((x, y), (ox, oy)).into())
        });
    };
    compare(&b, "isEastOf", |a, o| a.0 > o.0);
    compare(&b, "isWestOf", |a, o| a.0 < o.0);
    compare(&b, "isSouthOf", |a, o| a.1 > o.1);
    compare(&b, "isNorthOf", |a, o| a.1 < o.1);
    compare(&b, "isDiagonalTo", |a, o| o.0 != a.0 && o.1 != a.1);
}

// ---------------------------------------------------------------- IsoWorld

fn self_world(c: &NativeCall<'_>) -> LuaResult<()> {
    match key_of(&c.arg(0))? {
        Key::World => Ok(()),
        _ => Err(wrong(c, 0, "zombie.iso.IsoWorld")),
    }
}

fn register_iso_world(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.IsoWorld", Some("java.lang.Object"));
    b.field("instance", obj(Key::World));
    get(&b, "getCell", |c| {
        self_world(c)?;
        Ok(current_cell())
    });
    get(&b, "getMetaGrid", |c| {
        self_world(c)?;
        let has = WORLD.with(|w| w.borrow().meta_grid.is_some());
        Ok(opt(has.then_some(Key::MetaGrid)))
    });
    get(&b, "getMetaChunk", |c| {
        self_world(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let k = grid(|g| Ok(chunk_data(g, x, y)))?;
        Ok(opt(k.map(|(x, y)| Key::MetaChunk(x, y))))
    });
    get(&b, "getMetaChunkFromTile", |c| {
        self_world(c)?;
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let k = grid(|g| Ok(chunk_data_from_tile(g, x, y)))?;
        Ok(opt(k.map(|(x, y)| Key::MetaChunk(x, y))))
    });
    get(&b, "isValidSquare", |c| {
        self_world(c)?;
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        if !(-32..=31).contains(&z) {
            return Ok(false.into());
        }
        grid(|g| Ok(g.is_valid_square(x, y).into()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_fs::GameFs;
    use pz_world::{CreateParams, GameRand, IsoChunk, IsoGridSquare, WorldGenBounds};
    use std::path::{Path, PathBuf};

    struct Zero;
    impl GameRand for Zero {
        fn next(&mut self, _max: i32) -> i32 {
            0
        }
    }

    fn ints(v: &mut Vec<u8>, xs: &[i32]) {
        for x in xs {
            v.extend(x.to_le_bytes());
        }
    }

    /// Hand-built map: cell (1, 1) with a two-rect kitchen and an
    /// emptyoutside on level 0, a bedroom on level 1, all in one building.
    fn write_map(root: &Path) {
        let dir = root.join("media/maps/Test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("map.info"), "title=Test\nfixed2x=true\n").unwrap();
        let mut h = b"LOTH".to_vec();
        ints(&mut h, &[1, 0, 8, 8, 0, 1, 3]);
        h.extend(b"kitchen\n");
        ints(&mut h, &[0, 2, 3, 4, 2, 2, 5, 4, 1, 1, 1, 7, 4, 5]);
        h.extend(b"bedroom\n");
        ints(&mut h, &[1, 1, 3, 4, 3, 3, 0]);
        h.extend(b"emptyoutside\n");
        ints(&mut h, &[0, 1, 0, 0, 2, 2, 0]);
        ints(&mut h, &[1, 3, 0, 1, 2]);
        h.extend((0..1024).map(|i| (i % 5) as u8));
        std::fs::write(dir.join("1_1.lotheader"), h).unwrap();
    }

    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn load_world(tag: &str) -> TempDir {
        let root = std::env::temp_dir().join(format!("pz-lua-world-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write_map(&root);
        let fs = GameFs::new(&root);
        let grid = IsoMetaGrid::create(
            &fs,
            CreateParams {
                game_map: "Test",
                world_gen: WorldGenBounds::default(),
                rand: &mut Zero,
                voronois: Vec::new(),
            },
        )
        .unwrap();
        let kitchen = grid.get_room_at(259, 260, 0).unwrap().id;
        set_meta_grid(Some(grid));
        let mut cell = IsoCell::default();
        let mut ch = IsoChunk::new(32, 32);
        let mut sq = IsoGridSquare::new(259, 260, 0);
        sq.room_id = kitchen;
        ch.set_square(3, 4, 0, Some((sq).into()));
        ch.set_square(4, 4, 0, Some((IsoGridSquare::new(260, 260, 0)).into()));
        ch.set_square(3, 4, 1, Some((IsoGridSquare::new(259, 260, 1)).into()));
        cell.chunks.insert((32, 32), ch);
        set_cell(Some(cell));
        TempDir(root)
    }

    fn thread() -> Thread {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        t
    }

    fn run(t: &mut Thread, src: &str) -> Vec<Value> {
        let prelude = "local n = 0\nlocal function check(c, m) n = n + 1 \
                       if not c then error((m or 'check failed') .. ' #' .. n) end end\n";
        let f = pz_kahlua::stdlib::load_string(
            &format!("{prelude}{src}"),
            Some("test"),
            t.globals().clone(),
        )
        .expect("compile");
        t.call(&f, &[])
            .unwrap_or_else(|e| panic!("runtime error: {e}"))
    }

    #[test]
    fn square_coords_of_lua_values() {
        let _dir = load_world("sqcoords");
        let mut t = thread();
        let out = run(
            &mut t,
            "local w = IsoWorld.instance return w:getCell():getGridSquare(259, 260, 1), w:getMetaGrid()",
        );
        assert_eq!(square_coords(&out[0]), Some((259, 260, 1)));
        assert_eq!(square_coords(&out[1]), None);
        assert_eq!(square_coords(&Value::Nil), None);
        // A square from world data that was replaced since.
        set_cell(Some(IsoCell::default()));
        assert_eq!(square_coords(&out[0]), None);
    }

    #[test]
    fn rooms_buildings_and_meta_objects() {
        let _dir = load_world("rooms");
        let mut t = thread();
        run(
            &mut t,
            r#"
            local grid = IsoWorld.instance:getMetaGrid()
            check(grid:getMinX() == 1 and grid:getMaxY() == 1 and grid:getWidth() == 1)
            local k = grid:getRoomAt(259, 260, 0)
            check(k:getName() == "kitchen")
            check(k == grid:getRoomAt(261, 260, 0), "same RoomDef value")
            check(grid:getRoomAt(259, 260, 1):getName() == "bedroom")
            check(grid:getRoomAt(256, 256, 0) == nil)
            check(grid:getEmptyOutsideAt(256, 256, 0):isEmptyOutside())
            check(k:getX() == 259 and k:getY() == 260 and k:getX2() == 262 and k:getY2() == 262)
            check(k:getW() == 3 and k:getH() == 2 and k:getZ() == 0 and k:getArea() == 5)
            check(k:contains(261, 260) and not k:contains(261, 261))
            check(k:isInside(259, 260, 0) and not k:isInside(259, 260, 1))
            check(k:intersects(261, 261, 1, 1) == false)
            check(k:isAdjacent(258, 260, 1, 1))
            check(k:getAreaOverlapping(259, 260, 1, 1) == 0.20000000298023224)
            check(k:getID() == grid:getRoomDefByID(k:getID()):getID())
            check(k:getIDString() == "281479271677952")
            local rects = k:getRects()
            check(rects:size() == 2 and rects:get(1):getX() == 261 and rects:get(1):getW() == 1)
            check(k:getRoomRect(261, 261, 0) == rects:get(0) and k:getRoomRect(262, 261, 0) == rects:get(1))
            local o = k:getObjects():get(0)
            check(o:getType() == 7 and o:getX() == 1 and o:getY() == 1 and o:getRoom() == k)
            check(not o:getUsed()); o:setUsed(true); check(k:getMetaObjects():get(0):getUsed())
            k:setName("pantry"); check(grid:getRoomAt(259, 260, 0):getName() == "pantry")

            local b = k:getBuilding()
            check(b == grid:getBuildings():get(0))
            check(b:getRoomsNumber() == 2 and b:getEmptyOutside():size() == 1)
            check(b:getMinLevel() == 0 and b:getMaxLevel() == 1)
            check(b:getRoom("BEDROOM"):getName() == "bedroom" and b:getRoom("nope") == nil)
            check(b:containsRoom("bedroom") and b:isResidential() and not b:isBasement())
            check(b:getX() == 256 and b:getY() == 256 and b:getCellX() == 1 and b:getChunkX() == 32)
            check(b:containsXYZ(260, 261, 1) and not b:containsXYZ(256, 256, 0))
            check(b:overlapsChunk(32, 32) and not b:overlapsChunk(40, 40))
            check(b:isFullyStreamedIn() == false)
            check(grid:getBuildingAt(257, 257) == b and grid:getBuildingAt(300, 300) == nil)
            check(grid:getAssociatedBuildingAt(258, 261) == b)
            check(grid:countRoomsIntersecting(256, 256, 8, 8) == 2)
            check(not b:isAllExplored()); b:setAllExplored(true); check(k:isExplored())
            k:offset(1, 0); check(k:getX() == 260 and rects:get(0):getX() == 260)
            check(grid:getRandomIndoorCoord() == nil)
            "#,
        );
    }

    #[test]
    fn meta_cells_and_chunks() {
        let _dir = load_world("meta");
        let mut t = thread();
        run(
            &mut t,
            r#"
            local grid = IsoWorld.instance:getMetaGrid()
            local cell = grid:getCellData(1, 1)
            check(cell == grid:getCell(0, 0) and cell:getX() == 1 and cell:getY() == 1)
            check(grid:getCellData(0, 0) == nil and grid:hasCell(0, 0))
            check(cell:getBuildingCount() == 1 and cell:getRoomCount() == 3)
            check(cell:hasChunk(0, 0) and not cell:hasChunk(5, 5) and not cell:hasChunk(40, 0))
            local mc = IsoWorld.instance:getMetaChunk(37, 37)
            check(mc ~= nil and cell:hasChunk(5, 5), "getChunkData creates the chunk")
            check(mc == cell:getChunk(5, 5) and mc == cell:getChunk(5 * 32 + 5))
            check(mc:getRoomsSize() == 0)
            local c0 = IsoWorld.instance:getMetaChunkFromTile(259, 260)
            check(c0:getRoomsSize() == 3 and c0:getRoomAt(259, 260, 1):getName() == "bedroom")
            check(c0:getUnadjustedZombieIntensity() == 0)
            c0:setZombieIntensity(200); check(c0:getUnadjustedZombieIntensity() == 200)
            check(c0:getAssociatedBuildingAt(258, 259) == grid:getBuildings():get(0))
            local eo = c0:getEmptyOutsideAt(257, 257, 0)
            c0:removeRoom(eo); check(c0:getRoomsSize() == 2)
            c0:addRoom(eo); check(c0:getEmptyOutsideAt(257, 257, 0) == eo)
            check(IsoWorld.instance:isValidSquare(300, 300, 0) and not IsoWorld.instance:isValidSquare(300, 300, 40))
            check(grid:isValidChunk(32, 32) and not grid:isValidChunk(0, 0))
            "#,
        );
    }

    #[test]
    fn cell_chunks_and_squares() {
        let _dir = load_world("squares");
        let mut t = thread();
        run(
            &mut t,
            r#"
            local cell = IsoWorld.instance:getCell()
            check(cell == IsoCell.getInstance() and cell:getMaxFloors() == 32)
            local sq = cell:getGridSquare(259.7, 260.2, 0)
            check(sq == cell:getGridSquare(259, 260, 0) and sq:getX() == 259 and sq:getZ() == 0)
            check(cell:getGridSquare(250, 250, 0) == nil)
            local k = sq:getRoomDef()
            check(k:getName() == "kitchen" and sq:hasRoomDef() and sq:getRoomID() == k:getID())
            check(sq:getBuildingDef() == k:getBuilding() and not sq:isUserDefinedRoom())
            local e = sq:getE()
            check(e:getX() == 260 and e:getW() == sq and e:getRoomDef() == nil and e:getRoomID() == -1)
            check(sq:getN() == nil and sq:getSquareAbove():getZ() == 1 and sq:getSquareBelow() == nil)
            check(e:isEastOf(sq) and sq:isWestOf(e) and not sq:isDiagonalTo(e))
            check(sq:isInsideRectangle(259, 260, 1, 1) and sq:getCenterX() == 259.5)
            local ch = sq:getChunk()
            check(ch == cell:getChunk(32, 32) and ch == cell:getChunkForGridSquare(263, 263, 0))
            check(ch:getGridSquare(3, 4, 0) == sq and ch:getMinLevel() == 0 and ch:getMaxLevel() == 1)
            check(ch:containsPoint(256.5, 263.9) and not ch:containsPoint(264, 260))
            check(k:getAreaOverlapping(ch) == 1)
            check(IsoGridSquare.setMatrixBit(0, 1, 1, 1, true) == 8192)
            check(IsoGridSquare.getMatrixBit(8192, 1, 1, 1) and not IsoGridSquare.getMatrixBit(8192, 0, 1, 1))
            check(IsoCell.toIntColor(1, 0, 0, 1) == -16776961 and IsoCell.getBComponent(0x123456) == 0x12)
            "#,
        );
    }

    #[test]
    fn room_rects_lists_and_stale_handles() {
        let _dir = load_world("misc");
        let mut t = thread();
        let r = run(
            &mut t,
            r#"
            local r = RoomRect.new(1, 2, 3, 4)
            check(RoomDef.RoomRect == RoomRect and r:getX2() == 4 and r:getY2() == 6)
            check(r:contains(1, 2) and not r:contains(4, 2))
            check(r:set(0, 0, 1, 1) == r and r:getW() == 1)
            local ok, err = pcall(function() return IsoWorld.instance:getMetaGrid():getBuildings():get(5) end)
            check(not ok)
            return IsoWorld.instance:getMetaGrid():getRoomAt(259, 260, 0)
            "#,
        );
        set_meta_grid(None);
        let ok = run(
            &mut t,
            "local k = ...; return IsoWorld.instance:getMetaGrid()",
        );
        assert!(ok[0].is_nil());
        let f = pz_kahlua::stdlib::load_string(
            "local k = ... ; return k:getName()",
            None,
            t.globals().clone(),
        )
        .unwrap();
        assert!(
            t.call(&f, &[r[0].clone()]).is_err(),
            "stale handle must not read new data"
        );
    }

    #[test]
    fn get_world_and_zone_registration() {
        let dir = load_world("lua-world");
        let maps = dir.0.join("media/maps/Test");
        std::fs::write(
            maps.join("streets.xml"),
            r#"<streets version="1">
                <street name="Main St" width="6"><points><point x="260" y="300"/><point x="460" y="300"/></points></street>
                <street name="Old Railroad" width="4"><points><point x="260" y="310"/><point x="300" y="310"/></points></street>
                <street name="Diag Rd" width="3"><points><point x="260" y="320"/><point x="270" y="330"/><point x="280" y="340"/></points></street>
            </streets>"#,
        )
        .unwrap();
        crate::bind_core::shared_core().borrow_mut().game_map = "Test".into();
        lua_world::set_railroad_strings(vec!["Railroad".into(), "Branch Line".into()]);
        let fs = Rc::new(RefCell::new(GameFs::new(&dir.0)));
        // `java.util.ArrayList` from bind_java_util, as in the game.
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);
        lua_world::register(&mut t, fs);
        run(
            &mut t,
            r#"
            local w = getWorld()
            check(w == IsoWorld.instance)
            local dirs = getLotDirectories()
            check(dirs:size() == 1 and dirs:get(0) == "Test")
            check(w:getMetaGrid():getLotDirectories():get(0) == "Test")
            check(fileExists("media/maps/Test/map.info") and not fileExists("media/maps/Test/nope.lua"))
            local z = w:registerZone("n", "TownZone", 260, 260, 0, 10, 10)
            check(z:getType() == "TownZone" and z:getName() == "n" and z:getWidth() == 10)
            check(w:registerVehiclesZone("v", "TownZone", 260, 260, 0, 5, 5, {}) == nil)
            local v = w:registerVehiclesZone("v", "ParkingStall", 270, 270, 0, 5, 5, {})
            check(v ~= nil and v:getType() == "ParkingStall")
            local g = w:getMetaGrid():registerGeometryZone("p", "Forest", 0, "polygon", { 260, 260, 270, 260, 270, 270 }, nil)
            check(g:isPolygon())
            check(not pcall(function() w:getMetaGrid():registerGeometryZone("p", "Forest", 0, "circle", { 1, 2 }, nil) end))
            check(not pcall(function() w:getMetaGrid():registerGeometryZone("p", "Forest", 0, "point", { 1 }, nil) end))
            check(w:getMetaGrid():registerWorldGenZone("g", "Other", 260, 260, 0, 5, 5, {}) == nil)
            w:registerSpawnOrigin(1, 2, 3, 4, nil)
            w:registerSpawnOrigin(-1, 2, 3, 4, nil)
            w:registerWaterFlow(1, 2, 90, 0.5)
            w:registerWaterZone(0, 0, 10, 10, 1.0, 0.0)
            w:registerNavZones()
            w:checkVehiclesZones()
            w:setLuaPosX(300); w:setLuaPosY(-5)
            check(w:getLuaPosX() == 300 and w:getLuaSpawnCellX() == 44 and w:getLuaSpawnCellY() == 251)
            w:setWorld("  save  "); check(w:getWorld() == "save")
            w:setGameMode("LastStand"); check(w:getGameMode() == "LastStand")
            "#,
        );
        assert_eq!(lua_world::take_spawn_origins(), vec![(1, 2, 3, 4)]);
        assert!(lua_world::take_spawn_origins().is_empty());
        assert_eq!(lua_world::shared_water_flow().borrow().zones.len(), 1);
        assert!(crate::bind_core::shared_core().borrow().last_stand);
        with_world(|w| {
            let g = w.meta_grid.as_ref().unwrap();
            let nav: Vec<_> = g
                .zone_defs
                .iter()
                .filter(|z| z.zone_type == "Nav")
                .collect();
            // Main St: one 200x6 rectangle; the railroad nothing; Diag Rd
            // one polyline.
            assert_eq!(nav.len(), 2, "{nav:?}");
            assert_eq!((nav[0].x, nav[0].y, nav[0].w, nav[0].h), (260, 297, 200, 6));
            assert!(nav[1].is_polyline());
            assert_eq!(nav[1].points, vec![260, 320, 270, 330, 270, 330, 280, 340]);
        });
    }
}
