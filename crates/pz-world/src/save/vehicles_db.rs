//! `VehiclesDB2.SQLStore` on a save's `vehicles.db`: `initUsedIDs`;
//! `loadChunk` with `WorldStreamerThread.vehicleLoaded`, which puts a loaded
//! chunk's vehicles into [`IsoChunk::vehicles`]; and `unloadChunk` with
//! `updateVehicle`, which writes them back when the chunk is saved.
//!
//! The connection is the store's own (`VehiclesDB2.SQLStore.conn`, opened
//! by `SQLStore.create`); these functions take it from the caller so the
//! world streamer and the saving side share one connection, as the
//! original's single store does.
//!
//! `WorldStreamer.DoChunkAlways` calls `VehiclesDB2.instance.loadChunk`
//! right after `IsoChunk.LoadChunk` succeeds (a chunk read from its save
//! file or made by `LoadBrandNew`), before `loadInWorldStreamerThread`
//! creates the missing squares: [`load_chunk`] belongs between
//! [`crate::save::chunk_file::load_chunk`] (or
//! [`crate::chunk::load_brand_new`]) and
//! [`crate::save::chunk_file::finish_chunk_load`]. The world streamer
//! does this for every chunk it serves when its
//! [`crate::streamer::StreamCtx::vehicles_db`] is set.

use std::collections::BTreeSet;

use pz_sqlite::{Connection, Row, Table, Value};

use pz_inventory::template::ItemClass;

use super::io::Writer;
use super::items::ItemBlob;
use super::vehicle::{AlarmSounds, BaseVehicle, VehicleRow};
use super::SaveCtx;
use crate::chunk::IsoChunk;
use crate::chunk_vehicles::{LoadedVehicle, VehicleHandle, VehicleOrigin};

/// `rs.getFloat`: `(float) sqlite3_column_double`.
fn get_float(v: &Value) -> f32 {
    let text = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0);
    (match v {
        Value::Null => 0.0,
        Value::Integer(i) => *i as f64,
        Value::Real(r) => *r,
        Value::Text(s) => text(s),
        Value::Blob(b) => text(&String::from_utf8_lossy(b)),
    }) as f32
}

/// An `INTEGER` column compared with an integer parameter (`wx=?`).
fn int_eq(v: &Value, want: i32) -> bool {
    match v {
        Value::Integer(i) => *i == i64::from(want),
        Value::Real(r) => *r == f64::from(want),
        _ => false,
    }
}

/// `rs.getBinaryStream`: `None` for NULL (the original then fails on the
/// null stream).
fn get_bytes(v: Value) -> Option<Vec<u8>> {
    match v {
        Value::Null => None,
        Value::Blob(b) => Some(b),
        Value::Text(s) => Some(s.into_bytes()),
        v => Some(v.as_text().unwrap_or_default().into_bytes()),
    }
}

/// `SQLStore.initUsedIDs`: `SELECT wx,wy,id FROM vehicles`, filling the
/// used ids and the seen chunks (`wy << 16 | wx`).
pub fn init_used_ids(conn: &mut Connection) -> pz_sqlite::Result<(BTreeSet<i32>, BTreeSet<i32>)> {
    let t = conn.table("vehicles")?;
    let (wx, wy, id) = (t.column("wx")?, t.column("wy")?, t.column("id")?);
    let mut used = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for r in conn.rows(&t)? {
        let (x, y) = (r.get(&t, wx).as_i32(), r.get(&t, wy).as_i32());
        seen.insert(y << 16 | x);
        used.insert(r.get(&t, id).as_i32());
    }
    Ok((used, seen))
}

/// One row of `SELECT id, x, y, data, worldversion, inMeta FROM vehicles
/// WHERE wx=? AND wy=?`; `data` is `None` for NULL.
fn chunk_row(t: &Table, r: &Row, wx: i32, wy: i32) -> pz_sqlite::Result<(VehicleRow, bool)> {
    let col = |n: &str| t.column(n).map(|i| r.get(t, i));
    let data = get_bytes(col("data")?);
    let row = VehicleRow {
        id: col("id")?.as_i32(),
        wx,
        wy,
        x: get_float(&col("x")?),
        y: get_float(&col("y")?),
        world_version: col("worldversion")?.as_i32(),
        in_meta: col("inMeta")?.as_i64() != 0,
        data: data.clone().unwrap_or_default(),
    };
    Ok((row, data.is_some()))
}

