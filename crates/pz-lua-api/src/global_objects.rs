//! `zombie.globalObjects`: the client (`CGlobalObjects`) and server
//! (`SGlobalObjects`) registries of global-object systems, the systems and
//! objects themselves, `GlobalObjectLookup`'s per-chunk index, and the
//! single-player loopback that carries `sendCommand` and object sync
//! between the two halves.
//!
//! A system is mostly a Lua object: its mod-data table *is* the Lua
//! instance (`CGlobalObjectSystem:new` sets a metatable on it) and the Java
//! side calls back into that table (`OnServerCommand`, `newLuaObjectAt`,
//! ...). That is why this lives next to the Lua API rather than in a world
//! crate: the state is Lua tables and the behaviour is Lua calls.
//!
//! The engine drives it with [`init_lookup`] once the world's meta grid is
//! known (`GlobalObjectLookup.init`), [`init_systems`] at world start, and
//! [`update_server`] / [`update_client`] each frame (the single-player
//! packet pumps). Saving and loading `gos_*.bin` is in
//! [`crate::global_objects_save`]; multiplayer transport and the
//! `IsoObject`-based hooks are not ported.

use pz_kahlua::{LuaError, LuaResult, TableRef, Thread, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

/// Which registry a system belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// `CGlobalObjects` / `CGlobalObjectSystem` / `CGlobalObject`.
    Client,
    /// `SGlobalObjects` / `SGlobalObjectSystem` / `SGlobalObject`.
    Server,
}

fn npe(msg: &str) -> LuaError {
    LuaError::Runtime {
        message: msg.to_string(),
        class: "java.lang.NullPointerException",
    }
}

pub(crate) fn illegal_state(msg: impl Into<String>) -> LuaError {
    LuaError::Runtime {
        message: msg.into(),
        class: "java.lang.IllegalStateException",
    }
}

pub(crate) fn illegal_argument(msg: impl Into<String>) -> LuaError {
    LuaError::Runtime {
        message: msg.into(),
        class: "java.lang.IllegalArgumentException",
    }
}

/// `DebugLog.log(message)`.
pub(crate) fn debug_log(message: &str) {
    pz_core::debug::with(|d| d.log(pz_core::debug::DebugType::General, message));
}

/// `CGlobalObjects.noise` / `SGlobalObjects.noise`: logged only with
/// `Core.debug`.
pub fn noise(side: Side, message: &str) {
    if crate::bind_core::shared_core().borrow().debug {
        let prefix = match side {
            Side::Client => "CGlobalObjects",
            Side::Server => "SGlobalObjects",
        };
        debug_log(&format!("{prefix}: {message}"));
    }
}

/// `CGlobalObjectSystem` / `SGlobalObjectSystem` (and their shared base
/// `GlobalObjectSystem`).
pub struct GlobalObjectSystem {
    pub side: Side,
    pub name: String,
    /// The Lua instance of the system class.
    pub mod_data: TableRef,
    objects: RefCell<Vec<Rc<GlobalObject>>>,
    /// Server only: `modDataKeys`, `objectModDataKeys`, `objectSyncKeys`.
    pub mod_data_keys: RefCell<HashSet<String>>,
    pub object_mod_data_keys: RefCell<HashSet<String>>,
    pub object_sync_keys: RefCell<HashSet<String>>,
    pub loaded_world_version: Cell<i32>,
    me: Weak<GlobalObjectSystem>,
}

/// `CGlobalObject` / `SGlobalObject`.
pub struct GlobalObject {
    pub side: Side,
    system: RefCell<Weak<GlobalObjectSystem>>,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub mod_data: TableRef,
    me: Weak<GlobalObject>,
}

impl GlobalObject {
    /// This object's own handle.
    pub fn rc(&self) -> Rc<GlobalObject> {
        self.me.upgrade().expect("alive while borrowed")
    }

    /// `getSystem()`; `None` once the object was reset.
    pub fn system(&self) -> Option<Rc<GlobalObjectSystem>> {
        self.system.borrow().upgrade()
    }

    fn belongs_to(&self, system: &GlobalObjectSystem) -> bool {
        std::ptr::eq(self.system.borrow().as_ptr(), system)
    }

    /// `Reset()`.
    pub fn reset(&self) {
        *self.system.borrow_mut() = Weak::new();
        self.mod_data.wipe();
    }

