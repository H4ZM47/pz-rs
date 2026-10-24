//! The game's writes to a save's `players.db` and `vehicles.db`:
//! `PlayerDBHelper.create` / `setPlayer1` / `removePlayer`, `PlayerDB`'s
//! `SQLPlayerStore` (saving and loading local players) and the
//! `VehiclesDB2.SQLStore` statements, on `pz-sqlite` connections.
//!
//! The statements, bindings and commit points follow the originals:
//! prepared-statement parameters are bound as sqlite-jdbc binds them
//! (`setFloat` as a double, `setBoolean` as 0/1, `setString(null)` as NULL)
//! and the stores run with auto-commit off, committing after each row and
//! rolling back on failure. Where the original calls `System.exit(1)` after
//! a failed database setup, these functions return the error instead.

use crate::savefile::{absolute_path, exists};
use pz_fs::jfile::{self, SEP};
use pz_sqlite::{Connection, Error, Result, Row, Table, Value};
use std::collections::BTreeSet;

/// `localPlayers` / `networkPlayers` / `vehicles` as `PlayerDBHelper` and
/// `VehiclesDB2` create them.
pub const CREATE_LOCAL_PLAYERS: &str = "CREATE TABLE localPlayers (id   INTEGER PRIMARY KEY NOT NULL,name STRING,wx    INTEGER,wy    INTEGER,x    FLOAT,y    FLOAT,z    FLOAT,worldversion    INTEGER,data BLOB,isDead BOOLEAN);";
pub const CREATE_NETWORK_PLAYERS: &str = "CREATE TABLE networkPlayers (id   INTEGER PRIMARY KEY NOT NULL,world TEXT,username TEXT,playerIndex   INTEGER,name STRING,steamid STRING,x    FLOAT,y    FLOAT,z    FLOAT,worldversion    INTEGER,data BLOB,isDead BOOLEAN,splitUsername STRING);";
pub const CREATE_NETWORK_PLAYERS_INDEX: &str =
    "CREATE INDEX inpusername ON networkPlayers (username);";
pub const ADD_SPLIT_USERNAME: &str = "ALTER TABLE 'networkPlayers' ADD 'splitUsername' STRING";
pub const CREATE_VEHICLES: &str = "CREATE TABLE vehicles (id   INTEGER PRIMARY KEY NOT NULL,wx    INTEGER,wy    INTEGER,x    FLOAT,y    FLOAT,inMeta BOOLEAN NULL DEFAULT FALSE,worldversion    INTEGER,data BLOB);";
pub const CREATE_VEHICLES_WX: &str = "CREATE INDEX ivwx ON vehicles (wx);";
pub const CREATE_VEHICLES_WY: &str = "CREATE INDEX ivwy ON vehicles (wy);";
pub const ADD_IN_META: &str = "ALTER TABLE 'vehicles' ADD 'inMeta' BOOLEAN NULL DEFAULT FALSE";
const JOURNAL_MODE: &str = "PRAGMA JOURNAL_MODE=TRUNCATE;";

fn log_exception(what: &str, e: &dyn std::fmt::Display) {
    eprintln!("ERROR: {what}: {e}");
}

/// `File.setReadable/setExecutable/setWritable(true, false)`: adds the
/// permission for everybody; nothing happens to a missing file.
fn add_permissions(path: &str, bits: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = jfile::normalize(path);
        if let Ok(m) = std::fs::metadata(&p) {
            let mut perm = m.permissions();
            perm.set_mode(perm.mode() | bits);
            let _ = std::fs::set_permissions(&p, perm);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, bits);
    }
}

/// `PZMath.fastfloor(float)`.
pub fn fastfloor(v: f32) -> i32 {
    let xi = v as i32;
    if v < xi as f32 {
        xi.wrapping_sub(1)
    } else {
        xi
    }
}

/// `setFloat`: sqlite-jdbc binds the float widened to a double.
fn float(v: f32) -> Value {
    Value::Real(f64::from(v))
}

