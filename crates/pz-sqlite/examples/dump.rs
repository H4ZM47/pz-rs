//! Prints every row of every table of a database, one line per row:
//! `rowid|col|col...` with NULL as `NULL`, integers in decimal, reals as
//! `R:` plus their SQLite text form, text and blobs as SQL literals
//! (`'it''s'`, `X'0A1B'`). The same lines can be produced through SQLite
//! itself (`typeof()`, `quote()`, the driver's `getString` for reals) to
//! compare this reader against it. Reals are rendered like SQLite 3.48 (the
//! sqlite-jdbc the game bundles), with `%!.15g`; newer `sqlite3` CLIs print
//! up to 17 digits instead.
//!
//! Usage: `cargo run -p pz-sqlite --example dump -- <file.db>`

use pz_sqlite::{Database, Value};

fn show(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Integer(i) => i.to_string(),
        Value::Real(_) => format!("R:{}", v.as_text().unwrap()),
        Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Blob(b) => format!(
            "X'{}'",
            b.iter().map(|x| format!("{x:02X}")).collect::<String>()
        ),
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file.db>");
    let db = Database::open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    for e in db.schema().unwrap() {
        if e.kind != "table" {
            continue;
        }
        let t = db.table(&e.name).unwrap();
        let cols: Vec<&str> = t.def.columns.iter().map(|c| c.name.as_str()).collect();
        println!("== {} ({})", t.name, cols.join(","));
        db.for_each_row(&t, |r| {
            let vals: Vec<String> = (0..cols.len()).map(|i| show(&r.get(&t, i))).collect();
            println!("{}|{}", r.rowid, vals.join("|"));
            true
        })
        .unwrap();
    }
}