    /// `isValidIsoObject(obj)` for a non-null object: only farming plants
    /// are recognised, by this object's own mod data.
    pub fn is_valid_iso_object(&self) -> bool {
        let Some(system) = self.system() else {
            return false;
        };
        if system.name == "farming" {
            let md = &self.mod_data;
            return !md.get_str("state").is_nil()
                && !md.get_str("nbOfGrow").is_nil()
                && !md.get_str("health").is_nil();
        }
        false
    }
}

impl GlobalObjectSystem {
    /// This system's own handle.
    pub fn rc(&self) -> Rc<GlobalObjectSystem> {
        self.me.upgrade().expect("alive while borrowed")
    }

    /// `new CGlobalObjectSystem(name)` / `new SGlobalObjectSystem(name)`
    /// (not registered).
    pub fn new(side: Side, name: &str) -> LuaResult<Rc<GlobalObjectSystem>> {
        if !name.is_empty() && (name.contains("..") || name.contains("\0.\0.")) {
            return Err(illegal_state(format!(
                "Name of GlobalObjectSystem invalid: {name}"
            )));
        }
        Ok(Rc::new_cyclic(|me| GlobalObjectSystem {
            side,
            name: name.to_string(),
            mod_data: TableRef::new(),
            objects: RefCell::new(Vec::new()),
            mod_data_keys: RefCell::default(),
            object_mod_data_keys: RefCell::default(),
            object_sync_keys: RefCell::default(),
            loaded_world_version: Cell::new(-1),
            me: me.clone(),
        }))
    }

    /// `newObject(x, y, z)`.
    pub fn new_object(&self, x: i32, y: i32, z: i32) -> LuaResult<Rc<GlobalObject>> {
        if self.get_object_at(x, y, z)?.is_some() {
            return Err(illegal_state(format!("already an object at {x},{y},{z}")));
        }
        let object = Rc::new_cyclic(|me| GlobalObject {
            side: self.side,
            system: RefCell::new(self.me.clone()),
            x,
            y,
            z,
            mod_data: TableRef::new(),
            me: me.clone(),
        });
        self.objects.borrow_mut().push(object.clone());
        lookup_add(self.side, &object)?;
        Ok(object)
    }

    /// `removeObject(object)`.
    pub fn remove_object(&self, object: &Rc<GlobalObject>) -> LuaResult<()> {
        if !object.belongs_to(self) {
            return Err(illegal_state("object not in this system"));
        }
        {
            let mut objects = self.objects.borrow_mut();
            if let Some(i) = objects.iter().position(|o| Rc::ptr_eq(o, object)) {
                objects.remove(i);
            }
        }
        lookup_remove(self.side, object)?;
        object.reset();
        Ok(())
    }

    /// `getObjectAt(x, y, z)`.
    pub fn get_object_at(&self, x: i32, y: i32, z: i32) -> LuaResult<Option<Rc<GlobalObject>>> {
        with_chunk(self.side, x, y, |chunk| {
            chunk
                .iter()
                .find(|o| o.belongs_to(self) && o.x == x && o.y == y && o.z == z)
                .cloned()
        })
        .map(Option::flatten)
    }

    /// `hasObjectsInChunk(wx, wy)`.
    pub fn has_objects_in_chunk(&self, wx: i32, wy: i32) -> LuaResult<bool> {
        Ok(with_chunk(self.side, wx * 8, wy * 8, |chunk| {
            chunk.iter().any(|o| o.belongs_to(self))
        })?
        .unwrap_or(false))
    }

    /// `getObjectsInChunk(wx, wy)`, appended to `out`.
    pub fn objects_in_chunk(
        &self,
        wx: i32,
        wy: i32,
        out: &mut Vec<Rc<GlobalObject>>,
    ) -> LuaResult<()> {
        with_chunk(self.side, wx * 8, wy * 8, |chunk| {
            out.extend(chunk.iter().filter(|o| o.belongs_to(self)).cloned())
        })?;
        Ok(())
    }

