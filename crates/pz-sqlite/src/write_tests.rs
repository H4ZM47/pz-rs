//! Tests of the write path. Files are checked with the `sqlite3` CLI when it
//! is installed (`PRAGMA integrity_check` and query results).

use crate::{Connection, Database, Error, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const LOCAL_PLAYERS: &str = "CREATE TABLE localPlayers (id   INTEGER PRIMARY KEY NOT NULL,name STRING,wx    INTEGER,wy    INTEGER,x    FLOAT,y    FLOAT,z    FLOAT,worldversion    INTEGER,data BLOB,isDead BOOLEAN);";
const NETWORK_PLAYERS: &str = "CREATE TABLE networkPlayers (id   INTEGER PRIMARY KEY NOT NULL,world TEXT,username TEXT,playerIndex   INTEGER,name STRING,steamid STRING,x    FLOAT,y    FLOAT,z    FLOAT,worldversion    INTEGER,data BLOB,isDead BOOLEAN,splitUsername STRING);";
const VEHICLES: &str = "CREATE TABLE vehicles (id   INTEGER PRIMARY KEY NOT NULL,wx    INTEGER,wy    INTEGER,x    FLOAT,y    FLOAT,inMeta BOOLEAN NULL DEFAULT FALSE,worldversion    INTEGER,data BLOB);";

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(tag: &str) -> (Tmp, PathBuf) {
    let dir = std::env::temp_dir().join(format!("pz-sqlite-w-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("test.db");
    (Tmp(dir), p)
}

/// `sqlite3` output for `sql`, after asserting `integrity_check` is ok;
/// `None` without the CLI.
fn sqlite3(path: &Path, sql: &str) -> Option<String> {
    let run = |q: &str| {
        let out = std::process::Command::new("sqlite3")
            .arg("-readonly")
            .arg(path)
            .arg(q)
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    };
    let check = run("PRAGMA integrity_check")?;
    assert_eq!(check, "ok", "sqlite3 rejects {}", path.display());
    run(sql)
}

fn rng(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn blob(seed: &mut u64, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng(seed) as u8).collect()
}

fn create_players(c: &mut Connection) {
    c.execute(LOCAL_PLAYERS).unwrap();
    c.execute(NETWORK_PLAYERS).unwrap();
    c.execute("CREATE INDEX inpusername ON networkPlayers (username);")
        .unwrap();
    c.execute("PRAGMA JOURNAL_MODE=TRUNCATE;").unwrap();
}

fn player_values(
    name: &str,
    x: f32,
    y: f32,
    data: Vec<u8>,
    dead: bool,
) -> Vec<(&'static str, Value)> {
    vec![
        ("wx", Value::Integer((x as i64) / 8)),
        ("wy", Value::Integer((y as i64) / 8)),
        ("x", Value::Real(f64::from(x))),
        ("y", Value::Real(f64::from(y))),
        ("z", Value::Real(0.0)),
        ("worldversion", Value::Integer(244)),
        ("data", Value::Blob(data)),
        ("isDead", Value::Integer(i64::from(dead))),
        ("name", Value::Text(name.into())),
    ]
}

#[test]
fn players_db_lifecycle() {
    let (_t, path) = tmp("players");
    let mut c = Connection::open(&path).unwrap();
    create_players(&mut c);
    c.set_autocommit(false).unwrap();
    let mut seed = 7;
    let mut v = player_values(
        "Kate Smith",
        10850.5,
        9870.25,
        blob(&mut seed, 40_000),
        false,
    );
    v.push(("id", Value::Integer(1)));
    assert_eq!(c.insert("localPlayers", &v).unwrap(), 1);
    c.commit().unwrap();
    let mut v = player_values("Bob", 100.0, 200.0, vec![], true);
    v.push(("id", Value::Integer(2)));
    c.insert("localPlayers", &v).unwrap();
    c.commit().unwrap();
    // Duplicate id.
    assert!(matches!(
        c.insert("localPlayers", &v),
        Err(Error::Constraint(_))
    ));
    let v = player_values("Kate Smith", 11000.0, 9000.0, blob(&mut seed, 9000), true);
    assert!(c.update("localPlayers", 1, &v).unwrap());
    c.commit().unwrap();
    assert!(!c.update("localPlayers", 9, &v).unwrap());
    drop(c);

    let out = sqlite3(
        &path,
        "SELECT id, name, wx, wy, x, y, z, typeof(z), worldversion, length(data), typeof(data), isDead FROM localPlayers",
    );
    if let Some(out) = out {
        assert_eq!(
            out,
            "1|Kate Smith|1375|1125|11000.0|9000.0|0.0|real|244|9000|blob|1\n\
             2|Bob|12|25|100.0|200.0|0.0|real|244|0|blob|1"
        );
        let schema = sqlite3(&path, "SELECT type, name, tbl_name, sql FROM sqlite_schema").unwrap();
        assert!(schema.contains("table|localPlayers|localPlayers|CREATE TABLE localPlayers (id   INTEGER PRIMARY KEY NOT NULL,name STRING,wx    INTEGER,wy    INTEGER,x    FLOAT,y    FLOAT,z    FLOAT,worldversion    INTEGER,data BLOB,isDead BOOLEAN)"));
        assert!(schema.contains("index|inpusername|networkPlayers|CREATE INDEX inpusername ON networkPlayers (username)"));
    }
    // The reader sees the same rows.
    let db = Database::open(&path).unwrap();
    let t = db.table("localPlayers").unwrap();
    let rows = db.rows(&t).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get(&t, 4), Value::Real(11000.0));
}

#[test]
fn set_player1_moves_rows() {
    let (_t, path) = tmp("setp1");
    let mut c = Connection::open(&path).unwrap();
    create_players(&mut c);
    for id in [1, 3, 5] {
        let mut v = player_values(&format!("p{id}"), 1.0, 2.0, vec![id as u8; 100], false);
        v.push(("id", Value::Integer(id)));
        c.insert("localPlayers", &v).unwrap();
    }
    // UPDATE localPlayers SET id=? WHERE id=? three times.
    assert!(c
        .update("localPlayers", 1, &[("id", Value::Integer(6))])
        .unwrap());
    assert!(c
        .update("localPlayers", 3, &[("id", Value::Integer(1))])
        .unwrap());
    assert!(c
        .update("localPlayers", 6, &[("id", Value::Integer(3))])
        .unwrap());
    assert!(matches!(
        c.update("localPlayers", 3, &[("id", Value::Integer(5))]),
        Err(Error::Constraint(_))
    ));
    drop(c);
    if let Some(out) = sqlite3(&path, "SELECT id, name FROM localPlayers") {
        assert_eq!(out, "1|p3\n3|p1\n5|p5");
    }
}

#[test]
fn vehicles_alter_drop_defaults() {
    let (_t, path) = tmp("vehicles");
    let mut c = Connection::open(&path).unwrap();
    c.execute("CREATE TABLE vehicles (id   INTEGER PRIMARY KEY NOT NULL,wx    INTEGER,wy    INTEGER,x    FLOAT,y    FLOAT,worldversion    INTEGER,data BLOB);").unwrap();
    c.execute("CREATE INDEX ivwx ON vehicles (wx);").unwrap();
    c.execute("CREATE INDEX ivwy ON vehicles (wy);").unwrap();
    c.execute(LOCAL_PLAYERS).unwrap();
    c.execute(NETWORK_PLAYERS).unwrap();
    for id in 1..=50 {
        c.insert(
            "vehicles",
            &[
                ("wx", Value::Integer(id % 7)),
                ("wy", Value::Integer(-id)),
                ("x", Value::Real(id as f64 + 0.5)),
                ("data", Value::Blob(vec![1; 300])),
                ("id", Value::Integer(id)),
            ],
        )
        .unwrap();
    }
    c.execute("ALTER TABLE 'vehicles' ADD 'inMeta' BOOLEAN NULL DEFAULT FALSE")
        .unwrap();
    assert!(matches!(
        c.execute("ALTER TABLE 'vehicles' ADD 'inMeta' BOOLEAN"),
        Err(Error::Sql(_))
    ));
    let t = c.table("vehicles").unwrap();
    let r = c.row(&t, 3).unwrap().unwrap();
    assert_eq!(r.get(&t, t.column("inMeta").unwrap()), Value::Integer(0));
    c.update(
        "vehicles",
        3,
        &[("wx", Value::Integer(100)), ("inMeta", Value::Integer(1))],
    )
    .unwrap();
    assert!(c.delete("vehicles", 4).unwrap());
    assert!(!c.delete("vehicles", 4).unwrap());
    c.execute("DROP TABLE localPlayers").unwrap();
    c.execute("DROP TABLE networkPlayers").unwrap();
    assert!(matches!(
        c.execute("DROP TABLE networkPlayers"),
        Err(Error::NoSuchTable(_))
    ));
    drop(c);
    if let Some(out) = sqlite3(
        &path,
        "SELECT count(*), sum(inMeta), (SELECT id FROM vehicles WHERE wx=100) FROM vehicles; \
         SELECT sql FROM sqlite_schema WHERE name='vehicles'; \
         PRAGMA freelist_count",
    ) {
        let mut l = out.lines();
        assert_eq!(l.next(), Some("49|1|3"));
        assert_eq!(
            l.next(),
            Some(
                &*VEHICLES
                    .replace(",inMeta BOOLEAN NULL DEFAULT FALSE", "")
                    .replace(
                        "data BLOB);",
                        "data BLOB, 'inMeta' BOOLEAN NULL DEFAULT FALSE)"
                    )
            )
        );
        assert_ne!(l.next(), Some("0"));
    }
}

#[test]
fn hot_journal_is_rolled_back() {
    let (_t, path) = tmp("journal");
    let mut c = Connection::open(&path).unwrap();
    c.execute(VEHICLES).unwrap();
    c.insert("vehicles", &[("id", Value::Integer(1))]).unwrap();
    drop(c);
    let before = crate::pager::read_file(&path);
    // Simulate a crash after the journal was synced and some pages were
    // written: the journal holds the original pages 1 and 2.
    let ps = 4096;
    let pages = vec![
        (1u32, before[..ps].to_vec()),
        (2u32, before[ps..2 * ps].to_vec()),
    ];
    crate::pager::write_hot_journal_for_test(&path, &pages, 2);
    let mut damaged = before.clone();
    damaged[ps + 100] ^= 0xff;
    damaged.extend(vec![0xAB; ps]);
    std::fs::write(&path, &damaged).unwrap();
    let mut c = Connection::open(&path).unwrap();
    assert_eq!(crate::pager::read_file(&path), before);
    let t = c.table("vehicles").unwrap();
    assert_eq!(c.rows(&t).unwrap().len(), 1);
    drop(c);
    let mut j = path.as_os_str().to_owned();
    j.push("-journal");
    assert!(std::fs::metadata(&j).is_err());
    if let Some(out) = sqlite3(&path, "SELECT count(*) FROM vehicles") {
        assert_eq!(out, "1");
    }
}

/// A commit interrupted after some database pages were written leaves a
/// hot journal that both this crate and `sqlite3` roll back.
#[test]
fn interrupted_commit() {
    let (_t, path) = tmp("crash");
    let mut c = Connection::open(&path).unwrap();
    c.execute(VEHICLES).unwrap();
    c.execute("CREATE INDEX ivwx ON vehicles (wx);").unwrap();
    c.execute("PRAGMA journal_mode=TRUNCATE").unwrap();
    let mut seed = 3;
    for id in 1..=40 {
        c.insert(
            "vehicles",
            &[
                ("id", Value::Integer(id)),
                ("wx", Value::Integer(id % 5)),
                ("data", Value::Blob(blob(&mut seed, 3000))),
            ],
        )
        .unwrap();
    }
    drop(c);
    let before = crate::pager::read_file(&path);
    let mut c = Connection::open(&path).unwrap();
    c.set_autocommit(false).unwrap();
    for id in 1..=40 {
        if id % 3 == 0 {
            c.delete("vehicles", id).unwrap();
        } else {
            c.update(
                "vehicles",
                id,
                &[("data", Value::Blob(blob(&mut seed, 9000)))],
            )
            .unwrap();
        }
    }
    crate::pager::CRASH_AFTER.with(|x| x.set(Some(5)));
    assert!(c.commit().is_err());
    crate::pager::CRASH_AFTER.with(|x| x.set(None));
    drop(c);
    assert_ne!(crate::pager::read_file(&path), before);
    let mut j = path.as_os_str().to_owned();
    j.push("-journal");
    assert!(std::fs::metadata(&j).unwrap().len() > 0);

    // sqlite3 (allowed to write, so it can roll back) sees the old rows.
    let copy = path.with_file_name("copy.db");
    std::fs::copy(&path, &copy).unwrap();
    let mut jc = copy.as_os_str().to_owned();
    jc.push("-journal");
    std::fs::copy(&j, &jc).unwrap();
    let out = std::process::Command::new("sqlite3")
        .arg(&copy)
        .arg("PRAGMA integrity_check; SELECT count(*), sum(length(data)) FROM vehicles")
        .output();
    if let Ok(out) = out {
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok\n40|120000");
        assert_eq!(crate::pager::read_file(&copy), before);
    }

    // So does this crate.
    let mut c = Connection::open(&path).unwrap();
    assert_eq!(crate::pager::read_file(&path), before);
    let t = c.table("vehicles").unwrap();
    assert_eq!(c.rows(&t).unwrap().len(), 40);
}

#[test]
fn affinity_and_errors() {
    let (_t, path) = tmp("affinity");
    let mut c = Connection::open(&path).unwrap();
    c.execute("CREATE TABLE t (a INTEGER, b TEXT, c FLOAT, d STRING, e, f NOT NULL DEFAULT -1.5)")
        .unwrap();
    let id = c
        .insert(
            "t",
            &[
                ("a", Value::Text(" 12 ".into())),
                ("b", Value::Real(1.5)),
                ("c", Value::Text("3".into())),
                ("d", Value::Text("007".into())),
                ("e", Value::Text("8".into())),
            ],
        )
        .unwrap();
    assert_eq!(id, 1);
    c.insert(
        "t",
        &[
            ("a", Value::Real(2.0)),
            ("c", Value::Real(f64::NAN)),
            ("d", Value::Text("Kate".into())),
        ],
    )
    .unwrap();
    assert!(matches!(
        c.insert("t", &[("f", Value::Null)]),
        Err(Error::Constraint(_))
    ));
    assert!(matches!(
        c.insert("t", &[("zz", Value::Null)]),
        Err(Error::Sql(_))
    ));
    assert!(matches!(
        c.execute(LOCAL_PLAYERS.replace("localPlayers", "t").as_str()),
        Err(Error::Sql(_))
    ));
    drop(c);
    if let Some(out) = sqlite3(
        &path,
        "SELECT rowid, a, typeof(a), b, typeof(b), c, typeof(c), d, typeof(d), e, typeof(e), f FROM t",
    ) {
        assert_eq!(
            out,
            "1|12|integer|1.5|text|3.0|real|7|integer|8|text|-1.5\n\
             2|2|integer||null||null|Kate|text||null|-1.5"
        );
    }
}

/// Random inserts, updates (including rowid moves) and deletes against a
/// model, with rows from empty to many overflow pages, on small pages so
/// the trees grow several levels; checked by the reader and `sqlite3`.
fn stress(page_size: u32, ops: usize, seed0: u64) {
    let (_t, path) = tmp(&format!("stress-{page_size}-{seed0}"));
    if page_size != 4096 {
        // An empty file with the page size set, made by sqlite3 when present.
        let ok = std::process::Command::new("sqlite3")
            .arg(&path)
            .arg(format!("PRAGMA page_size={page_size}; VACUUM;"))
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return;
        }
    }
    let mut c = Connection::open(&path).unwrap();
    c.execute(VEHICLES).unwrap();
    c.execute("CREATE INDEX ivwx ON vehicles (wx);").unwrap();
    c.execute("CREATE INDEX ivwy ON vehicles (wy);").unwrap();
    c.execute("CREATE TABLE names (id INTEGER PRIMARY KEY, n TEXT, k BLOB)")
        .unwrap();
    c.execute("CREATE INDEX inames ON names (n, k);").unwrap();
    c.set_autocommit(false).unwrap();
    let mut model: BTreeMap<i64, (i64, i64, Vec<u8>)> = BTreeMap::new();
    let mut names: BTreeMap<i64, (String, Vec<u8>)> = BTreeMap::new();
    let mut seed = seed0;
    for op in 0..ops {
        let r = rng(&mut seed);
        let id = (r % 400) as i64 + 1;
        let size = match rng(&mut seed) % 10 {
            0..=4 => (rng(&mut seed) % 64) as usize,
            5..=7 => (rng(&mut seed) % 1500) as usize,
            8 => (rng(&mut seed) % 6000) as usize,
            _ => (rng(&mut seed) % 40_000) as usize,
        };
        let wx = (rng(&mut seed) % 50) as i64 - 25;
        let wy = (rng(&mut seed) % 5000) as i64;
        match (r >> 20) % 6 {
            0 | 1 => {
                let data = blob(&mut seed, size);
                let res = c.insert(
                    "vehicles",
                    &[
                        ("wx", Value::Integer(wx)),
                        ("wy", Value::Integer(wy)),
                        ("data", Value::Blob(data.clone())),
                        ("id", Value::Integer(id)),
                    ],
                );
                if let std::collections::btree_map::Entry::Vacant(e) = model.entry(id) {
                    res.unwrap();
                    e.insert((wx, wy, data));
                } else {
                    assert!(matches!(res, Err(Error::Constraint(_))));
                }
            }
            2 => {
                let data = blob(&mut seed, size);
                let hit = c
                    .update(
                        "vehicles",
                        id,
                        &[
                            ("wx", Value::Integer(wx)),
                            ("data", Value::Blob(data.clone())),
                        ],
                    )
                    .unwrap();
                assert_eq!(hit, model.contains_key(&id));
                if let Some(m) = model.get_mut(&id) {
                    m.0 = wx;
                    m.2 = data;
                }
            }
            3 => {
                let to = (rng(&mut seed) % 400) as i64 + 1;
                let res = c.update("vehicles", id, &[("id", Value::Integer(to))]);
                match (model.contains_key(&id), model.contains_key(&to) && to != id) {
                    (false, _) => assert!(!res.unwrap()),
                    (true, true) => assert!(matches!(res, Err(Error::Constraint(_)))),
                    (true, false) => {
                        assert!(res.unwrap());
                        let v = model.remove(&id).unwrap();
                        model.insert(to, v);
                    }
                }
            }
            4 => {
                assert_eq!(
                    c.delete("vehicles", id).unwrap(),
                    model.remove(&id).is_some()
                );
            }
            _ => {
                // Index keys with long text (overflowing index cells).
                let n: String = (0..size.min(3000))
                    .map(|_| (b'a' + (rng(&mut seed) % 3) as u8) as char)
                    .collect();
                let k = blob(&mut seed, size % 300);
                if names.contains_key(&id) && rng(&mut seed).is_multiple_of(2) {
                    assert!(c.delete("names", id).unwrap());
                    names.remove(&id);
                } else if let Some(e) = names.get_mut(&id) {
                    c.update("names", id, &[("n", Value::Text(n.clone()))])
                        .unwrap();
                    e.0 = n;
                } else {
                    c.insert(
                        "names",
                        &[
                            ("id", Value::Integer(id)),
                            ("n", Value::Text(n.clone())),
                            ("k", Value::Blob(k.clone())),
                        ],
                    )
                    .unwrap();
                    names.insert(id, (n, k));
                }
            }
        }
        if op % 7 == 0 {
            c.commit().unwrap();
        }
        if op % 997 == 0 && rng(&mut seed).is_multiple_of(3) {
            // Roll back a batch of changes.
            c.commit().unwrap();
            let saved = (model.clone(), names.clone());
            c.delete("vehicles", *model.keys().next().unwrap_or(&1))
                .ok();
            c.rollback();
            (model, names) = saved;
        }
    }
    c.commit().unwrap();
    let t = c.table("vehicles").unwrap();
    let rows = c.rows(&t).unwrap();
    assert_eq!(rows.len(), model.len());
    for (r, (id, m)) in rows.iter().zip(&model) {
        assert_eq!(r.rowid, *id);
        assert_eq!(r.get(&t, 1), Value::Integer(m.0));
        assert_eq!(r.get(&t, 2), Value::Integer(m.1));
        assert_eq!(r.get(&t, 7), Value::Blob(m.2.clone()));
    }
    drop(c);
    let db = Database::open(&path).unwrap();
    let nt = db.table("names").unwrap();
    let got: Vec<(i64, Value)> = db
        .rows(&nt)
        .unwrap()
        .into_iter()
        .map(|r| (r.rowid, r.get(&nt, 1)))
        .collect();
    let want: Vec<(i64, Value)> = names
        .iter()
        .map(|(id, (n, _))| (*id, Value::Text(n.clone())))
        .collect();
    assert_eq!(got, want);
    let total: usize = model.values().map(|m| m.2.len()).sum();
    if let Some(out) = sqlite3(
        &path,
        "SELECT count(*), coalesce(sum(length(data)),0) FROM vehicles; \
         SELECT count(*) FROM vehicles INDEXED BY ivwx WHERE wx > -1000; \
         SELECT count(*) FROM names INDEXED BY inames WHERE n > ''",
    ) {
        let nonempty = names.values().filter(|(n, _)| !n.is_empty()).count();
        assert_eq!(
            out,
            format!("{}|{total}\n{}\n{nonempty}", model.len(), model.len())
        );
    }
}

#[test]
fn stress_4096() {
    stress(4096, 4000, 0x9e37_79b9_7f4a_7c15);
}

#[test]
fn stress_512() {
    stress(512, 3000, 12345);
}

#[test]
fn stress_1024() {
    stress(1024, 3000, 777);
}

#[test]
fn rows_until_error_keeps_the_rows_before_a_bad_record() {
    let (_t, path) = tmp("rows-until-error");
    {
        let mut c = Connection::open(&path).unwrap();
        c.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, data BLOB);")
            .unwrap();
        for i in 1..=3u8 {
            let mark = [
                b'M',
                b'A',
                b'R',
                b'K',
                b'0' + i,
                b'M',
                b'A',
                b'R',
                b'K',
                b'0' + i,
            ];
            c.insert("t", &[("data", Value::Blob(mark.to_vec()))])
                .unwrap();
        }
    }
    // Break the third record's header size (the byte three before its blob).
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes.windows(10).position(|w| w == b"MARK3MARK3").unwrap();
    bytes[at - 3] = 0x7f;
    std::fs::write(&path, &bytes).unwrap();
    let mut c = Connection::open(&path).unwrap();
    let t = c.table("t").unwrap();
    let (rows, err) = c.rows_until_error(&t);
    assert_eq!(rows.len(), 2);
    assert!(err.is_some());
}
