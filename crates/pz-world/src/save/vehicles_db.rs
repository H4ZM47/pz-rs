//! The reading side of `VehiclesDB2.SQLStore` on a save's `vehicles.db`:
//! `initUsedIDs`, and `loadChunk` with `WorldStreamerThread.vehicleLoaded`,
//! which puts a loaded chunk's vehicles into [`IsoChunk::vehicles`].
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
//! [`crate::save::chunk_file::finish_chunk_load`].

use std::collections::BTreeSet;

use pz_sqlite::{Connection, Row, Table, Value};

use super::vehicle::VehicleRow;
use super::SaveCtx;
use crate::chunk::IsoChunk;
use crate::chunk_vehicles::VehicleHandle;

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