    /// `getObjectsAdjacentTo(x, y, z)`, appended to `out`.
    pub fn objects_adjacent_to(
        &self,
        x: i32,
        y: i32,
        z: i32,
        out: &mut Vec<Rc<GlobalObject>>,
    ) -> LuaResult<()> {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(o) = self.get_object_at(x + dx, y + dy, z)? {
                    out.push(o);
                }
            }
        }
        Ok(())
    }

    pub fn object_count(&self) -> usize {
        self.objects.borrow().len()
    }

    /// `getObjectByIndex(index)`: `None` when out of range.
    pub fn object_by_index(&self, index: i32) -> Option<Rc<GlobalObject>> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.objects.borrow().get(i).cloned())
    }

    /// `Reset()`.
    pub fn reset(&self) {
        let objects = std::mem::take(&mut *self.objects.borrow_mut());
        for o in &objects {
            o.reset();
        }
        self.mod_data.wipe();
        if self.side == Side::Server {
            self.mod_data_keys.borrow_mut().clear();
            self.object_mod_data_keys.borrow_mut().clear();
            self.object_sync_keys.borrow_mut().clear();
        }
    }

    /// `rawget(modData, name)` as a method of this system; the original
    /// throws when a callback is missing.
    fn callback(&self, name: &str) -> LuaResult<Value> {
        let f = self.mod_data.get_own(&Value::str(name));
        if f.is_nil() {
            return Err(illegal_state(format!(
                "{name} method undefined for system '{}'",
                self.name
            )));
        }
        Ok(f)
    }

    /// `LuaManager.caller.pcall(thread, function, modData, args...)`.
    fn pcall_method(
        &self,
        thread: &mut Thread,
        name: &str,
        args: &[Value],
    ) -> LuaResult<Option<Vec<Value>>> {
        let f = self.callback(name)?;
        let mut all = Vec::with_capacity(args.len() + 1);
        all.push(Value::Table(self.mod_data.clone()));
        all.extend_from_slice(args);
        Ok(pcall_logged(thread, &f, &all))
    }

    // ---- CGlobalObjectSystem ------------------------------------------------

    /// `receiveServerCommand(command, args)`.
    pub fn receive_server_command(
        &self,
        thread: &mut Thread,
        command: Value,
        args: Value,
    ) -> LuaResult<()> {
        self.pcall_method(thread, "OnServerCommand", &[command, args])
            .map(drop)
    }

    /// `receiveNewLuaObjectAt(x, y, z, args)`.
    pub fn receive_new_lua_object_at(
        &self,
        thread: &mut Thread,
        x: i32,
        y: i32,
        z: i32,
        args: &TableRef,
    ) -> LuaResult<()> {
        self.pcall_method(
            thread,
            "newLuaObjectAt",
            &[Value::from(x), Value::from(y), Value::from(z)],
        )?;
        if let Some(o) = self.get_object_at(x, y, z)? {
            for (k, v) in args.entries() {
                o.mod_data.rawset(k, v);
            }
        }
        Ok(())
    }

    /// `receiveRemoveLuaObjectAt(x, y, z)`.
    pub fn receive_remove_lua_object_at(
        &self,
        thread: &mut Thread,
        x: i32,
        y: i32,
        z: i32,
    ) -> LuaResult<()> {
        self.pcall_method(
            thread,
            "removeLuaObjectAt",
            &[Value::from(x), Value::from(y), Value::from(z)],
        )
        .map(drop)
    }

    /// `receiveUpdateLuaObjectAt(x, y, z, args)`.
    pub fn receive_update_lua_object_at(
        &self,
        thread: &mut Thread,
        x: i32,
        y: i32,
        z: i32,
        args: &TableRef,
    ) -> LuaResult<()> {
        let Some(o) = self.get_object_at(x, y, z)? else {
            return Ok(());
        };
        for (k, v) in args.entries() {
            o.mod_data.rawset(k, v);
        }
        self.pcall_method(
            thread,
            "OnLuaObjectUpdated",
            &[Value::Table(o.mod_data.clone())],
        )
        .map(drop)
    }

    // ---- SGlobalObjectSystem ------------------------------------------------

    /// `chunkLoaded(wx, wy)`.
    pub fn chunk_loaded(&self, thread: &mut Thread, wx: i32, wy: i32) -> LuaResult<()> {
        if !self.has_objects_in_chunk(wx, wy)? {
            return Ok(());
        }
        self.pcall_method(thread, "OnChunkLoaded", &[Value::from(wx), Value::from(wy)])
            .map(drop)
    }

    /// `receiveClientCommand(command, playerObj, args)`.
    pub fn receive_client_command(
        &self,
        thread: &mut Thread,
        command: Value,
        player: Value,
        args: Value,
    ) -> LuaResult<()> {
        self.pcall_method(thread, "OnClientCommand", &[command, player, args])
            .map(drop)
    }

    /// `getInitialStateForClient()`.
    pub fn initial_state_for_client(&self, thread: &mut Thread) -> LuaResult<Option<TableRef>> {
        let r = self.pcall_method(thread, "getInitialStateForClient", &[])?;
        Ok(r.and_then(|v| v.into_iter().next())
            .and_then(|v| v.as_table().cloned()))
    }
}

