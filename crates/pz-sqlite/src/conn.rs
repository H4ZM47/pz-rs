//! A read-write connection: the statements the game's `players.db` and
//! `vehicles.db` code runs, carried out directly on the file.
//!
//! Schema statements go through [`Connection::execute`] as SQL text (see
//! [`crate::ddl`] for the forms understood). Row changes are typed calls
//! keyed by rowid, mirroring the game's prepared statements:
//!
//! * `INSERT INTO t(cols) VALUES(?...)` → [`Connection::insert`],
//! * `UPDATE t SET ... WHERE id=?` → [`Connection::update`] (setting the
//!   `INTEGER PRIMARY KEY` column moves the row),
//! * `DELETE FROM t WHERE id=?` → [`Connection::delete`].
//!
//! Values are stored as SQLite stores bound parameters: column affinity is
//! applied, NaN reals become NULL, left-out columns take their `DEFAULT`,
//! `NOT NULL` and rowid uniqueness are enforced, and every index of the
//! table is kept in step. Transactions follow JDBC: in autocommit mode each
//! statement commits; otherwise changes wait for [`Connection::commit`].
//! A failing statement leaves the transaction as it was before it.

use crate::btree::{BTree, Kind};
use crate::ddl::{self, Ddl};
use crate::pager::{JournalMode, Pager};
use crate::record::{apply_affinity, decode_record, encode_record, integer_affinity};
use crate::schema::{parse_create_table, Literal};
use crate::{Error, Result, Row, SchemaEntry, Table, Value};
use std::path::Path;

/// An index kept in step with its table.
#[derive(Clone, Debug)]
struct IndexDef {
    root: u32,
    /// Table column numbers of the key.
    cols: Vec<usize>,
}

struct Target {
    table: Table,
    indexes: Vec<IndexDef>,
}

pub struct Connection {
    pager: Pager,
    autocommit: bool,
}

fn eq_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `OP_MustBeInt` on a new rowid value.
fn must_be_int(v: &Value) -> Result<i64> {
    let mismatch = || Error::Sql("datatype mismatch".into());
    let v = match v {
        Value::Text(s) => crate::record::numeric_from_text(s).ok_or_else(mismatch)?,
        Value::Real(r) => integer_affinity(*r),
        v => v.clone(),
    };
    match v {
        Value::Integer(i) => Ok(i),
        _ => Err(mismatch()),
    }
}

/// A bound value as SQLite keeps it: NaN reals are NULL.
fn bound(v: &Value) -> Value {
    match v {
        Value::Real(r) if r.is_nan() => Value::Null,
        v => v.clone(),
    }
}