/// The rows of chunk `(wx, wy)` in the order the statement returns them:
/// both `ivwx` and `ivwy` list equal keys by rowid, so it is `id` order.
/// A row whose `data` is NULL ends the list (the original's statement loop
/// fails on it).
pub fn chunk_rows(
    conn: &mut Connection,
    wx: i32,
    wy: i32,
) -> pz_sqlite::Result<(Vec<VehicleRow>, bool)> {
    let t = conn.table("vehicles")?;
    let (cx, cy) = (t.column("wx")?, t.column("wy")?);
    let mut rows = Vec::new();
    for r in conn.rows(&t)? {
        if !int_eq(&r.get(&t, cx), wx) || !int_eq(&r.get(&t, cy), wy) {
            continue;
        }
        let (row, has_data) = chunk_row(&t, &r, wx, wy)?;
        if !has_data {
            return Ok((rows, false));
        }
        rows.push(row);
    }
    Ok((rows, true))
}

/// `SQLStore.removeVehicle(sqlID)`: `DELETE FROM vehicles WHERE id=?`,
/// committed; rolled back and logged on failure.
pub fn remove_vehicle(conn: &mut Connection, sql_id: i32) {
    if sql_id < 1 {
        return;
    }
    let r = conn
        .delete("vehicles", i64::from(sql_id))
        .and_then(|_| conn.commit());
    if let Err(e) = r {
        eprintln!("ERROR: vehicles.db: {e}");
        conn.rollback();
    }
}

/// What [`load_chunk`] did.
#[derive(Debug, Default)]
pub struct ChunkVehiclesLoaded {
    /// The vehicles added to the chunk's list, in order.
    pub added: Vec<VehicleHandle>,
    /// Rows whose `BaseVehicle.load` failed and that were deleted from the
    /// database (`vehicle %d is being deleted because an error occurred
    /// loading it`), with the error.
    pub removed: Vec<(i32, String)>,
    /// The error that ended the statement loop early (logged by the
    /// original; the rows before it stay loaded).
    pub error: Option<String>,
}

/// `VehiclesDB2.loadChunk(chunk)` → `SQLStore.loadChunk` →
/// `WorldStreamerThread.vehicleLoaded` for the chunk at `(chunk.wx,
/// chunk.wy)`: every row of the chunk whose blob starts with a set
/// serialise flag and the vehicle class id is loaded (`BaseVehicle.load`
/// after those two bytes) and added with [`IsoChunk::vehicle_loaded`]. A
/// row that fails to load is deleted from `conn`. `conn` is `None` when
/// the store has no connection (`Core.isNoSave`), which loads nothing.
///
/// Not ported: `vehicle.softReset()` for `JobType.SoftReset` chunks (a
/// server-only job).
pub fn load_chunk(
    conn: Option<&mut Connection>,
    chunk: &mut IsoChunk,
    ctx: &SaveCtx<'_>,
) -> ChunkVehiclesLoaded {
    let mut out = ChunkVehiclesLoaded::default();
    let Some(conn) = conn else {
        return out;
    };
    let (rows, complete) = match chunk_rows(conn, chunk.wx, chunk.wy) {
        Ok(r) => r,
        Err(e) => {
            out.error = Some(e.to_string());
            return out;
        }
    };
    let class = ctx.tables.class_id("Vehicle");
    let ts = ctx.settings.tile_scale as f32;
    for row in &rows {
        // `bb.get()` twice: an empty or one-byte blob underflows and ends
        // the loop.
        if row.data.len() < 2 {
            out.error = Some("java.nio.BufferUnderflowException".into());
            return out;
        }
        match row.load(class, (32.0 * ts, 96.0 * ts), ctx.items) {
            Ok(Some(v)) => out.added.push(chunk.vehicle_loaded(row, v)),
            Ok(None) => {}
            Err(e) => {
                eprintln!(
                    "ERROR: vehicle {} is being deleted because an error occurred loading it: {}",
                    row.id, e.0
                );
                remove_vehicle(conn, row.id);
                out.removed.push((row.id, e.0));
            }
        }
    }
    if !complete {
        out.error = Some("java.lang.NullPointerException".into());
    }
    out
}

// ---- VehiclesDB2.unloadChunk ------------------------------------------------

/// `pstmt.setFloat`: the `float` widened to the `REAL` sqlite stores.
fn real(v: f32) -> Value {
    Value::Real(f64::from(v))
}

/// `SQLStore.isInDB(id)`: `SELECT 1 FROM vehicles WHERE id=?`.
pub fn is_in_db(conn: &mut Connection, id: i32) -> pz_sqlite::Result<bool> {
    let t = conn.table("vehicles")?;
    Ok(conn.row(&t, i64::from(id))?.is_some())
}

/// The columns `addToDB`/`updateDB` bind, in their statements' order
/// (`wx, wy, x, y, worldversion, inMeta, data`).
fn row_values(v: &VehicleRow) -> Vec<(&'static str, Value)> {
    vec![
        ("wx", Value::Integer(i64::from(v.wx))),
        ("wy", Value::Integer(i64::from(v.wy))),
        ("x", real(v.x)),
        ("y", real(v.y)),
        ("worldversion", Value::Integer(i64::from(v.world_version))),
        ("inMeta", Value::Integer(i64::from(v.in_meta))),
        ("data", Value::Blob(v.data.clone())),
    ]
}