/// `caller.pcall`: errors are logged and give `None`.
pub(crate) fn pcall_logged(thread: &mut Thread, f: &Value, args: &[Value]) -> Option<Vec<Value>> {
    match thread.pcall(f, args) {
        Ok(v) => Some(v),
        Err((msg, trace)) => {
            (thread.log)(&format!(
                "{}\n{}",
                pz_kahlua::exec::java_value_of(&msg),
                trace
            ));
            None
        }
    }
}

// ---- GlobalObjectLookup -------------------------------------------------------

const SQUARES_PER_CHUNK: i32 = 8;
const SQUARES_PER_CELL: i32 = 256;
const CHUNKS_PER_CELL: i32 = 32;

/// The meta grid's cell bounds (`IsoMetaGrid.minX/minY/getWidth/getHeight`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub width: i32,
    pub height: i32,
}

struct LookupCell {
    cx: i32,
    cy: i32,
    chunks: Vec<Option<Vec<Rc<GlobalObject>>>>,
}

impl LookupCell {
    fn chunk_index(&self, x: i32, y: i32) -> usize {
        let cx = (x - self.cx * SQUARES_PER_CELL) / SQUARES_PER_CHUNK;
        let cy = (y - self.cy * SQUARES_PER_CELL) / SQUARES_PER_CHUNK;
        (cx + cy * CHUNKS_PER_CELL) as usize
    }
}

#[derive(Default)]
struct Lookup {
    grid: Option<GridBounds>,
    /// `sharedServer.cells` / `sharedClient.cells`.
    server: Option<Vec<Option<LookupCell>>>,
    client: Option<Vec<Option<LookupCell>>>,
}

impl Lookup {
    fn cells(&mut self, side: Side) -> &mut Option<Vec<Option<LookupCell>>> {
        match side {
            Side::Server => &mut self.server,
            Side::Client => &mut self.client,
        }
    }

    /// `getCellAt(x, y, create)`: the cell index, `Ok(None)` (logged) for a
    /// square outside the grid.
    fn cell_at(&mut self, side: Side, x: i32, y: i32, create: bool) -> LuaResult<Option<usize>> {
        let g = self
            .grid
            .ok_or_else(|| npe("GlobalObjectLookup.metaGrid is null"))?;
        let rx = x - g.min_x * SQUARES_PER_CELL;
        let ry = y - g.min_y * SQUARES_PER_CELL;
        if rx < 0 || ry < 0 || rx >= g.width * SQUARES_PER_CELL || ry >= g.height * SQUARES_PER_CELL
        {
            debug_log(&format!(
                "ERROR: GlobalObjectLookup.getCellForObject object location invalid {x},{y}"
            ));
            return Ok(None);
        }
        let (cell_x, cell_y) = (rx / SQUARES_PER_CELL, ry / SQUARES_PER_CELL);
        let index = (cell_x + cell_y * g.width) as usize;
        let cells = self
            .cells(side)
            .as_mut()
            .ok_or_else(|| npe("GlobalObjectLookup cells are null"))?;
        if cells[index].is_none() && create {
            cells[index] = Some(LookupCell {
                cx: g.min_x + cell_x,
                cy: g.min_y + cell_y,
                chunks: (0..CHUNKS_PER_CELL * CHUNKS_PER_CELL)
                    .map(|_| None)
                    .collect(),
            });
        }
        Ok(cells[index].as_ref().map(|_| index))
    }

    fn cell(&mut self, side: Side, index: usize) -> &mut LookupCell {
        self.cells(side)
            .as_mut()
            .and_then(|c| c[index].as_mut())
            .expect("cell checked by cell_at")
    }
}

thread_local! {
    static LOOKUP: RefCell<Lookup> = RefCell::default();
}

/// `GlobalObjectLookup.init(metaGrid)` for single player: both the server
/// and client indexes get one slot per meta-grid cell.
pub fn init_lookup(grid: GridBounds) {
    LOOKUP.with(|l| {
        let mut l = l.borrow_mut();
        l.grid = Some(grid);
        let n = (grid.width.max(0) * grid.height.max(0)) as usize;
        l.server = Some((0..n).map(|_| None).collect());
        l.client = Some((0..n).map(|_| None).collect());
    });
}

