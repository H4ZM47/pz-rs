//! `zombie.savefile.AccountDBHelper`: the saved server list in
//! `<cache>/db/ServerList.db` (`ServerListSteam.db` in Steam mode).
//!
//! Like the original, every operation opens the database (creating it with
//! its two tables when the file is missing), turns autocommit off and
//! imports a legacy `<cache>/Lua/ServerList.txt` if one exists, then closes
//! it again. Operations that change rows commit, so they also commit
//! whatever the import inserted; `getServerList` never commits, so an
//! import is visible to that one read and then rolled back.

use crate::datetime::LocalDateTime;
use crate::server::{Account, Server};
use pz_sqlite::{Connection, Table, Value};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const CREATE_SERVER: &str = "CREATE TABLE IF NOT EXISTS server (\n id INTEGER PRIMARY KEY AUTOINCREMENT,\n name TEXT NOT NULL,\n ip TEXT NOT NULL,\n port INTEGER NOT NULL,\n serverPassword TEXT,\n description TEXT,\n mods TEXT,\n icon BLOB,\n banner BLOB,\n panelBackground BLOB,\n screenBackground BLOB,\n lastOnline TEXT,\n lastDataUpdate TEXT\n);";
const CREATE_ACCOUNT: &str = "CREATE TABLE IF NOT EXISTS account (\n id INTEGER PRIMARY KEY AUTOINCREMENT,\n serverId INTEGER NOT NULL,\n playerFirstAndLastName  TEXT,\n username TEXT NOT NULL,\n password TEXT,\n isSavePassword INTEGER DEFAULT 0,\n isUseSteamRelay INTEGER DEFAULT 0,\n authType INTEGER DEFAULT 1,\n icon BLOB,\n timePlayed INTEGER DEFAULT 0,\n lastLogon TEXT,\n FOREIGN KEY (serverId) REFERENCES server (id)\n);";

/// What can stop an operation. The original prints SQL errors and returns
/// its failure value; only an unparsable legacy port escapes (a
/// `NumberFormatException` the original does not catch).
#[derive(Debug)]
pub enum Error {
    Sql(pz_sqlite::Error),
    /// `Integer.parseInt` failed on the legacy file's `port`.
    LegacyPort(String),
}

impl From<pz_sqlite::Error> for Error {
    fn from(e: pz_sqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Sql(e) => write!(f, "{e}"),
            Error::LegacyPort(s) => write!(f, "For input string: \"{s}\""),
        }
    }
}

pub struct AccountDb {
    pub cache_dir: PathBuf,
    /// `SteamUtils.isSteamModeEnabled()`.
    pub steam: bool,
}

/// `Integer.parseInt`.
fn parse_java_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn text(v: Option<&str>) -> Value {
    v.map_or(Value::Null, |s| Value::Text(s.to_string()))
}

fn date(v: Option<LocalDateTime>) -> Value {
    text(v.map(|t| t.format_db()).as_deref())
}

/// `setBoolean`: stored as 1 or 0.
fn boolean(b: bool) -> Value {
    Value::Integer(b as i64)
}

/// `AccountDBHelper.parseDateTime`.
fn parse_date(v: Option<String>) -> Option<LocalDateTime> {
    let s = v?;
    if s.is_empty() {
        return None;
    }
    let t = LocalDateTime::parse_db(&s);
    if t.is_none() {
        eprintln!("Error parsing datetime: {s}");
    }
    t
}

/// Columns of a row by name, read as sqlite-jdbc's getters read them.
struct Cols<'a> {
    table: &'a Table,
    row: &'a pz_sqlite::Row,
}

impl Cols<'_> {
    fn value(&self, name: &str) -> Value {
        self.table
            .column(name)
            .map_or(Value::Null, |i| self.row.get(self.table, i))
    }
    fn string(&self, name: &str) -> Option<String> {
        self.value(name).as_text()
    }
    fn int(&self, name: &str) -> i32 {
        self.value(name).as_i32()
    }
    fn boolean(&self, name: &str) -> bool {
        self.int(name) != 0
    }
    /// `getBinaryStream`: `None` for NULL and empty values.
    fn bytes(&self, name: &str) -> Option<Vec<u8>> {
        let b = match self.value(name) {
            Value::Null => return None,
            Value::Blob(b) => b,
            v => v.as_text().unwrap_or_default().into_bytes(),
        };
        (!b.is_empty()).then_some(b)
    }
}

