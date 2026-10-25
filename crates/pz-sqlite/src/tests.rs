use crate::writer::Writer;
use crate::{Database, Error, TextEncoding, Value};
use std::path::PathBuf;

const LOCAL_PLAYERS: &str =
    "CREATE TABLE localPlayers (id INTEGER PRIMARY KEY NOT NULL,name STRING,\
wx INTEGER,wy INTEGER,x FLOAT,y FLOAT,z FLOAT,worldversion INTEGER,data BLOB,isDead BOOLEAN)";

fn temp_file(tag: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pz-sqlite-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("test.db");
    std::fs::write(&p, bytes).unwrap();
    p
}

/// Runs the `sqlite3` CLI on `path` when it is installed: checks the file
/// is well formed and returns the query output.
fn sqlite3(path: &PathBuf, sql: &str) -> Option<String> {
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
    assert_eq!(check, "ok", "sqlite3 rejects the test database");
    run(sql)
}

fn player(id: i64, name: &str, dead: bool, data: Vec<u8>) -> (i64, Vec<Value>) {
    let v = vec![
        Value::Null,
        Value::Text(name.into()),
        Value::Integer(10),
        Value::Integer(-3),
        Value::Real(10_850.5),
        Value::Real(9_870.25),
        // SQLite stores integral values of REAL columns as integers.
        Value::Integer(0),
        Value::Integer(244),
        Value::Blob(data),
        Value::Integer(i64::from(dead)),
    ];
    (id, v)
}

#[test]
fn players_table() {
    let mut w = Writer::new(4096, 0, TextEncoding::Utf8);
    let rows = vec![
        player(1, "Franklin Diggs", false, vec![1, 2, 3]),
        player(3, "Kate", true, vec![]),
    ];
    w.table("localPlayers", LOCAL_PLAYERS, &rows);
    let bytes = w.finish();
    let path = temp_file("players", &bytes);
    let db = Database::open(&path).unwrap();
    let h = db.header.clone().unwrap();
    assert_eq!(
        (h.page_size, h.usable, h.text_encoding),
        (4096, 4096, TextEncoding::Utf8)
    );
    let t = db.table("LOCALPLAYERS").unwrap();
    assert_eq!(t.column("isdead").unwrap(), 9);
    assert!(t.is_rowid_alias(0));
    let all = db.rows(&t).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].get(&t, 0), Value::Integer(1));
    assert_eq!(all[1].get(&t, 0), Value::Integer(3));
    assert_eq!(all[0].get(&t, 1), Value::Text("Franklin Diggs".into()));
    assert_eq!(all[0].get(&t, 4), Value::Real(10_850.5));
    assert_eq!(all[0].get(&t, 6), Value::Real(0.0));
    assert_eq!(all[0].values[6], Value::Integer(0));
    assert_eq!(all[1].get(&t, 9).as_i32(), 1);
    assert_eq!(
        db.row_by_rowid(&t, 3)
            .unwrap()
            .unwrap()
            .get(&t, 1)
            .as_text()
            .unwrap(),
        "Kate"
    );
    assert!(db.row_by_rowid(&t, 2).unwrap().is_none());
    assert!(matches!(db.table("nope"), Err(Error::NoSuchTable(_))));
    assert!(matches!(t.column("nope"), Err(Error::NoSuchColumn(_))));
    if let Some(out) = sqlite3(&path, "SELECT id, name, isDead FROM localPlayers") {
        assert_eq!(out, "1|Franklin Diggs|0\n3|Kate|1");
    }
}

/// Deterministic pseudo-random bytes.
fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