/// `GlobalObjectLookup.Reset()`.
pub fn reset_lookup() {
    LOOKUP.with(|l| {
        let mut l = l.borrow_mut();
        l.server = None;
        l.client = None;
    });
}

/// Run `f` on the chunk holding square (x, y) if it exists. Errors only
/// when the lookup was never initialised (the original's NPE).
fn with_chunk<R>(
    side: Side,
    x: i32,
    y: i32,
    f: impl FnOnce(&[Rc<GlobalObject>]) -> R,
) -> LuaResult<Option<R>> {
    let chunk = LOOKUP.with(|l| -> LuaResult<_> {
        let mut l = l.borrow_mut();
        let Some(ci) = l.cell_at(side, x, y, false)? else {
            return Ok(None);
        };
        let cell = l.cell(side, ci);
        let i = cell.chunk_index(x, y);
        Ok(cell.chunks[i].clone())
    })?;
    Ok(chunk.map(|c| f(&c)))
}

fn lookup_add(side: Side, object: &Rc<GlobalObject>) -> LuaResult<()> {
    LOOKUP.with(|l| {
        let mut l = l.borrow_mut();
        let Some(ci) = l.cell_at(side, object.x, object.y, true)? else {
            debug_log(&format!(
                "ERROR: GlobalObjectLookup.addObject object location invalid {},{}",
                object.x, object.y
            ));
            return Ok(());
        };
        let cell = l.cell(side, ci);
        let i = cell.chunk_index(object.x, object.y);
        let chunk = cell.chunks[i].get_or_insert_with(Vec::new);
        if chunk.iter().any(|o| Rc::ptr_eq(o, object)) {
            return Err(illegal_state("duplicate object"));
        }
        chunk.push(object.clone());
        Ok(())
    })
}

fn lookup_remove(side: Side, object: &Rc<GlobalObject>) -> LuaResult<()> {
    LOOKUP.with(|l| {
        let mut l = l.borrow_mut();
        let Some(ci) = l.cell_at(side, object.x, object.y, false)? else {
            debug_log(&format!(
                "ERROR: GlobalObjectLookup.removeObject object location invalid {},{}",
                object.x, object.y
            ));
            return Ok(());
        };
        let cell = l.cell(side, ci);
        let i = cell.chunk_index(object.x, object.y);
        let pos = cell.chunks[i]
            .as_ref()
            .and_then(|c| c.iter().position(|o| Rc::ptr_eq(o, object)));
        match pos {
            Some(p) => {
                cell.chunks[i].as_mut().unwrap().remove(p);
                Ok(())
            }
            None => Err(illegal_state("chunk doesn't contain object")),
        }
    })
}

// ---- registries -----------------------------------------------------------------

#[derive(Default)]
struct Registry {
    systems: Vec<Rc<GlobalObjectSystem>>,
    /// Client only: `CGlobalObjects.initialState`.
    initial_state: HashMap<String, TableRef>,
}

thread_local! {
    static CLIENT: RefCell<Registry> = RefCell::default();
    static SERVER: RefCell<Registry> = RefCell::default();
}

fn with_registry<R>(side: Side, f: impl FnOnce(&mut Registry) -> R) -> R {
    match side {
        Side::Client => CLIENT.with(|r| f(&mut r.borrow_mut())),
        Side::Server => SERVER.with(|r| f(&mut r.borrow_mut())),
    }
}

/// `getSystemCount()`.
pub fn system_count(side: Side) -> usize {
    with_registry(side, |r| r.systems.len())
}

/// `getSystemByIndex(index)`.
pub fn system_by_index(side: Side, index: i32) -> Option<Rc<GlobalObjectSystem>> {
    with_registry(side, |r| {
        usize::try_from(index)
            .ok()
            .and_then(|i| r.systems.get(i).cloned())
    })
}

/// `getSystemByName(name)`.
pub fn system_by_name(side: Side, name: &str) -> Option<Rc<GlobalObjectSystem>> {
    with_registry(side, |r| r.systems.iter().find(|s| s.name == name).cloned())
}