fn string(s: Option<&str>) -> Value {
    s.map_or(Value::Null, |s| Value::Text(s.to_string()))
}

/// `rs.getFloat`: `(float) sqlite3_column_double`.
fn get_float(v: &Value) -> f32 {
    (match v {
        Value::Null => 0.0,
        Value::Integer(i) => *i as f64,
        Value::Real(r) => *r,
        Value::Text(s) => pz_kahlua::number::java_parse_double(s.trim()).unwrap_or(0.0),
        Value::Blob(b) => {
            pz_kahlua::number::java_parse_double(String::from_utf8_lossy(b).trim()).unwrap_or(0.0)
        }
    }) as f32
}

/// The table's `has column` check (`DatabaseMetaData.getColumns`).
fn has_column(conn: &mut Connection, table: &str, column: &str) -> Result<bool> {
    match conn.table(table) {
        Ok(t) => Ok(t.column(column).is_ok()),
        Err(Error::NoSuchTable(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// `new File(dir).mkdirs()` unless it exists.
fn mkdirs(dir: &str) {
    if !exists(dir) {
        let _ = std::fs::create_dir_all(jfile::normalize(dir));
    }
}

/// Opens `<save_dir>/<name>`, creating it with `create` statements when
/// missing (`createNewFile` then the statements in auto-commit).
fn open_or_create(save_dir: &str, name: &str, create: &[&str]) -> Result<Connection> {
    mkdirs(save_dir);
    let path = format!("{save_dir}{SEP}{name}");
    add_permissions(&path, 0o444);
    add_permissions(&path, 0o111);
    add_permissions(&path, 0o222);
    let abs = absolute_path(&path);
    if !exists(&path) {
        let mut c = Connection::open(&abs)?;
        for s in create {
            c.execute(s)?;
        }
        return Ok(c);
    }
    Connection::open(&abs)
}

// ---- PlayerDBHelper -------------------------------------------------------------

/// `PlayerDBHelper.create()` for `save_dir` (`getCurrentSaveDir()`): the
/// connection with auto-commit off.
pub fn create_players_db(save_dir: &str) -> Result<Connection> {
    let mut c = open_or_create(
        save_dir,
        "players.db",
        &[
            CREATE_LOCAL_PLAYERS,
            CREATE_NETWORK_PLAYERS,
            CREATE_NETWORK_PLAYERS_INDEX,
        ],
    )?;
    if !has_column(&mut c, "networkPlayers", "splitUsername")? {
        c.execute(ADD_SPLIT_USERNAME)?;
    }
    c.execute(JOURNAL_MODE)?;
    c.set_autocommit(false)?;
    Ok(c)
}

/// `SELECT id FROM localPlayers` → `getInt(1)` per row.
fn local_player_ids(c: &mut Connection) -> Result<Vec<i32>> {
    let t = c.table("localPlayers")?;
    let id = t.column("id")?;
    Ok(c.rows(&t)?.iter().map(|r| r.get(&t, id).as_i32()).collect())
}

fn open_existing(save_dir: &str) -> Option<Result<Connection>> {
    let path = format!("{save_dir}{SEP}players.db");
    if !exists(&path) {
        return None;
    }
    add_permissions(&path, 0o444);
    Some(Connection::open(absolute_path(&path)))
}

/// `PlayerDBHelper.setPlayer1(saveDir, sqlID)`: makes `sql_id` the player
/// with id 1, swapping ids with the current player 1; `sql_id <= 0` moves
/// player 1 out of the way. Each `UPDATE` commits on its own.
pub fn set_player1(no_save: bool, save_dir: &str, sql_id: i32) -> Result<()> {
    if no_save || sql_id == 1 {
        return Ok(());
    }
    let Some(conn) = open_existing(save_dir) else {
        return Ok(());
    };
    let mut c = conn?;
    let mut has_player1 = false;
    let mut has_selected = false;
    let mut non_player1 = -1;
    let mut max_id = -1i32;
    for id in local_player_ids(&mut c)? {
        if id == 1 {
            has_player1 = true;
        } else if non_player1 == -1 || non_player1 > id {
            non_player1 = id;
        }
        if id == sql_id {
            has_selected = true;
        }
        max_id = max_id.max(id);
    }
    let _ = non_player1;
    let set_id = |c: &mut Connection, to: i32, from: i32| -> Result<()> {
        c.update(
            "localPlayers",
            i64::from(from),
            &[("id", Value::Integer(i64::from(to)))],
        )
        .map(|_| ())
    };
    let tmp = max_id.wrapping_add(1);
    if sql_id <= 0 {
        if has_player1 {
            set_id(&mut c, tmp, 1)?;
        }
        return Ok(());
    }
    if !has_selected {
        return Ok(());
    }
    if has_player1 {
        set_id(&mut c, tmp, 1)?;
        set_id(&mut c, 1, sql_id)?;
        set_id(&mut c, sql_id, tmp)?;
    } else {
        set_id(&mut c, 1, sql_id)?;
    }
    Ok(())
}

/// `PlayerDBHelper.removePlayer(saveDir, name, world)`: `DELETE FROM
/// networkPlayers WHERE world = ? and username = ?`.
pub fn remove_player(save_dir: &str, name: Option<&str>, world: Option<&str>) -> Result<()> {
    let Some(conn) = open_existing(save_dir) else {
        return Ok(());
    };
    let mut c = conn?;
    // TEXT columns compare as text with text parameters; NULL matches
    // nothing.
    let eq = |t: &Table, r: &Row, col: &str, want: Option<&str>| {
        let Ok(i) = t.column(col) else { return false };
        matches!((r.get(t, i), want), (Value::Text(s), Some(w)) if s == w)
    };
    c.delete_where("networkPlayers", |t, r| {
        eq(t, r, "world", world) && eq(t, r, "username", name)
    })?;
    Ok(())
}

// ---- PlayerDB ---------------------------------------------------------------------

/// `PlayerDB.PlayerData`.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerData {
    pub sql_id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub is_dead: bool,
    pub name: Option<String>,
    pub world_version: i32,
    /// The serialized player (`IsoPlayer.save`).
    pub bytes: Vec<u8>,
}

impl Default for PlayerData {
    fn default() -> Self {
        PlayerData {
            sql_id: -1,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            is_dead: false,
            name: None,
            world_version: 0,
            bytes: Vec::new(),
        }
    }
}

/// `PlayerDB.SQLPlayerStore`.
#[derive(Default)]
pub struct PlayerStore {
    pub conn: Option<Connection>,
}

impl PlayerStore {
    /// `init(usedIDs)`: opens the database and lists the ids in use.
    pub fn init(&mut self, no_save: bool, save_dir: &str, used: &mut BTreeSet<i32>) -> Result<()> {
        used.clear();
        if no_save {
            return Ok(());
        }
        let mut c = create_players_db(save_dir)?;
        used.extend(local_player_ids(&mut c)?);
        self.conn = Some(c);
        Ok(())
    }

    /// `Reset()`: closes the connection (uncommitted changes are lost).
    pub fn reset(&mut self) {
        self.conn = None;
    }

    /// `save(playerData)`: updates the row or adds it.
    pub fn save(&mut self, p: &PlayerData) -> Result<()> {
        if self.conn.is_none() {
            return Ok(());
        }
        if self.is_in_db(p.sql_id)? {
            self.update(p)
        } else {
            self.add(p)
        }
    }

    /// `SELECT 1 FROM localPlayers WHERE id=?`.
    pub fn is_in_db(&mut self, id: i32) -> Result<bool> {
        let c = self
            .conn
            .as_mut()
            .ok_or_else(|| Error::Sql("closed".into()))?;
        let t = c.table("localPlayers")?;
        Ok(c.row(&t, i64::from(id))?.is_some())
    }

    fn values(p: &PlayerData, wx: i32, wy: i32) -> Vec<(&'static str, Value)> {
        vec![
            ("wx", Value::Integer(i64::from(wx))),
            ("wy", Value::Integer(i64::from(wy))),
            ("x", float(p.x)),
            ("y", float(p.y)),
            ("z", float(p.z)),
            ("worldversion", Value::Integer(i64::from(p.world_version))),
            ("data", Value::Blob(p.bytes.clone())),
            ("isDead", Value::Integer(i64::from(p.is_dead))),
            ("name", string(p.name.as_deref())),
        ]
    }

    /// Runs one statement, then commits; rolls back on failure.
    fn commit_one(&mut self, f: impl FnOnce(&mut Connection) -> Result<()>) -> Result<()> {
        let Some(c) = self.conn.as_mut() else {
            return Ok(());
        };
        let r = f(c).and_then(|_| c.commit());
        if r.is_err() {
            c.rollback();
        }
        r
    }

    /// `add`: `INSERT INTO localPlayers(wx,wy,x,y,z,worldversion,data,isDead,name,id)`,
    /// with `wx = fastfloor(x) / 8`.
    pub fn add(&mut self, p: &PlayerData) -> Result<()> {
        if p.sql_id < 1 {
            return Ok(());
        }
        let mut v = Self::values(p, fastfloor(p.x) / 8, fastfloor(p.y) / 8);
        v.push(("id", Value::Integer(i64::from(p.sql_id))));
        self.commit_one(|c| c.insert("localPlayers", &v).map(|_| ()))
    }

    /// `update`: `UPDATE localPlayers SET ... WHERE id=?`, with
    /// `wx = fastfloor(x / 8f)`.
    pub fn update(&mut self, p: &PlayerData) -> Result<()> {
        if p.sql_id < 1 {
            return Ok(());
        }
        let v = Self::values(p, fastfloor(p.x / 8.0), fastfloor(p.y / 8.0));
        let id = i64::from(p.sql_id);
        self.commit_one(|c| c.update("localPlayers", id, &v).map(|_| ()))
    }

    fn select(&mut self, id: i32) -> Result<Option<(Table, Row)>> {
        let Some(c) = self.conn.as_mut() else {
            return Ok(None);
        };
        let t = c.table("localPlayers")?;
        Ok(c.row(&t, i64::from(id))?.map(|r| (t, r)))
    }

    fn fill(p: &mut PlayerData, t: &Table, r: &Row) -> Result<()> {
        let col = |n: &str| t.column(n).map(|i| r.get(t, i));
        p.world_version = col("worldversion")?.as_i32();
        p.x = get_float(&col("x")?);
        p.y = get_float(&col("y")?);
        p.z = get_float(&col("z")?);
        p.is_dead = col("isDead")?.as_i32() != 0;
        p.name = col("name")?.as_text();
        Ok(())
    }

    /// `load`: `SELECT data,worldversion,x,y,z,isDead,name FROM localPlayers
    /// WHERE id=?`. A NULL `data` fails as the original's null stream does.
    pub fn load(&mut self, p: &mut PlayerData) -> Result<bool> {
        let Some((t, r)) = self.select(p.sql_id)? else {
            return Ok(false);
        };
        p.bytes = match r.get(&t, t.column("data")?) {
            Value::Null => return Err(Error::Sql("java.lang.NullPointerException".into())),
            Value::Blob(b) => b,
            Value::Text(s) => s.into_bytes(),
            v => v.as_text().unwrap_or_default().into_bytes(),
        };
        Self::fill(p, &t, &r)?;
        Ok(true)
    }

    /// `loadEverythingExceptBytes`.
    pub fn load_everything_except_bytes(&mut self, p: &mut PlayerData) -> Result<bool> {
        let Some((t, r)) = self.select(p.sql_id)? else {
            return Ok(false);
        };
        Self::fill(p, &t, &r)?;
        Ok(true)
    }
}

/// `PlayerDB`'s id bookkeeping around the store.
pub struct PlayerDb {
    pub store: PlayerStore,
    pub used_ids: BTreeSet<i32>,
}

impl PlayerDb {
    /// `new PlayerDB()` (`create()`): nothing is opened with saving off;
    /// id 1 is always reserved.
    pub fn new(no_save: bool, save_dir: &str) -> PlayerDb {
        let mut db = PlayerDb {
            store: PlayerStore::default(),
            used_ids: BTreeSet::new(),
        };
        if no_save {
            return db;
        }
        if let Err(e) = db.store.init(no_save, save_dir, &mut db.used_ids) {
            log_exception("PlayerDB", &e);
        }
        db.used_ids.insert(1);
        db
    }

    /// `allocateID`: the smallest unused id from 1.
    pub fn allocate_id(&mut self) -> i32 {
        let mut i = 1;
        while self.used_ids.contains(&i) {
            i += 1;
        }
        self.used_ids.insert(i);
        i
    }

    /// `savePlayerAsync` + `updateWorldStreamer` for one player: gives the
    /// player an id when it has none, then stores it. Failures are logged.
    pub fn save_player(&mut self, p: &mut PlayerData) {
        if p.sql_id == -1 {
            p.sql_id = self.allocate_id();
        }
        if let Err(e) = self.store.save(p) {
            log_exception("PlayerDB", &e);
        }
    }

    /// `loadPlayer` without the `IsoPlayer.load`: the stored row, `None`
    /// when missing or unreadable (logged).
    pub fn load_player(&mut self, sql_id: i32) -> Option<PlayerData> {
        let mut p = PlayerData {
            sql_id,
            ..Default::default()
        };
        match self.store.load(&mut p) {
            Ok(true) => Some(p),
            Ok(false) => None,
            Err(e) => {
                log_exception("PlayerDB", &e);
                None
            }
        }
    }

    /// `close()`.
    pub fn close(&mut self) {
        self.store.reset();
    }
}

// ---- VehiclesDB2.SQLStore ------------------------------------------------------------

/// `VehiclesDB2.VehicleBuffer`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleBuffer {
    pub id: i32,
    pub wx: i32,
    pub wy: i32,
    pub x: f32,
    pub y: f32,
    pub world_version: i32,
    pub in_meta: bool,
    pub bytes: Vec<u8>,
}

/// `VehiclesDB2.SQLStore`.
#[derive(Default)]
pub struct VehicleStore {
    pub conn: Option<Connection>,
}

impl VehicleStore {
    /// `create()` for `save_dir`. The `inMeta` upgrade runs after
    /// auto-commit is off, so it commits with the next row.
    pub fn create(&mut self, save_dir: &str) -> Result<()> {
        let mut c = open_or_create(
            save_dir,
            "vehicles.db",
            &[CREATE_VEHICLES, CREATE_VEHICLES_WX, CREATE_VEHICLES_WY],
        )?;
        c.execute(JOURNAL_MODE)?;
        c.set_autocommit(false)?;
        let r = has_column(&mut c, "vehicles", "inMeta").and_then(|has| {
            if has {
                Ok(())
            } else {
                c.execute(ADD_IN_META)
            }
        });
        if let Err(e) = r {
            log_exception("vehicles.db", &e);
        }
        self.conn = Some(c);
        Ok(())
    }

    fn commit_one(&mut self, f: impl FnOnce(&mut Connection) -> Result<()>) -> Result<()> {
        let Some(c) = self.conn.as_mut() else {
            return Ok(());
        };
        let r = f(c).and_then(|_| c.commit());
        if r.is_err() {
            c.rollback();
        }
        r
    }

    /// `SELECT 1 FROM vehicles WHERE id=?`.
    pub fn is_in_db(&mut self, id: i32) -> Result<bool> {
        let c = self
            .conn
            .as_mut()
            .ok_or_else(|| Error::Sql("closed".into()))?;
        let t = c.table("vehicles")?;
        Ok(c.row(&t, i64::from(id))?.is_some())
    }

    fn values(v: &VehicleBuffer) -> Vec<(&'static str, Value)> {
        vec![
            ("wx", Value::Integer(i64::from(v.wx))),
            ("wy", Value::Integer(i64::from(v.wy))),
            ("x", float(v.x)),
            ("y", float(v.y)),
            ("worldversion", Value::Integer(i64::from(v.world_version))),
            ("inMeta", Value::Integer(i64::from(v.in_meta))),
            ("data", Value::Blob(v.bytes.clone())),
        ]
    }

    /// `addToDB`.
    pub fn add_to_db(&mut self, v: &VehicleBuffer) -> Result<()> {
        let mut vals = Self::values(v);
        vals.push(("id", Value::Integer(i64::from(v.id))));
        self.commit_one(|c| c.insert("vehicles", &vals).map(|_| ()))
    }

    /// `updateDB`.
    pub fn update_db(&mut self, v: &VehicleBuffer) -> Result<()> {
        let vals = Self::values(v);
        let id = i64::from(v.id);
        self.commit_one(|c| c.update("vehicles", id, &vals).map(|_| ()))
    }

    /// `updateVehicle`: update or add; failures are logged.
    pub fn update_vehicle(&mut self, v: &VehicleBuffer) {
        if self.conn.is_none() {
            return;
        }
        let r = self.is_in_db(v.id).and_then(|inside| {
            if inside {
                self.update_db(v)
            } else {
                self.add_to_db(v)
            }
        });
        if let Err(e) = r {
            log_exception("vehicles.db", &e);
        }
    }

    /// `removeVehicle(sqlID)`.
    pub fn remove_vehicle(&mut self, sql_id: i32) {
        if self.conn.is_none() || sql_id < 1 {
            return;
        }
        if let Err(e) = self.commit_one(|c| c.delete("vehicles", i64::from(sql_id)).map(|_| ())) {
            log_exception("vehicles.db", &e);
        }
    }

    /// The end of `importPlayersFromOldDB`: drops the player tables of a
    /// pre-`players.db` save (only when `localPlayers` exists).
    pub fn drop_old_player_tables(&mut self) {
        let Some(c) = self.conn.as_mut() else {
            return;
        };
        match c.table("localPlayers") {
            Ok(_) => {}
            Err(Error::NoSuchTable(_)) => return,
            Err(e) => return log_exception("vehicles.db", &e),
        }
        let r = c
            .execute("DROP TABLE localPlayers")
            .and_then(|_| c.execute("DROP TABLE networkPlayers"))
            .and_then(|_| c.commit());
        if let Err(e) = r {
            log_exception("vehicles.db", &e);
        }
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        self.conn = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> String {
        let d = std::env::temp_dir().join(format!("pz-player-db-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("Sandbox").join("w").to_string_lossy().into_owned()
    }

    fn player(id: i32, name: &str, x: f32, dead: bool) -> PlayerData {
        PlayerData {
            sql_id: id,
            x,
            y: -x,
            z: 0.0,
            is_dead: dead,
            name: Some(name.into()),
            world_version: 244,
            bytes: (0..(name.len() * 3000)).map(|i| i as u8).collect(),
        }
    }

    #[test]
    fn store_save_load_and_set_player1() {
        let dir = tmp("store");
        let mut db = PlayerDb::new(false, &dir);
        assert!(exists(&format!("{dir}{SEP}players.db")));
        assert_eq!(db.used_ids.iter().copied().collect::<Vec<_>>(), [1]);
        let mut p = player(-1, "Kate Smith", 10850.5, false);
        db.save_player(&mut p);
        assert_eq!(p.sql_id, 2);
        let mut q = player(1, "Bob", -12.5, true);
        db.save_player(&mut q);
        // add: fastfloor(x) / 8 truncates toward zero; update floors x / 8.
        let c = db.store.conn.as_mut().unwrap();
        let t = c.table("localPlayers").unwrap();
        let wx = |c: &mut Connection, id| c.row(&t, id).unwrap().unwrap().get(&t, 2);
        assert_eq!(wx(c, 1), Value::Integer(-1));
        db.save_player(&mut q);
        let c = db.store.conn.as_mut().unwrap();
        assert_eq!(wx(c, 1), Value::Integer(-2));
        let got = db.load_player(2).unwrap();
        assert_eq!(got, p);
        let mut e = PlayerData {
            sql_id: 1,
            ..Default::default()
        };
        assert!(db.store.load_everything_except_bytes(&mut e).unwrap());
        assert!(e.is_dead && e.bytes.is_empty() && e.name.as_deref() == Some("Bob"));
        assert!(db.load_player(7).is_none());
        db.close();

        // Reopening sees the ids; setPlayer1 swaps 2 into slot 1.
        let db = PlayerDb::new(false, &dir);
        assert_eq!(db.used_ids.iter().copied().collect::<Vec<_>>(), [1, 2]);
        drop(db);
        set_player1(false, &dir, 2).unwrap();
        let rows = crate::savefile::get_players(false, &dir).unwrap();
        let names: Vec<_> = rows
            .iter()
            .map(|r| (r.sql_id, r.name.clone().unwrap()))
            .collect();
        assert_eq!(names, [(1, "Kate Smith".into()), (2, "Bob".into())]);
        // -1 moves player 1 to max + 1; a missing id changes nothing.
        set_player1(false, &dir, -1).unwrap();
        set_player1(false, &dir, 9).unwrap();
        let ids: Vec<_> = crate::savefile::get_players(false, &dir)
            .unwrap()
            .iter()
            .map(|r| r.sql_id)
            .collect();
        assert_eq!(ids, [2, 3]);
        set_player1(true, &dir, 3).unwrap();
        assert!(set_player1(false, &format!("{dir}x"), 3).is_ok());
        let _ = std::fs::remove_dir_all(std::path::Path::new(&dir).parent().unwrap());
    }

    #[test]
    fn vehicles_and_network_players() {
        let dir = tmp("veh");
        let mut v = VehicleStore::default();
        v.create(&dir).unwrap();
        for id in 1..=30 {
            v.update_vehicle(&VehicleBuffer {
                id,
                wx: id % 4,
                wy: -id,
                x: id as f32 * 8.5,
                y: 1.0,
                world_version: 244,
                in_meta: id % 2 == 0,
                bytes: vec![id as u8; 2000],
            });
        }
        v.update_vehicle(&VehicleBuffer {
            id: 3,
            wx: 99,
            ..Default::default()
        });
        v.remove_vehicle(4);
        v.remove_vehicle(0);
        v.drop_old_player_tables();
        let c = v.conn.as_mut().unwrap();
        let t = c.table("vehicles").unwrap();
        let rows = c.rows(&t).unwrap();
        assert_eq!(rows.len(), 29);
        assert_eq!(rows[2].get(&t, 1), Value::Integer(99));
        v.reset();

        // removePlayer on networkPlayers (indexed by username).
        let mut c = create_players_db(&dir).unwrap();
        for (w, u) in [("A", "bob"), ("B", "bob"), ("A", "kate")] {
            c.insert(
                "networkPlayers",
                &[
                    ("world", Value::Text(w.into())),
                    ("username", Value::Text(u.into())),
                ],
            )
            .unwrap();
        }
        c.commit().unwrap();
        drop(c);
        remove_player(&dir, Some("bob"), Some("A")).unwrap();
        remove_player(&dir, None, Some("A")).unwrap();
        let mut c = Connection::open(format!("{dir}{SEP}players.db")).unwrap();
        let t = c.table("networkPlayers").unwrap();
        let left: Vec<_> = c
            .rows(&t)
            .unwrap()
            .iter()
            .map(|r| (r.rowid, r.get(&t, 1), r.get(&t, 2)))
            .collect();
        assert_eq!(
            left,
            [
                (2, Value::Text("B".into()), Value::Text("bob".into())),
                (3, Value::Text("A".into()), Value::Text("kate".into()))
            ]
        );
        let _ = std::fs::remove_dir_all(std::path::Path::new(&dir).parent().unwrap());
    }
}