/// SQLite's ordering of values (BINARY collation): NULL < numbers < text < blobs.
fn sql_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Integer(_) | Value::Real(_) => 1,
        Value::Text(_) => 2,
        Value::Blob(_) => 3,
    };
    let num = |v: &Value| match v {
        Value::Integer(i) => *i as f64,
        Value::Real(r) => *r,
        _ => 0.0,
    };
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Text(x), Value::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ if rank(a) == 1 && rank(b) == 1 => num(a).partial_cmp(&num(b)).unwrap_or(Ordering::Equal),
        _ => rank(a).cmp(&rank(b)),
    }
}

impl AccountDb {
    pub fn new(cache_dir: impl Into<PathBuf>, steam: bool) -> Self {
        AccountDb {
            cache_dir: cache_dir.into(),
            steam,
        }
    }

    pub fn db_path(&self) -> PathBuf {
        let name = if self.steam {
            "ServerListSteam.db"
        } else {
            "ServerList.db"
        };
        self.cache_dir.join("db").join(name)
    }

    /// `getServerListFile()` and the legacy file `create()` imports.
    pub fn legacy_file_name(&self) -> &'static str {
        if self.steam {
            "ServerListSteam.txt"
        } else {
            "ServerList.txt"
        }
    }

    /// `create()`.
    fn open(&self) -> Result<Connection, Error> {
        let db_dir = self.cache_dir.join("db");
        let _ = std::fs::create_dir_all(&db_dir);
        let path = self.db_path();
        // setReadable/setExecutable/setWritable(true, false) on an existing file.
        #[cfg(unix)]
        if let Ok(m) = std::fs::metadata(&path) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &path,
                std::fs::Permissions::from_mode(m.permissions().mode() | 0o777),
            );
        }
        let new = !path.exists();
        let mut conn = Connection::open(&path)?;
        if new {
            conn.execute(CREATE_SERVER)?;
            conn.execute(CREATE_ACCOUNT)?;
        }
        conn.execute("PRAGMA JOURNAL_MODE=TRUNCATE;")?;
        conn.set_autocommit(false)?;
        // `LuaManager.getLuaCacheDir()` makes the directory.
        let lua_dir = self.cache_dir.join("Lua");
        let _ = std::fs::create_dir(&lua_dir);
        let legacy = lua_dir.join(self.legacy_file_name());
        if legacy.exists() {
            Self::import_legacy(&mut conn, &legacy)?;
            let _ = std::fs::remove_file(&legacy);
        }
        Ok(conn)
    }

    /// `parseInputFile`: `key=value` lines, a `name` key starting the next
    /// server. The original's account insert names seven columns for eight
    /// values, so it always fails: the first server is inserted (left
    /// uncommitted) and the import stops there.
    fn import_legacy(conn: &mut Connection, path: &Path) -> Result<(), Error> {
        let Ok(text) = std::fs::read(path) else {
            return Ok(());
        };
        let text = String::from_utf8_lossy(&text);
        let mut data: Vec<(String, String)> = Vec::new();
        let get = |data: &[(String, String)], k: &str| -> String {
            data.iter()
                .rev()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        let mut lines = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .peekable();
        let flush = |conn: &mut Connection, data: &[(String, String)]| -> Result<bool, Error> {
            let port = data
                .iter()
                .rev()
                .find(|(k, _)| k == "port")
                .map_or("0".to_string(), |(_, v)| v.clone());
            let port = parse_java_int(&port).ok_or(Error::LegacyPort(port))?;
            match conn.insert(
                "server",
                &[
                    ("name", Value::Text(get(data, "name"))),
                    ("ip", Value::Text(get(data, "ip"))),
                    ("port", Value::Integer(port as i64)),
                    ("serverPassword", Value::Text(get(data, "serverpassword"))),
                    ("description", Value::Text(get(data, "description"))),
                ],
            ) {
                Ok(id) if id > 0 => {
                    eprintln!("Database error: 8 values for 7 columns");
                    Ok(true)
                }
                Ok(_) => Ok(false),
                Err(e) => {
                    eprintln!("Database error: {e}");
                    Ok(true)
                }
            }
        };
        // `BufferedReader.readLine` drops a final empty line.
        while let Some(line) = lines.next() {
            if line.is_empty() && lines.peek().is_none() {
                break;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim().to_string(), v.trim().to_string());
            if k == "name" && !data.is_empty() {
                if flush(conn, &data)? {
                    return Ok(());
                }
                data.clear();
            }
            match data.iter_mut().find(|(key, _)| *key == k) {
                Some(e) => e.1 = v,
                None => data.push((k, v)),
            }
        }
        if !data.is_empty() {
            flush(conn, &data)?;
        }
        Ok(())
    }

    /// `getServerList()`: servers by their accounts' latest `lastLogon`
    /// (newest first, servers without one last, ties by id).
    pub fn get_server_list(&self) -> Result<Vec<Server>, Error> {
        let mut conn = self.open()?;
        let mut read = || -> pz_sqlite::Result<Vec<Server>> {
            let st = conn.table("server")?;
            let at = conn.table("account")?;
            let servers = conn.rows(&st)?;
            let accounts = conn.rows(&at)?;
            let mut keyed: Vec<(Value, Server)> = Vec::new();
            for row in &servers {
                let c = Cols { table: &st, row };
                let mut s = Server::default();
                s.id = c.int("id");
                s.name = c.string("name");
                s.set_ip(c.string("ip"));
                s.port = c.int("port");
                s.set_server_password(c.string("serverPassword"));
                s.description = c.string("description");
                s.last_online = parse_date(c.string("lastOnline"));
                s.last_data_update = parse_date(c.string("lastDataUpdate"));
                // GROUP BY s.id: the join key is the server's own id value.
                let sid = c.value("id");
                let mut latest = Value::Null;
                for row in &accounts {
                    let a = Cols { table: &at, row };
                    if sql_cmp(&a.value("serverId"), &sid).is_eq()
                        && !matches!(a.value("serverId"), Value::Null)
                    {
                        let l = a.value("lastLogon");
                        if !l.is_null() && (latest.is_null() || sql_cmp(&l, &latest).is_gt()) {
                            latest = l;
                        }
                    }
                }
                // `loadAccountsForServer`: WHERE serverId = getID().
                for row in &accounts {
                    let a = Cols { table: &at, row };
                    if !sql_cmp(&a.value("serverId"), &Value::Integer(s.id as i64)).is_eq() {
                        continue;
                    }
                    let acc = Account {
                        id: a.int("id"),
                        user_name: a.string("username"),
                        pwd: a.string("password"),
                        save_pwd: a.boolean("isSavePassword"),
                        use_steam_relay: a.boolean("isUseSteamRelay"),
                        auth_type: a.int("authType"),
                        player_first_and_last_name: a.string("playerFirstAndLastName"),
                        icon: a.bytes("icon"),
                        time_played: a.int("timePlayed"),
                        last_logon: parse_date(a.string("lastLogon")),
                    };
                    s.accounts.push(Rc::new(RefCell::new(acc)));
                }
                keyed.push((latest, s));
            }
            // ORDER BY MAX(lastLogon) DESC NULLS LAST, stable on the group order.
            keyed.sort_by(|(a, _), (b, _)| match (a.is_null(), b.is_null()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => sql_cmp(b, a),
            });
            Ok(keyed.into_iter().map(|(_, s)| s).collect())
        };
        match read() {
            Ok(v) => Ok(v),
            Err(e) => {
                eprintln!("Error retrieving server list: {e}");
                Ok(Vec::new())
            }
        }
    }

    /// `saveNewServer(server)`: the new id, or -1.
    pub fn save_new_server(&self, s: &Server) -> Result<i32, Error> {
        let mut conn = self.open()?;
        let r = conn.insert(
            "server",
            &[
                ("name", text(s.name.as_deref())),
                ("ip", text(s.host.as_deref())),
                ("port", Value::Integer(s.port as i64)),
                ("serverPassword", Value::Text(s.server_pwd.clone())),
                ("description", text(s.description.as_deref())),
                ("lastOnline", date(s.last_online)),
                ("lastDataUpdate", date(s.last_data_update)),
            ],
        );
        Ok(match r.and_then(|id| conn.commit().map(|_| id)) {
            Ok(id) => id as i32,
            Err(e) => {
                eprintln!("Error saving new server: {e}");
                -1
            }
        })
    }

    /// `updateServer(server)`.
    pub fn update_server(&self, s: &Server) -> Result<bool, Error> {
        if s.id <= 0 {
            return Ok(false);
        }
        let mut conn = self.open()?;
        let r = conn.update(
            "server",
            s.id as i64,
            &[
                ("name", text(s.name.as_deref())),
                ("ip", text(s.host.as_deref())),
                ("port", Value::Integer(s.port as i64)),
                ("serverPassword", Value::Text(s.server_pwd.clone())),
                ("description", text(s.description.as_deref())),
                ("lastOnline", date(s.last_online)),
                ("lastDataUpdate", date(s.last_data_update)),
            ],
        );
        Ok(match r.and_then(|n| conn.commit().map(|_| n)) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("Error updating server: {e}");
                false
            }
        })
    }

    /// `deleteServer(server)`: its accounts, then the server; rolled back
    /// when no server row matched.
    pub fn delete_server(&self, s: &Server) -> Result<bool, Error> {
        if s.id <= 0 {
            return Ok(false);
        }
        let mut conn = self.open()?;
        let id = Value::Integer(s.id as i64);
        let r = conn
            .delete_where("account", |t, row| {
                t.column("serverId")
                    .is_ok_and(|i| sql_cmp(&row.get(t, i), &id).is_eq())
            })
            .and_then(|_| conn.delete("server", s.id as i64));
        Ok(match r {
            Ok(true) => match conn.commit() {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Error deleting server and accounts: {e}");
                    false
                }
            },
            Ok(false) => {
                conn.rollback();
                false
            }
            Err(e) => {
                conn.rollback();
                eprintln!("Error deleting server and accounts: {e}");
                false
            }
        })
    }

    /// `saveNewAccount(server, account)`: the new id, or -1.
    pub fn save_new_account(&self, s: &Server, a: &Account) -> Result<i32, Error> {
        let mut conn = self.open()?;
        let r = conn.insert(
            "account",
            &[
                ("serverId", Value::Integer(s.id as i64)),
                ("username", text(a.user_name.as_deref())),
                ("password", text(a.pwd.as_deref())),
                ("isSavePassword", boolean(a.save_pwd)),
                ("isUseSteamRelay", boolean(a.use_steam_relay)),
                ("authType", Value::Integer(a.auth_type as i64)),
                ("timePlayed", Value::Integer(a.time_played as i64)),
                ("lastLogon", text(a.last_logon_string().as_deref())),
            ],
        );
        Ok(match r.and_then(|id| conn.commit().map(|_| id)) {
            Ok(id) => id as i32,
            Err(e) => {
                eprintln!("Error saving new account: {e}");
                -1
            }
        })
    }

    /// `updateAccount(account)`.
    pub fn update_account(&self, a: &Account) -> Result<bool, Error> {
        if a.id <= 0 {
            return Ok(false);
        }
        let mut conn = self.open()?;
        let r = conn.update(
            "account",
            a.id as i64,
            &[
                ("username", text(a.user_name.as_deref())),
                ("password", text(a.pwd.as_deref())),
                ("isSavePassword", boolean(a.save_pwd)),
                ("isUseSteamRelay", boolean(a.use_steam_relay)),
                ("authType", Value::Integer(a.auth_type as i64)),
                ("timePlayed", Value::Integer(a.time_played as i64)),
                ("lastLogon", text(a.last_logon_string().as_deref())),
                (
                    "playerFirstAndLastName",
                    text(a.player_first_and_last_name.as_deref()),
                ),
            ],
        );
        Ok(match r.and_then(|n| conn.commit().map(|_| n)) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("Error updating account: {e}");
                false
            }
        })
    }

    /// `deleteAccount(account)`.
    pub fn delete_account(&self, a: &Account) -> Result<bool, Error> {
        if a.id <= 0 {
            return Ok(false);
        }
        let mut conn = self.open()?;
        let r = conn.delete("account", a.id as i64);
        Ok(match r.and_then(|n| conn.commit().map(|_| n)) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("Error deleting account: {e}");
                false
            }
        })
    }
}