/// `newSystem(name)`.
pub fn new_system(side: Side, name: &str) -> LuaResult<Rc<GlobalObjectSystem>> {
    if system_by_name(side, name).is_some() {
        return Err(illegal_state("system with that name already exists"));
    }
    noise(side, &format!("newSystem {name}"));
    let system = GlobalObjectSystem::new(side, name)?;
    with_registry(side, |r| r.systems.push(system.clone()));
    Ok(system)
}

fn int_field(t: &TableRef, key: &str) -> LuaResult<i32> {
    match t.get_own(&Value::str(key)) {
        Value::Num(n) => Ok(n as i32),
        Value::Nil => Err(npe(&format!("{key} is null"))),
        _ => Err(LuaError::Runtime {
            message: format!("{key} is not a Double"),
            class: "java.lang.ClassCastException",
        }),
    }
}

/// `registerSystem(name)`. A new client system takes over the initial
/// state the server handed over; a new server system loads its save file
/// from the folder set by [`crate::global_objects_save::set_save_dir`].
pub fn register_system(side: Side, name: &str) -> LuaResult<Rc<GlobalObjectSystem>> {
    if let Some(s) = system_by_name(side, name) {
        return Ok(s);
    }
    let system = new_system(side, name)?;
    if side == Side::Server {
        crate::global_objects_save::load_registered(&system);
    }
    if side == Side::Client {
        let state = with_registry(side, |r| r.initial_state.get(name).cloned());
        if let Some(tbl) = state {
            for (key, value) in tbl.entries() {
                if matches!(&key, Value::Str(s) if &**s == "_objects") {
                    let Some(objects) = value.as_table() else {
                        return Err(npe("_objects is not a table"));
                    };
                    for i in 1..=objects.len() {
                        let Some(obj) = objects.get_int(i).as_table().cloned() else {
                            return Err(npe("object table is null"));
                        };
                        let (x, y, z) = (
                            int_field(&obj, "x")?,
                            int_field(&obj, "y")?,
                            int_field(&obj, "z")?,
                        );
                        for k in ["x", "y", "z"] {
                            obj.set_str(k, Value::Nil);
                        }
                        let object = system.new_object(x, y, z)?;
                        for (k, v) in obj.entries() {
                            object.mod_data.rawset(k, v);
                        }
                    }
                    objects.wipe();
                    continue;
                }
                system.mod_data.rawset(key, value);
            }
        }
    }
    Ok(system)
}

/// `CGlobalObjects.Reset()` / `SGlobalObjects.Reset()` (the server one also
/// resets the lookup).
pub fn reset(side: Side) {
    let systems = with_registry(side, |r| {
        r.initial_state.clear();
        std::mem::take(&mut r.systems)
    });
    for s in &systems {
        s.reset();
    }
    if side == Side::Server {
        reset_lookup();
    }
}

/// `CGlobalObjects.receiveServerCommand(systemName, command, args)`.
pub fn client_receive_server_command(
    thread: &mut Thread,
    system: &str,
    command: &str,
    args: Value,
) -> LuaResult<bool> {
    let s = system_by_name(Side::Client, system)
        .ok_or_else(|| illegal_state(format!("system '{system}' not found")))?;
    s.receive_server_command(thread, Value::str(command), args)?;
    Ok(true)
}

/// `SGlobalObjects.receiveClientCommand(systemName, command, playerObj, args)`.
pub fn server_receive_client_command(
    thread: &mut Thread,
    system: &str,
    command: &str,
    player: Value,
    args: Value,
) -> LuaResult<bool> {
    let s = system_by_name(Side::Server, system)
        .ok_or_else(|| illegal_state(format!("system '{system}' not found")))?;
    s.receive_client_command(thread, Value::str(command), player, args)?;
    Ok(true)
}

/// `SGlobalObjects.chunkLoaded(wx, wy)`.
pub fn chunk_loaded(thread: &mut Thread, wx: i32, wy: i32) -> LuaResult<()> {
    let systems = with_registry(Side::Server, |r| r.systems.clone());
    for s in systems {
        s.chunk_loaded(thread, wx, wy)?;
    }
    Ok(())
}

// ---- initial state and single-player transport -----------------------------------

/// What `TableNetworkUtils` can carry: string/number keys; string, number,
/// boolean, table and `IsoDirections` values (inventory items and dead
/// bodies are not ported).
pub fn can_send(key: &Value, value: &Value) -> bool {
    matches!(key, Value::Str(_) | Value::Num(_))
        && match value {
            Value::Str(_) | Value::Num(_) | Value::Bool(_) | Value::Table(_) => true,
            Value::Object(o) => o.class_name() == crate::bind_iso_enums::ISO_DIRECTIONS,
            _ => false,
        }
}