impl Connection {
    /// Opens `path` for reading and writing, creating an empty file when it
    /// does not exist (as sqlite-jdbc does), after rolling back a hot
    /// journal left by an interrupted commit.
    pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
        let pager = Pager::open(path.as_ref())?;
        let mut c = Connection {
            pager,
            autocommit: true,
        };
        if c.pager.npages > 0 {
            // Fail early on files this crate cannot write.
            c.schema()?;
        }
        Ok(c)
    }

    pub fn path(&self) -> &Path {
        self.pager.path()
    }

    /// `Connection.setAutoCommit`. Turning it on commits pending changes.
    pub fn set_autocommit(&mut self, on: bool) -> Result<()> {
        if on && !self.autocommit {
            self.pager.commit()?;
        }
        self.autocommit = on;
        Ok(())
    }

    pub fn autocommit(&self) -> bool {
        self.autocommit
    }

    /// Commits pending changes (`Connection.commit`).
    pub fn commit(&mut self) -> Result<()> {
        self.pager.commit()
    }

    /// Discards pending changes (`Connection.rollback`).
    pub fn rollback(&mut self) {
        self.pager.rollback();
    }

    /// Whether uncommitted changes are pending.
    pub fn in_transaction(&self) -> bool {
        self.pager.in_write()
    }

    /// Runs one statement: undone on error, committed in autocommit mode.
    fn stmt<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let sp = self.pager.savepoint();
        match f(self) {
            Ok(v) => {
                if self.autocommit {
                    self.pager.commit()?;
                }
                Ok(v)
            }
            Err(e) => {
                self.pager.restore(sp);
                Err(e)
            }
        }
    }

    fn format4(&mut self) -> Result<bool> {
        Ok(self.pager.npages > 0 && self.pager.header_u32(44)? >= 4)
    }

    // ---- reading ----------------------------------------------------------------

    fn schema_rows(&mut self) -> Result<Vec<(i64, SchemaEntry)>> {
        if self.pager.npages == 0 {
            return Ok(Vec::new());
        }
        let bt = BTree::new(Kind::Table, &self.pager);
        let enc = self.pager.enc;
        let mut raw = Vec::new();
        bt.table_scan(&mut self.pager, 1, &mut |id, p| {
            raw.push((id, p));
            Ok(())
        })?;
        let mut out = Vec::with_capacity(raw.len());
        for (id, p) in raw {
            let v = decode_record(&p, enc)?;
            let text = |i: usize| v.get(i).and_then(Value::as_text);
            match (text(0), text(1), text(2)) {
                (Some(kind), Some(name), Some(tbl_name)) => out.push((
                    id,
                    SchemaEntry {
                        kind,
                        name,
                        tbl_name,
                        rootpage: v.get(3).map_or(0, Value::as_i64),
                        sql: text(4),
                    },
                )),
                _ => return Err(Error::Corrupt("malformed schema row".into())),
            }
        }
        Ok(out)
    }

    /// The rows of `sqlite_schema`.
    pub fn schema(&mut self) -> Result<Vec<SchemaEntry>> {
        Ok(self.schema_rows()?.into_iter().map(|(_, e)| e).collect())
    }

    /// Looks up a rowid table by name (ASCII case-insensitive).
    pub fn table(&mut self, name: &str) -> Result<Table> {
        let rows = self.schema_rows()?;
        Self::table_from(&rows, name)
    }

    fn table_from(rows: &[(i64, SchemaEntry)], name: &str) -> Result<Table> {
        let e = rows
            .iter()
            .map(|(_, e)| e)
            .find(|e| e.kind == "table" && eq_name(&e.name, name))
            .ok_or_else(|| Error::NoSuchTable(name.to_string()))?;
        let root = u32::try_from(e.rootpage)
            .ok()
            .filter(|&r| r != 0)
            .ok_or_else(|| Error::Unsupported(format!("table {} has no b-tree", e.name)))?;
        let sql = e
            .sql
            .as_deref()
            .ok_or_else(|| Error::Corrupt(format!("table {} has no SQL", e.name)))?;
        Ok(Table {
            name: e.name.clone(),
            root,
            def: parse_create_table(sql)?,
        })
    }

    /// A table about to be written, with its indexes.
    fn target(&mut self, name: &str) -> Result<Target> {
        let rows = self.schema_rows()?;
        let table = Self::table_from(&rows, name)?;
        if table.def.autoindex {
            return Err(Error::Unsupported(format!(
                "writing table {} with UNIQUE / PRIMARY KEY indexes",
                table.name
            )));
        }
        let mut indexes = Vec::new();
        for (_, e) in &rows {
            if !eq_name(&e.tbl_name, &table.name) || e.kind == "table" {
                continue;
            }
            if e.kind != "index" {
                return Err(Error::Unsupported(format!(
                    "writing table {} with a {}",
                    table.name, e.kind
                )));
            }
            let sql = e
                .sql
                .as_deref()
                .ok_or_else(|| Error::Unsupported(format!("automatic index {}", e.name)))?;
            let Ddl::CreateIndex {
                columns, unique, ..
            } = ddl::parse(sql)?
            else {
                return Err(Error::Corrupt(format!("index {} SQL", e.name)));
            };
            if unique {
                return Err(Error::Unsupported(format!("UNIQUE index {}", e.name)));
            }
            let cols = columns
                .iter()
                .map(|c| table.column(c))
                .collect::<Result<Vec<_>>>()?;
            let root = u32::try_from(e.rootpage)
                .ok()
                .filter(|&r| r != 0)
                .ok_or_else(|| Error::Corrupt(format!("index {} root", e.name)))?;
            indexes.push(IndexDef { root, cols });
        }
        Ok(Target { table, indexes })
    }

    fn decode_row(&self, rowid: i64, payload: &[u8]) -> Result<Row> {
        Ok(Row {
            rowid,
            values: decode_record(payload, self.pager.enc)?,
        })
    }

    /// Every row of `table` in rowid order.
    pub fn rows(&mut self, table: &Table) -> Result<Vec<Row>> {
        let bt = BTree::new(Kind::Table, &self.pager);
        let mut raw = Vec::new();
        bt.table_scan(&mut self.pager, table.root, &mut |id, p| {
            raw.push((id, p));
            Ok(())
        })?;
        raw.into_iter()
            .map(|(id, p)| self.decode_row(id, &p))
            .collect()
    }

    /// Rows of `table` in rowid order as a statement steps through them:
    /// the rows read before a corrupt page or record, and that error
    /// (`ResultSet.next()` throwing part-way).
    pub fn rows_until_error(&mut self, table: &Table) -> (Vec<Row>, Option<Error>) {
        let bt = BTree::new(Kind::Table, &self.pager);
        let mut raw = Vec::new();
        let scan = bt.table_scan(&mut self.pager, table.root, &mut |id, p| {
            raw.push((id, p));
            Ok(())
        });
        let mut rows = Vec::new();
        for (id, p) in raw {
            match self.decode_row(id, &p) {
                Ok(r) => rows.push(r),
                Err(e) => return (rows, Some(e)),
            }
        }
        (rows, scan.err())
    }

    /// The row with `rowid`.
    pub fn row(&mut self, table: &Table, rowid: i64) -> Result<Option<Row>> {
        let bt = BTree::new(Kind::Table, &self.pager);
        match bt.table_get(&mut self.pager, table.root, rowid)? {
            Some(p) => Ok(Some(self.decode_row(rowid, &p)?)),
            None => Ok(None),
        }
    }

    // ---- row changes --------------------------------------------------------------

    /// Full record values (the alias column as NULL) of an existing row,
    /// with defaults for columns its record predates.
    fn stored_values(table: &Table, row: &Row) -> Vec<Value> {
        (0..table.def.columns.len())
            .map(|i| {
                if table.is_rowid_alias(i) {
                    Value::Null
                } else {
                    match row.values.get(i) {
                        Some(v) => v.clone(),
                        None => {
                            let c = &table.def.columns[i];
                            apply_affinity(c.default_value(), c.affinity())
                        }
                    }
                }
            })
            .collect()
    }

    fn index_key(
        &mut self,
        ix: &IndexDef,
        t: &Table,
        vals: &[Value],
        rowid: i64,
    ) -> Result<Vec<u8>> {
        let mut key: Vec<Value> = ix
            .cols
            .iter()
            .map(|&c| {
                if t.is_rowid_alias(c) {
                    Value::Integer(rowid)
                } else {
                    vals[c].clone()
                }
            })
            .collect();
        key.push(Value::Integer(rowid));
        Ok(encode_record(&key, self.pager.enc, self.format4()?))
    }

    fn check_not_null(t: &Table, vals: &[Value]) -> Result<()> {
        for (i, c) in t.def.columns.iter().enumerate() {
            if c.not_null && !t.is_rowid_alias(i) && vals[i].is_null() {
                return Err(Error::Constraint(format!(
                    "NOT NULL constraint failed: {}.{}",
                    t.name, c.name
                )));
            }
        }
        Ok(())
    }

    fn unique_rowid_error(t: &Table) -> Error {
        let col = t
            .def
            .rowid_alias
            .map_or("rowid".to_string(), |i| t.def.columns[i].name.clone());
        Error::Constraint(format!("UNIQUE constraint failed: {}.{col}", t.name))
    }

    fn put_row(&mut self, tg: &Target, rowid: i64, vals: &[Value]) -> Result<()> {
        let rec = encode_record(vals, self.pager.enc, self.format4()?);
        let bt = BTree::new(Kind::Table, &self.pager);
        if !bt.table_insert(&mut self.pager, tg.table.root, rowid, &rec)? {
            return Err(Self::unique_rowid_error(&tg.table));
        }
        let ib = BTree::new(Kind::Index, &self.pager);
        for ix in &tg.indexes {
            let key = self.index_key(ix, &tg.table, vals, rowid)?;
            if !ib.index_insert(&mut self.pager, ix.root, &key)? {
                return Err(Error::Corrupt("duplicate index entry".into()));
            }
        }
        Ok(())
    }

    fn remove_row(&mut self, tg: &Target, rowid: i64, vals: &[Value]) -> Result<()> {
        let ib = BTree::new(Kind::Index, &self.pager);
        for ix in &tg.indexes {
            let key = self.index_key(ix, &tg.table, vals, rowid)?;
            if !ib.index_delete(&mut self.pager, ix.root, &key)? {
                return Err(Error::Corrupt(format!(
                    "index entry missing for row {rowid} of {}",
                    tg.table.name
                )));
            }
        }
        let bt = BTree::new(Kind::Table, &self.pager);
        if !bt.table_delete(&mut self.pager, tg.table.root, rowid)? {
            return Err(Error::Corrupt("row vanished".into()));
        }
        Ok(())
    }

    /// `INSERT INTO table(cols...) VALUES(...)`: returns the new rowid. A
    /// NULL or left-out `INTEGER PRIMARY KEY` gets the largest rowid plus
    /// one.
    pub fn insert(&mut self, table: &str, values: &[(&str, Value)]) -> Result<i64> {
        self.stmt(|c| {
            let tg = c.target(table)?;
            let t = &tg.table;
            let n = t.def.columns.len();
            let mut given: Vec<Option<Value>> = vec![None; n];
            for (name, v) in values {
                let i = t.column(name).map_err(|_| {
                    Error::Sql(format!("table {} has no column named {name}", t.name))
                })?;
                given[i] = Some(bound(v));
            }
            let mut rowid = None;
            if let Some(a) = t.def.rowid_alias {
                if let Some(v) = given[a].take().filter(|v| !v.is_null()) {
                    rowid = Some(must_be_int(&v)?);
                }
            }
            let vals: Vec<Value> = (0..n)
                .map(|i| {
                    let col = &t.def.columns[i];
                    if t.is_rowid_alias(i) {
                        return Value::Null;
                    }
                    let v = given[i].take().unwrap_or_else(|| col.default_value());
                    bound(&apply_affinity(v, col.affinity()))
                })
                .collect();
            Self::check_not_null(t, &vals)?;
            let bt = BTree::new(Kind::Table, &c.pager);
            let rowid = match rowid {
                Some(r) => {
                    if bt.table_get(&mut c.pager, t.root, r)?.is_some() {
                        return Err(Self::unique_rowid_error(t));
                    }
                    r
                }
                None => match bt.table_max_rowid(&mut c.pager, t.root)? {
                    None => 1,
                    Some(i64::MAX) => {
                        return Err(Error::Unsupported("rowid space exhausted".into()))
                    }
                    Some(m) => m.max(0) + 1,
                },
            };
            c.put_row(&tg, rowid, &vals)?;
            Ok(rowid)
        })
    }

    /// `UPDATE table SET col=?... WHERE rowid=?`: `false` when no row
    /// matched.
    pub fn update(&mut self, table: &str, rowid: i64, sets: &[(&str, Value)]) -> Result<bool> {
        self.stmt(|c| {
            let tg = c.target(table)?;
            let t = &tg.table;
            let Some(row) = c.row(t, rowid)? else {
                return Ok(false);
            };
            let old = Self::stored_values(t, &row);
            let mut new = old.clone();
            let mut new_rowid = rowid;
            for (name, v) in sets {
                let i = t.column(name)?;
                let v = bound(v);
                if t.is_rowid_alias(i) {
                    new_rowid = must_be_int(&v)?;
                } else {
                    let col = &t.def.columns[i];
                    new[i] = bound(&apply_affinity(v, col.affinity()));
                }
            }
            Self::check_not_null(t, &new)?;
            if new_rowid != rowid {
                let bt = BTree::new(Kind::Table, &c.pager);
                if bt.table_get(&mut c.pager, t.root, new_rowid)?.is_some() {
                    return Err(Self::unique_rowid_error(t));
                }
            }
            c.remove_row(&tg, rowid, &old)?;
            c.put_row(&tg, new_rowid, &new)?;
            Ok(true)
        })
    }

    /// `DELETE FROM table WHERE rowid=?`: `false` when no row matched.
    pub fn delete(&mut self, table: &str, rowid: i64) -> Result<bool> {
        self.stmt(|c| {
            let tg = c.target(table)?;
            let Some(row) = c.row(&tg.table, rowid)? else {
                return Ok(false);
            };
            let old = Self::stored_values(&tg.table, &row);
            c.remove_row(&tg, rowid, &old)?;
            Ok(true)
        })
    }

    /// `DELETE FROM table WHERE <pred>` as one statement: returns the
    /// number of rows deleted.
    pub fn delete_where(
        &mut self,
        table: &str,
        mut pred: impl FnMut(&Table, &Row) -> bool,
    ) -> Result<usize> {
        self.stmt(|c| {
            let tg = c.target(table)?;
            let rows = c.rows(&tg.table)?;
            let mut n = 0;
            for row in rows.iter().filter(|r| pred(&tg.table, r)) {
                let old = Self::stored_values(&tg.table, row);
                c.remove_row(&tg, row.rowid, &old)?;
                n += 1;
            }
            Ok(n)
        })
    }

    // ---- schema changes ------------------------------------------------------------

    fn bump_cookie(&mut self) -> Result<()> {
        let v = self.pager.header_u32(40)?.wrapping_add(1);
        self.pager.set_header_u32(40, v)
    }

    fn add_schema_row(&mut self, vals: [Value; 5]) -> Result<()> {
        let bt = BTree::new(Kind::Table, &self.pager);
        let id = bt.table_max_rowid(&mut self.pager, 1)?.unwrap_or(0) + 1;
        let rec = encode_record(&vals, self.pager.enc, self.format4()?);
        bt.table_insert(&mut self.pager, 1, id, &rec)?;
        Ok(())
    }

    /// Runs a schema statement (see [`crate::ddl`]).
    pub fn execute(&mut self, sql: &str) -> Result<()> {
        let d = ddl::parse(sql)?;
        if let Ddl::JournalMode(m) = &d {
            self.pager.journal_mode = match m.as_str() {
                "delete" => JournalMode::Delete,
                "truncate" => JournalMode::Truncate,
                "persist" => JournalMode::Persist,
                _ => return Err(Error::Unsupported(format!("journal_mode={m}"))),
            };
            return Ok(());
        }
        self.stmt(|c| c.run_ddl(d))
    }

    fn run_ddl(&mut self, d: Ddl) -> Result<()> {
        match d {
            Ddl::JournalMode(_) => Ok(()),
            Ddl::CreateTable {
                name,
                if_not_exists,
                sql,
            } => {
                let rows = self.schema_rows()?;
                if let Some((_, e)) = rows.iter().find(|(_, e)| eq_name(&e.name, &name)) {
                    if if_not_exists {
                        return Ok(());
                    }
                    return Err(Error::Sql(if e.kind == "index" {
                        format!("there is already an index named {name}")
                    } else {
                        format!("{} {name} already exists", e.kind)
                    }));
                }
                if name.len() >= 7 && name[..7].eq_ignore_ascii_case("sqlite_") {
                    return Err(Error::Sql(format!(
                        "object name reserved for internal use: {name}"
                    )));
                }
                let def = parse_create_table(&sql)?;
                for (i, col) in def.columns.iter().enumerate() {
                    if def.columns[..i].iter().any(|o| eq_name(&o.name, &col.name)) {
                        return Err(Error::Sql(format!("duplicate column name: {}", col.name)));
                    }
                }
                if def.autoindex {
                    return Err(Error::Unsupported(
                        "UNIQUE / non-rowid PRIMARY KEY constraints".into(),
                    ));
                }
                self.pager.init_empty()?;
                let root = self.pager.allocate()?;
                BTree::new(Kind::Table, &self.pager).init_root(&mut self.pager, root)?;
                self.add_schema_row([
                    Value::Text("table".into()),
                    Value::Text(name.clone()),
                    Value::Text(name),
                    Value::Integer(i64::from(root)),
                    Value::Text(sql),
                ])?;
                self.bump_cookie()
            }
            Ddl::CreateIndex {
                name,
                table,
                columns,
                unique,
                if_not_exists,
                sql,
            } => {
                let rows = self.schema_rows()?;
                if rows.iter().any(|(_, e)| eq_name(&e.name, &name)) {
                    if if_not_exists {
                        return Ok(());
                    }
                    return Err(Error::Sql(format!("index {name} already exists")));
                }
                if unique {
                    return Err(Error::Unsupported("UNIQUE index".into()));
                }
                let mut tg = self.target(&table)?;
                let cols = columns
                    .iter()
                    .map(|c| tg.table.column(c))
                    .collect::<Result<Vec<_>>>()?;
                let root = self.pager.allocate()?;
                BTree::new(Kind::Index, &self.pager).init_root(&mut self.pager, root)?;
                let ix = IndexDef { root, cols };
                let ib = BTree::new(Kind::Index, &self.pager);
                for row in self.rows(&tg.table)? {
                    let vals = Self::stored_values(&tg.table, &row);
                    let key = self.index_key(&ix, &tg.table, &vals, row.rowid)?;
                    ib.index_insert(&mut self.pager, root, &key)?;
                }
                tg.indexes.push(ix);
                self.add_schema_row([
                    Value::Text("index".into()),
                    Value::Text(name),
                    Value::Text(tg.table.name.clone()),
                    Value::Integer(i64::from(root)),
                    Value::Text(sql),
                ])?;
                self.bump_cookie()
            }
            Ddl::AddColumn { table, column, def } => {
                let rows = self.schema_rows()?;
                let t = Self::table_from(&rows, &table)?;
                let parsed = parse_create_table(&format!("CREATE TABLE x({def})"))?;
                let [col] = parsed.columns.as_slice() else {
                    return Err(Error::Sql(format!("near \"{def}\": syntax error")));
                };
                if t.def.columns.iter().any(|c| eq_name(&c.name, &column)) {
                    return Err(Error::Sql(format!("duplicate column name: {column}")));
                }
                if col.primary_key {
                    return Err(Error::Sql("Cannot add a PRIMARY KEY column".into()));
                }
                if col.unique {
                    return Err(Error::Sql("Cannot add a UNIQUE column".into()));
                }
                if matches!(col.default, Some(Literal::Unsupported)) {
                    return Err(Error::Unsupported(format!("column default in: {def}")));
                }
                if col.not_null && matches!(col.default, None | Some(Literal::Null)) {
                    return Err(Error::Sql(
                        "Cannot add a NOT NULL column with default value NULL".into(),
                    ));
                }
                let (rowid, e) = rows
                    .iter()
                    .find(|(_, e)| e.kind == "table" && eq_name(&e.name, &t.name))
                    .ok_or_else(|| Error::NoSuchTable(table.clone()))?;
                let old_sql = e.sql.clone().unwrap_or_default();
                let off = ddl::add_col_offset(&old_sql)?;
                let new_sql = format!("{}, {def}{}", &old_sql[..off], &old_sql[off..]);
                let bt = BTree::new(Kind::Table, &self.pager);
                bt.table_delete(&mut self.pager, 1, *rowid)?;
                let rec = encode_record(
                    &[
                        Value::Text(e.kind.clone()),
                        Value::Text(e.name.clone()),
                        Value::Text(e.tbl_name.clone()),
                        Value::Integer(e.rootpage),
                        Value::Text(new_sql),
                    ],
                    self.pager.enc,
                    self.format4()?,
                );
                bt.table_insert(&mut self.pager, 1, *rowid, &rec)?;
                // The schema format is at least 3 after ADD COLUMN.
                if self.pager.header_u32(44)? < 3 {
                    self.pager.set_header_u32(44, 3)?;
                }
                self.bump_cookie()
            }
            Ddl::DropTable { name, if_exists } => {
                let rows = self.schema_rows()?;
                let Some((_, t)) = rows
                    .iter()
                    .find(|(_, e)| e.kind == "table" && eq_name(&e.name, &name))
                else {
                    if if_exists {
                        return Ok(());
                    }
                    return Err(Error::NoSuchTable(name));
                };
                if t.name.len() >= 7 && t.name[..7].eq_ignore_ascii_case("sqlite_") {
                    return Err(Error::Sql(format!("table {} may not be dropped", t.name)));
                }
                let tbl = t.name.clone();
                let bt = BTree::new(Kind::Table, &self.pager);
                for (id, e) in rows.iter().filter(|(_, e)| eq_name(&e.tbl_name, &tbl)) {
                    if let Ok(root) = u32::try_from(e.rootpage) {
                        if root != 0 {
                            let kind = if e.kind == "index" {
                                Kind::Index
                            } else {
                                Kind::Table
                            };
                            BTree::new(kind, &self.pager).free_tree(&mut self.pager, root, 0)?;
                        }
                    }
                    bt.table_delete(&mut self.pager, 1, *id)?;
                }
                self.bump_cookie()
            }
        }
    }
}