#[test]
fn deep_tree_small_pages() {
    // 512-byte pages with reserved bytes force several interior levels.
    let mut w = Writer::new(512, 32, TextEncoding::Utf8);
    let rows: Vec<(i64, Vec<Value>)> = (0..3000i64)
        .map(|i| {
            let id = i * 7 - 9000;
            (
                id,
                vec![
                    Value::Integer(id),
                    Value::Text(format!("row {id}")),
                    Value::Blob(bytes(i as u64, (i % 40) as usize)),
                ],
            )
        })
        .collect();
    w.table("t", "CREATE TABLE t(a, b TEXT, c BLOB)", &rows);
    let bytes_ = w.finish();
    let path = temp_file("deep", &bytes_);
    let db = Database::open(&path).unwrap();
    let t = db.table("t").unwrap();
    assert_eq!(t.def.rowid_alias, None);
    let got = db.rows(&t).unwrap();
    assert_eq!(got.len(), rows.len());
    for (g, (id, v)) in got.iter().zip(&rows) {
        assert_eq!(g.rowid, *id);
        assert_eq!(&g.values, v);
    }
    for (id, v) in rows.iter().step_by(97) {
        assert_eq!(&db.row_by_rowid(&t, *id).unwrap().unwrap().values, v);
        assert!(db.row_by_rowid(&t, id + 1).unwrap().is_none());
    }
    assert!(db.row_by_rowid(&t, i64::MAX).unwrap().is_none());
    assert!(db.row_by_rowid(&t, i64::MIN).unwrap().is_none());
    // Early stop.
    let mut n = 0;
    db.for_each_row(&t, |_| {
        n += 1;
        n < 10
    })
    .unwrap();
    assert_eq!(n, 10);
    if let Some(out) = sqlite3(&path, "SELECT count(*), sum(rowid), max(length(c)) FROM t") {
        let sum: i64 = rows.iter().map(|r| r.0).sum();
        assert_eq!(out, format!("3000|{sum}|39"));
    }
}

#[test]
fn overflow_boundaries() {
    for (page_size, reserved) in [
        (1024u32, 0u8),
        (1024, 40),
        (4096, 8),
        (65536, 0),
        (65536, 255),
    ] {
        let u = (page_size - u32::from(reserved)) as usize;
        let x = u - 35;
        let m = ((u - 12) * 32 / 255) - 23;
        // Payload sizes around the local/overflow thresholds; the record
        // header for one blob column adds 1..=4 bytes.
        let mut sizes = vec![
            0,
            1,
            x - 4,
            x - 3,
            x - 2,
            x,
            x + 1,
            m,
            2 * u,
            3 * u + 17,
            200_000,
        ];
        sizes.extend((0..8).map(|i| x + (u - 4) * i - 5 + i));
        let mut w = Writer::new(page_size, reserved, TextEncoding::Utf8);
        let rows: Vec<(i64, Vec<Value>)> = sizes
            .iter()
            .enumerate()
            .map(|(i, &n)| {
                (
                    i as i64 + 1,
                    vec![Value::Blob(bytes(n as u64, n)), Value::Integer(n as i64)],
                )
            })
            .collect();
        w.table("big", "CREATE TABLE big(data BLOB, n INTEGER)", &rows);
        w.table(
            "txt",
            "CREATE TABLE txt(id INTEGER PRIMARY KEY, s TEXT)",
            &[(5, vec![Value::Null, Value::Text("é".repeat(70_000))])],
        );
        let bytes_ = w.finish();
        let db = Database::from_bytes(bytes_.clone()).unwrap();
        let h = db.header.clone().unwrap();
        assert_eq!((h.page_size, h.usable), (page_size, u as u32));
        let t = db.table("big").unwrap();
        let got = db.rows(&t).unwrap();
        for (g, (_, v)) in got.iter().zip(&rows) {
            assert_eq!(&g.values, v, "page {page_size} reserved {reserved}");
        }
        let t = db.table("txt").unwrap();
        let r = db.row_by_rowid(&t, 5).unwrap().unwrap();
        assert_eq!(r.get(&t, 0), Value::Integer(5));
        assert_eq!(r.get(&t, 1).as_text().unwrap().chars().count(), 70_000);
        let path = temp_file(&format!("ovf-{page_size}-{reserved}"), &bytes_);
        if let Some(out) = sqlite3(&path, "SELECT sum(length(data)) = sum(n) FROM big") {
            assert_eq!(out, "1");
        }
    }
}