/// `TableNetworkUtils.save` followed by `load` into a new table: a deep
/// copy of what can be sent. With `keys`, only those top-level keys
/// (`saveSome`).
pub fn network_copy(t: &TableRef, keys: Option<&HashSet<String>>) -> TableRef {
    let out = TableRef::new();
    for (k, v) in t.entries() {
        if let Some(keys) = keys {
            if !matches!(&k, Value::Str(s) if keys.contains(&**s)) {
                continue;
            }
        }
        if !can_send(&k, &v) {
            continue;
        }
        let v = match v {
            Value::Table(sub) => Value::Table(network_copy(&sub, None)),
            v => v,
        };
        out.rawset(k, v);
    }
    out
}

fn warn_unsendable(what: &str, args: &TableRef) {
    for (k, v) in args.entries() {
        if !can_send(&k, &v) {
            let (k, v) = (
                pz_kahlua::exec::java_value_of(&k),
                pz_kahlua::exec::java_value_of(&v),
            );
            debug_log(&format!("ERROR: {what}: can't save key,value={k},{v}"));
        }
    }
}

/// `SGlobalObjects.initSystems()` in single player: fire
/// `OnSGlobalObjectSystemInit`, then hand every system's initial state to
/// the client (`saveInitialStateForClient` / `loadInitialState`). Errors in
/// the hand-over are logged and drop it, like the original's catch.
pub fn init_server_systems(thread: &mut Thread, events: &crate::events::EventManager) {
    events.trigger(thread, "OnSGlobalObjectSystemInit", &[]);
    match initial_states_for_client(thread) {
        Ok(states) => CLIENT.with(|c| c.borrow_mut().initial_state.extend(states)),
        Err(e) => log_error(thread, &e),
    }
}

/// `CGlobalObjects.initSystems()`.
pub fn init_client_systems(thread: &mut Thread, events: &crate::events::EventManager) {
    events.trigger(thread, "OnCGlobalObjectSystemInit", &[]);
}

fn initial_states_for_client(thread: &mut Thread) -> LuaResult<Vec<(String, TableRef)>> {
    let systems = with_registry(Side::Server, |r| r.systems.clone());
    let mut out = Vec::new();
    for system in systems {
        let tbl = system
            .initial_state_for_client(thread)?
            .unwrap_or_else(TableRef::new);
        let objects = TableRef::new();
        tbl.set_str("_objects", objects.clone());
        for j in 0..system.object_count() {
            let o = system.object_by_index(j as i32).expect("index in range");
            let t = TableRef::new();
            t.set_str("x", o.x);
            t.set_str("y", o.y);
            t.set_str("z", o.z);
            for key in system.object_sync_keys.borrow().iter() {
                t.set_str(key, o.mod_data.get_str(key));
            }
            objects.set_int(j as i64 + 1, t);
        }
        out.push((system.name.clone(), network_copy(&tbl, None)));
    }
    Ok(out)
}

/// A `GlobalObjects` packet in flight between the single-player client and
/// server.
enum Packet {
    ClientCommand {
        player: Value,
        system: String,
        command: String,
        args: Option<TableRef>,
    },
    ServerCommand {
        system: String,
        command: String,
        args: Option<TableRef>,
    },
    NewLuaObjectAt {
        system: String,
        x: i32,
        y: i32,
        z: i32,
        args: TableRef,
    },
    RemoveLuaObjectAt {
        system: String,
        x: i32,
        y: i32,
        z: i32,
    },
    UpdateLuaObjectAt {
        system: String,
        x: i32,
        y: i32,
        z: i32,
        args: TableRef,
    },
}

#[derive(Default)]
struct Pending {
    to_server: Vec<Packet>,
    to_client: Vec<Packet>,
}

thread_local! {
    static PENDING: RefCell<Pending> = RefCell::default();
}

fn args_for_send(what: &str, args: &Value) -> Option<TableRef> {
    let t = args.as_table().filter(|t| !t.is_empty())?;
    warn_unsendable(what, t);
    Some(network_copy(t, None))
}

/// `CGlobalObjectSystem.sendCommand(command, player, args)`.
pub fn send_client_command(
    system: &GlobalObjectSystem,
    command: &str,
    player: Value,
    args: &Value,
) {
    let args = args_for_send("sendClientCommand", args);
    let p = Packet::ClientCommand {
        player,
        system: system.name.clone(),
        command: command.to_string(),
        args,
    };
    PENDING.with(|q| q.borrow_mut().to_server.push(p));
}