/// `SQLStore.addToDB`: `INSERT INTO vehicles(wx,wy,x,y,worldversion,inMeta,data,id)`,
/// committed; rolled back on failure.
pub fn add_to_db(conn: &mut Connection, v: &VehicleRow) -> pz_sqlite::Result<()> {
    let mut vals = row_values(v);
    vals.push(("id", Value::Integer(i64::from(v.id))));
    let r = conn.insert("vehicles", &vals).and_then(|_| conn.commit());
    if r.is_err() {
        conn.rollback();
    }
    r
}

/// `SQLStore.updateDB`: `UPDATE vehicles SET wx = ?, wy = ?, x = ?, y = ?,
/// worldversion = ?, inMeta = ?, data = ? WHERE id=?`, committed; rolled
/// back on failure.
pub fn update_db(conn: &mut Connection, v: &VehicleRow) -> pz_sqlite::Result<()> {
    let vals = row_values(v);
    let r = conn
        .update("vehicles", i64::from(v.id), &vals)
        .and_then(|_| conn.commit());
    if r.is_err() {
        conn.rollback();
    }
    r
}

/// `SQLStore.updateVehicle(vehicleBuffer)`: update the row when the id is in
/// the table, else insert it. A failure is rolled back and returned (the
/// original logs it and carries on).
pub fn update_vehicle(conn: &mut Connection, v: &VehicleRow) -> pz_sqlite::Result<()> {
    let r = is_in_db(conn, v.id).and_then(|inside| {
        if inside {
            update_db(conn, v)
        } else {
            add_to_db(conn, v)
        }
    });
    if r.is_err() {
        conn.rollback();
    }
    r
}

/// What `BaseVehicle.getScript()` tells `shouldUpdateInMeta` and `save`
/// about a vehicle's `VehicleScript` (`script.getSounds()`,
/// `script.getLightbar()`).
pub struct VehicleScriptFacts<'a> {
    /// `sounds.alarmEnable`, `sounds.alarm`, `sounds.alarmLoop`.
    pub sounds: AlarmSounds<'a>,
    /// `lightbar.enable` (`hasSiren`).
    pub lightbar_enable: bool,
}

/// The `VehicleScript` attached to a vehicle (`BaseVehicle.script`).
pub trait VehicleScripts {
    /// `None`: no script is attached. `BaseVehicle.load` only reads the
    /// script name; the script is attached by `setScript`, which runs when
    /// the vehicle is added to the world (`createPhysics`).
    fn script(&self, script_name: &str) -> Option<VehicleScriptFacts<'_>>;
}

/// No vehicle has a script attached: every vehicle is as `BaseVehicle.load`
/// left it, never added to the world.
pub struct NoVehicleScripts;

impl VehicleScripts for NoVehicleScripts {
    fn script(&self, _script_name: &str) -> Option<VehicleScriptFacts<'_>> {
        None
    }
}

/// `BaseVehicle.getBatteryCharge()`: the current uses of the item in the
/// part with id `Battery` when it is a `DrainableComboItem`, else 0. An
/// item kept as bytes cannot be asked (error).
pub fn battery_charge(v: &BaseVehicle) -> Result<f32, String> {
    // `VehicleParts.add` keeps the last part with that id as `battery`.
    let Some(part) = v.parts.iter().rev().find(|p| p.id() == "Battery") else {
        return Ok(0.0);
    };
    match &part.item {
        None => Ok(0.0),
        Some(ItemBlob::Item(i)) => Ok(match i.template.class {
            ItemClass::Drainable => i.get_current_uses_float(),
            _ => 0.0,
        }),
        Some(ItemBlob::Raw(_)) => Err("the battery item was kept as bytes".into()),
    }
}

/// `BaseVehicle.shouldUpdateInMeta()`: `isAlarmActive() || isSirenActive()
/// || isEngineRunning()`, evaluated in that order. Without a script,
/// `hasAlarm()` dereferences the null script
/// (`java.lang.NullPointerException`).
pub fn should_update_in_meta(
    v: &BaseVehicle,
    script: Option<&VehicleScriptFacts<'_>>,
) -> Result<bool, String> {
    let Some(s) = script else {
        return Err("java.lang.NullPointerException".into());
    };
    // `hasAlarm() && vehicleAlarm.isActive() && getBatteryCharge() > 0`.
    if s.sounds.alarm_enable && v.alarm_start_time > 0.0 && battery_charge(v)? > 0.0 {
        return Ok(true);
    }
    // `hasSiren() && lightbarSirenMode.isEnable() && getBatteryCharge() > 0`.
    if s.lightbar_enable && v.lightbar_siren_mode != 0 && battery_charge(v)? > 0.0 {
        return Ok(true);
    }
    // `getEngine() != null && getVehicleEngine().isRunning()`.
    Ok(v.engine.as_ref().is_some_and(|e| e.running))
}