#[test]
fn utf16_and_many_tables() {
    for enc in [TextEncoding::Utf16le, TextEncoding::Utf16be] {
        let mut w = Writer::new(1024, 0, enc);
        // Enough tables that the schema b-tree needs interior pages.
        for i in 0..60 {
            let name = format!("table_{i}_ünï");
            let sql = format!("CREATE TABLE \"{name}\" (k INTEGER PRIMARY KEY, v TEXT, w REAL)");
            w.table(
                &name,
                &sql,
                &[(
                    i,
                    vec![
                        Value::Null,
                        Value::Text(format!("value {i} ✓")),
                        Value::Real(i as f64 / 4.0),
                    ],
                )],
            );
        }
        let db = Database::from_bytes(w.finish()).unwrap();
        assert_eq!(db.header.as_ref().unwrap().text_encoding, enc);
        assert_eq!(db.schema().unwrap().len(), 60);
        let t = db.table("TABLE_41_ünï").unwrap();
        let r = db.rows(&t).unwrap();
        assert_eq!(r[0].get(&t, 0), Value::Integer(41));
        assert_eq!(r[0].get(&t, 1), Value::Text("value 41 ✓".into()));
        assert_eq!(r[0].get(&t, 2).as_text().unwrap(), "10.25");
    }
}

#[test]
fn altered_table_short_records() {
    let mut w = Writer::new(4096, 0, TextEncoding::Utf8);
    w.table(
        "networkPlayers",
        "CREATE TABLE networkPlayers (id INTEGER PRIMARY KEY NOT NULL,world TEXT, 'splitUsername' STRING)",
        &[(1, vec![Value::Null, Value::Text("w".into())]), (2, vec![Value::Null, Value::Text("w".into()), Value::Text("x".into())])],
    );
    let db = Database::from_bytes(w.finish()).unwrap();
    let t = db.table("networkPlayers").unwrap();
    let c = t.column("SPLITUSERNAME").unwrap();
    let rows = db.rows(&t).unwrap();
    assert_eq!(rows[0].get(&t, c), Value::Null);
    assert_eq!(rows[1].get(&t, c), Value::Text("x".into()));
}

#[test]
fn empty_and_bad_files() {
    let db = Database::from_bytes(Vec::new()).unwrap();
    assert!(db.schema().unwrap().is_empty());
    assert!(matches!(
        db.table("localPlayers"),
        Err(Error::NoSuchTable(_))
    ));
    let path = temp_file("empty", &[]);
    assert!(matches!(
        Database::open(&path).unwrap().table("x"),
        Err(Error::NoSuchTable(_))
    ));

    assert!(matches!(
        Database::from_bytes(vec![0; 4096]),
        Err(Error::NotADatabase(_))
    ));
    assert!(matches!(
        Database::from_bytes(b"SQLite format 3\0".to_vec()),
        Err(Error::NotADatabase(_))
    ));

    let mut w = Writer::new(1024, 0, TextEncoding::Utf8);
    w.table(
        "t",
        "CREATE TABLE t(a)",
        &[(1, vec![Value::Blob(vec![7; 5000])])],
    );
    let good = w.finish();
    // The in-header page count claims more pages than the file holds.
    let mut short = good.clone();
    short.truncate(good.len() - 1024);
    assert!(matches!(
        Database::from_bytes(short.clone()),
        Err(Error::Corrupt(_))
    ));
    // Without a valid in-header count the file size decides, and the lost
    // page (the table's root) shows up when the rows are read.
    short[92..96].copy_from_slice(&[0, 0, 0, 9]);
    let db = Database::from_bytes(short).unwrap();
    let t = db.table("t").unwrap();
    assert!(matches!(db.rows(&t), Err(Error::Corrupt(_))));
    // An overflow chain that ends early (overflow pages are allocated
    // first, from page 2).
    let mut cut = good.clone();
    cut[1024..1028].copy_from_slice(&0u32.to_be_bytes());
    let db = Database::from_bytes(cut).unwrap();
    let t = db.table("t").unwrap();
    assert!(matches!(db.rows(&t), Err(Error::Corrupt(_))));
}

#[test]
fn page_size_65536_header() {
    let mut w = Writer::new(65536, 0, TextEncoding::Utf8);
    w.table("t", "CREATE TABLE t(a)", &[(1, vec![Value::Integer(1)])]);
    let b = w.finish();
    assert_eq!(&b[16..18], &[0, 1]);
    let db = Database::from_bytes(b).unwrap();
    assert_eq!(db.header.as_ref().unwrap().page_size, 65536);
}