/// `SGlobalObjectSystem.sendCommand(command, args)`.
pub fn send_server_command(system: &GlobalObjectSystem, command: &str, args: &Value) {
    let args = args_for_send("sendServerCommand", args);
    let p = Packet::ServerCommand {
        system: system.name.clone(),
        command: command.to_string(),
        args,
    };
    PENDING.with(|q| q.borrow_mut().to_client.push(p));
}

/// Which of `addGlobalObjectOnClient`, `removeGlobalObjectOnClient` and
/// `updateGlobalObjectOnClient`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectSync {
    Add,
    Remove,
    Update,
}

/// `SGlobalObjectSystem.add/remove/updateGlobalObjectOnClient(object)`.
pub fn sync_object_to_client(
    system: &GlobalObjectSystem,
    object: Option<&Rc<GlobalObject>>,
    kind: ObjectSync,
) -> LuaResult<()> {
    let object = object.ok_or_else(|| illegal_argument("globalObject is null"))?;
    if !object.belongs_to(system) {
        return Err(illegal_argument("object not in this system"));
    }
    let (name, x, y, z) = (
        system.name.clone(),
        object.x,
        object.y,
        object.z as i8 as i32,
    );
    let p = match kind {
        ObjectSync::Remove => Packet::RemoveLuaObjectAt {
            system: name,
            x,
            y,
            z,
        },
        _ => {
            let args = network_copy(&object.mod_data, Some(&system.object_sync_keys.borrow()));
            if kind == ObjectSync::Add {
                Packet::NewLuaObjectAt {
                    system: name,
                    x,
                    y,
                    z,
                    args,
                }
            } else {
                Packet::UpdateLuaObjectAt {
                    system: name,
                    x,
                    y,
                    z,
                    args,
                }
            }
        }
    };
    PENDING.with(|q| q.borrow_mut().to_client.push(p));
    Ok(())
}

fn log_error(thread: &mut Thread, e: &LuaError) {
    let m = pz_kahlua::exec::java_value_of(&e.pcall_message());
    (thread.log)(&m);
}

/// `SinglePlayerServer.update()` for global-object packets: deliver what the
/// client sent since the last call.
pub fn update_server(thread: &mut Thread) {
    let packets = PENDING.with(|q| std::mem::take(&mut q.borrow_mut().to_server));
    for p in packets {
        if let Packet::ClientCommand {
            player,
            system,
            command,
            args,
        } = p
        {
            if player.is_nil() {
                // getAnyPlayerFromConnection: players are not ported.
                debug_log("receiveGlobalObjects: player is null");
                continue;
            }
            let args = args.map_or(Value::Nil, Value::Table);
            if let Err(e) = server_receive_client_command(thread, &system, &command, player, args) {
                log_error(thread, &e);
            }
        }
    }
}

/// `SinglePlayerClient.update()` for global-object packets.
pub fn update_client(thread: &mut Thread) {
    let packets = PENDING.with(|q| std::mem::take(&mut q.borrow_mut().to_client));
    for p in packets {
        let r = match p {
            Packet::ServerCommand {
                system,
                command,
                args,
            } => client_receive_server_command(
                thread,
                &system,
                &command,
                args.map_or(Value::Nil, Value::Table),
            )
            .map(drop),
            Packet::NewLuaObjectAt {
                system,
                x,
                y,
                z,
                args,
            } => match system_by_name(Side::Client, &system) {
                Some(s) => s.receive_new_lua_object_at(thread, x, y, z, &args),
                None => Ok(()),
            },
            Packet::RemoveLuaObjectAt { system, x, y, z } => {
                match system_by_name(Side::Client, &system) {
                    Some(s) => s.receive_remove_lua_object_at(thread, x, y, z),
                    None => Ok(()),
                }
            }
            Packet::UpdateLuaObjectAt {
                system,
                x,
                y,
                z,
                args,
            } => match system_by_name(Side::Client, &system) {
                Some(s) => s.receive_update_lua_object_at(thread, x, y, z, &args),
                None => Ok(()),
            },
            Packet::ClientCommand { .. } => Ok(()),
        };
        if let Err(e) = r {
            log_error(thread, &e);
        }
    }
}