/// `VehiclesDB2.VehicleBuffer.set(vehicle)` for one entry of
/// [`IsoChunk::vehicles`]: the row `unloadChunk` stores.
pub trait ChunkVehicleSaver {
    /// The row for `vehicle`, standing in `chunk` (`vehicle.chunk`), or the
    /// exception `set` throws (`unloadChunk` logs it and goes on to the
    /// next vehicle).
    fn vehicle_buffer(
        &self,
        vehicle: &VehicleHandle,
        chunk: &IsoChunk,
        ctx: &SaveCtx<'_>,
    ) -> Result<VehicleRow, String>;
}

/// [`ChunkVehicleSaver`] for the vehicles chunk loading put in the list
/// ([`LoadedVehicle`]), with the scripts `scripts` says are attached.
///
/// A vehicle the vehicle runtime replaced with its own object is not a
/// [`LoadedVehicle`]; the runtime saves those with its own saver.
pub struct LoadedVehicleSaver<'a> {
    pub scripts: &'a dyn VehicleScripts,
}

impl ChunkVehicleSaver for LoadedVehicleSaver<'_> {
    fn vehicle_buffer(
        &self,
        vehicle: &VehicleHandle,
        chunk: &IsoChunk,
        ctx: &SaveCtx<'_>,
    ) -> Result<VehicleRow, String> {
        let Ok(lv) = vehicle.clone().downcast::<LoadedVehicle>() else {
            return Err("not a vehicle chunk loading made (no saver)".into());
        };
        // `sqlId`: rows carry theirs; a vehicle from an old chunk file's
        // vehicle list gets one from `VehiclesDB2.addVehicle` when it is
        // added to the world (`doLoadGridsquare`), which is not ported.
        let VehicleOrigin::Database { sql_id } = lv.origin else {
            return Err("java.lang.AssertionError: vehicle.sqlId >= 1".into());
        };
        let v = &lv.vehicle;
        let script = self.scripts.script(&v.script_name);
        let in_meta = should_update_in_meta(v, script.as_ref())?;
        let mut w = Writer::new();
        let class = ctx.tables.class_id("Vehicle");
        let square = lv.square.map(|(x, y, _)| (x, y));
        v.save(
            &mut w,
            class,
            square,
            script.as_ref().map(|s| &s.sounds),
            ctx.items,
        )
        .map_err(|e| e.0)?;
        Ok(VehicleRow {
            id: sql_id,
            wx: chunk.wx,
            wy: chunk.wy,
            x: v.base.x,
            y: v.base.y,
            world_version: super::WORLD_VERSION,
            in_meta,
            data: w.buf,
        })
    }
}

/// What [`unload_chunk`] did.
#[derive(Debug, Default, PartialEq)]
pub struct ChunkVehiclesUnloaded {
    /// The ids of the rows written, in list order.
    pub saved: Vec<i32>,
    /// The vehicles that were not written: their index in the list and the
    /// exception (`VehicleBuffer.set` or the statement).
    pub errors: Vec<(usize, String)>,
}

/// `VehiclesDB2.unloadChunk(chunk)` → `WorldStreamerThread.unloadChunk`:
/// every vehicle in the chunk's list is written with
/// `SQLStore.updateVehicle` ([`update_vehicle`]). `IsoChunk.Save` calls it
/// for every save that is not a hot save, after the squares and erosion
/// data are written and before the chunk file. `conn` is `None` when the
/// store has no connection (`Core.isNoSave`), which writes nothing.
pub fn unload_chunk(
    conn: Option<&mut Connection>,
    chunk: &IsoChunk,
    ctx: &SaveCtx<'_>,
    saver: &dyn ChunkVehicleSaver,
) -> ChunkVehiclesUnloaded {
    let mut out = ChunkVehiclesUnloaded::default();
    let Some(conn) = conn else {
        return out;
    };
    for (i, v) in chunk.vehicles.iter().enumerate() {
        let row = match saver.vehicle_buffer(v, chunk, ctx) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ERROR: VehiclesDB2.unloadChunk: {e}");
                out.errors.push((i, e));
                continue;
            }
        };
        match update_vehicle(conn, &row) {
            Ok(()) => out.saved.push(row.id),
            Err(e) => {
                eprintln!("ERROR: vehicles.db: {e}");
                out.errors.push((i, e.to_string()));
            }
        }
    }
    out
}
