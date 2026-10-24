//! A small read-only reader for SQLite 3 database files, in pure Rust.
//!
//! The game keeps per-save player and vehicle records in SQLite files
//! (`players.db`, `vehicles.db`) and reads them through sqlite-jdbc. This
//! crate covers what those reads need, following the "Database File Format"
//! document of SQLite:
//!
//! * the 100-byte file header (page size with `1` meaning 65536, reserved
//!   bytes per page, text encoding, in-header page count validity),
//! * table b-tree pages (interior `0x05`, leaf `0x0D`), cell pointer arrays
//!   and the overflow page chains of large payloads,
//! * the record format and its serial types,
//! * `sqlite_schema` (page 1) to find a table's root page and columns,
//! * full scans in rowid order and rowid lookups.
//!
//! There is no SQL engine: callers implement their queries on top of
//! [`Database::for_each_row`] and [`Database::row_by_rowid`].
//!
//! Not supported: index b-trees (not needed for rowid scans), `WITHOUT
//! ROWID` tables, generated columns, and WAL databases whose `-wal` file is
//! non-empty (an error is returned). A hot rollback journal is not replayed;
//! the main file is read as it is. The file is only ever opened for reading.

pub mod record;
pub mod schema;
#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "writer"))]
pub mod writer;

use std::cell::RefCell;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub use record::Value;
pub use schema::{Affinity, Column, TableDef};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The file is not an SQLite 3 database (`SQLITE_NOTADB`).
    NotADatabase(String),
    /// Structural damage (`SQLITE_CORRUPT`).
    Corrupt(String),
    /// `no such table: <name>`.
    NoSuchTable(String),
    /// `no such column: <name>`.
    NoSuchColumn(String),
    /// A valid database using a feature this reader does not implement.
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::NotADatabase(s) => write!(f, "file is not a database: {s}"),
            Error::Corrupt(s) => write!(f, "database disk image is malformed: {s}"),
            Error::NoSuchTable(s) => write!(f, "no such table: {s}"),
            Error::NoSuchColumn(s) => write!(f, "no such column: {s}"),
            Error::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf16le,
    Utf16be,
}

/// The fields of the database header this reader uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Bytes per page (512..=65536).
    pub page_size: u32,
    pub write_version: u8,
    pub read_version: u8,
    /// Unused bytes at the end of every page.
    pub reserved: u8,
    /// Usable bytes per page: `page_size - reserved`.
    pub usable: u32,
    /// Pages in the database.
    pub page_count: u32,
    pub schema_format: u32,
    pub text_encoding: TextEncoding,
}

const MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// B-tree recursion limit; deeper trees only come from corrupt files.
const MAX_DEPTH: u32 = 64;

fn be16(b: &[u8], at: usize) -> u32 {
    u32::from(u16::from_be_bytes([b[at], b[at + 1]]))
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Header {
    /// Parses the first 100 bytes of a database file of `file_len` bytes.
    pub fn parse(b: &[u8], file_len: u64) -> Result<Header> {
        let not_db = |s: &str| Error::NotADatabase(s.to_string());
        if b.len() < 100 || &b[..16] != MAGIC {
            return Err(not_db("bad magic"));
        }
        let page_size = match be16(b, 16) {
            1 => 65536,
            n if (512..=32768).contains(&n) && n.is_power_of_two() => n,
            _ => return Err(not_db("bad page size")),
        };
        let (write_version, read_version) = (b[18], b[19]);
        if read_version > 2 {
            return Err(not_db("unknown read version"));
        }
        let reserved = b[20];
        // Fixed payload fractions.
        if b[21] != 64 || b[22] != 32 || b[23] != 32 {
            return Err(not_db("bad payload fractions"));
        }
        let usable = page_size - u32::from(reserved);
        if usable < 480 {
            return Err(not_db("usable page size below 480"));
        }
        let file_pages = u32::try_from(file_len / u64::from(page_size)).unwrap_or(u32::MAX);
        // The in-header size is valid only when non-zero and the change
        // counter matches "version-valid-for" (btree.c lockBtree).
        let header_pages = be32(b, 28);
        let page_count = if header_pages == 0 || b[24..28] != b[92..96] {
            file_pages
        } else if header_pages > file_pages {
            return Err(Error::Corrupt("in-header page count exceeds file".into()));
        } else {
            header_pages
        };
        let text_encoding = match be32(b, 56) {
            0 | 1 => TextEncoding::Utf8,
            2 => TextEncoding::Utf16le,
            3 => TextEncoding::Utf16be,
            _ => return Err(not_db("bad text encoding")),
        };
        Ok(Header {
            page_size,
            write_version,
            read_version,
            reserved,
            usable,
            page_count,
            schema_format: be32(b, 44),
            text_encoding,
        })
    }
}

enum Source {
    File(RefCell<File>),
    Bytes(Vec<u8>),
}

/// An open, read-only database.
pub struct Database {
    src: Source,
    /// `None` for an empty (zero-length) file, which SQLite treats as a
    /// database without tables.
    pub header: Option<Header>,
}

/// One `sqlite_schema` row.
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaEntry {
    pub kind: String,
    pub name: String,
    pub tbl_name: String,
    pub rootpage: i64,
    pub sql: Option<String>,
}

/// A rowid table found in the schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub root: u32,
    pub def: TableDef,
}

impl Table {
    /// Column index by name (ASCII case-insensitive, as SQLite resolves
    /// identifiers).
    pub fn column(&self, name: &str) -> Result<usize> {
        self.def
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| Error::NoSuchColumn(name.to_string()))
    }

    /// Whether `col` is the rowid alias (`INTEGER PRIMARY KEY`).
    pub fn is_rowid_alias(&self, col: usize) -> bool {
        self.def.rowid_alias == Some(col)
    }
}

/// A row of a table b-tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub rowid: i64,
    /// Record values in column order. May be shorter than the table's
    /// column list for rows written before an `ALTER TABLE ... ADD`.
    pub values: Vec<Value>,
}

impl Row {
    /// Column `col` of `table` as a query sees it: the rowid for the alias
    /// column, NULL for columns missing from an older record, and integers
    /// read back as reals in REAL-affinity columns (SQLite stores integral
    /// reals as integers there and `OP_RealAffinity` undoes it).
    pub fn get(&self, table: &Table, col: usize) -> Value {
        if table.is_rowid_alias(col) {
            return Value::Integer(self.rowid);
        }
        match self.values.get(col) {
            None => Value::Null,
            Some(Value::Integer(i))
                if table
                    .def
                    .columns
                    .get(col)
                    .is_some_and(|c| c.affinity() == Affinity::Real) =>
            {
                Value::Real(*i as f64)
            }
            Some(v) => v.clone(),
        }
    }
}

/// A parsed b-tree page.
struct Page {
    data: Vec<u8>,
    /// Offset of the b-tree header (100 on page 1).
    hdr: usize,
    kind: u8,
    ncells: usize,
    /// Right-most child of an interior page.
    right: u32,
}

impl Page {
    fn is_leaf(&self) -> bool {
        self.kind == 0x0D
    }

    fn header_len(&self) -> usize {
        if self.is_leaf() {
            8
        } else {
            12
        }
    }
}

impl Database {
    /// Opens `path` read-only.
    pub fn open(path: impl AsRef<Path>) -> Result<Database> {
        let path = path.as_ref();
        let mut f = File::open(path)?;
        let len = f.metadata()?.len();
        let header = if len == 0 {
            None
        } else {
            let mut b = vec![0u8; 100];
            let n = read_up_to(&mut f, &mut b)?;
            b.truncate(n);
            Some(Header::parse(&b, len)?)
        };
        if header.as_ref().is_some_and(|h| h.read_version == 2) {
            let mut wal = path.as_os_str().to_owned();
            wal.push("-wal");
            if std::fs::metadata(&wal).is_ok_and(|m| m.len() > 0) {
                return Err(Error::Unsupported(
                    "WAL database with a non-empty -wal file".into(),
                ));
            }
        }
        Ok(Database {
            src: Source::File(RefCell::new(f)),
            header,
        })
    }

    /// A database held in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Database> {
        let header = if bytes.is_empty() {
            None
        } else {
            Some(Header::parse(
                &bytes[..bytes.len().min(100)],
                bytes.len() as u64,
            )?)
        };
        Ok(Database {
            src: Source::Bytes(bytes),
            header,
        })
    }

    fn hdr(&self) -> Result<&Header> {
        self.header
            .as_ref()
            .ok_or_else(|| Error::Corrupt("empty database".into()))
    }

    fn raw_page(&self, pgno: u32) -> Result<Vec<u8>> {
        let h = self.hdr()?;
        if pgno == 0 || pgno > h.page_count {
            return Err(Error::Corrupt(format!("page {pgno} out of range")));
        }
        let size = h.page_size as usize;
        let off = u64::from(pgno - 1) * u64::from(h.page_size);
        match &self.src {
            Source::Bytes(b) => {
                let start = off as usize;
                Ok(b.get(start..start + size)
                    .ok_or_else(|| Error::Corrupt("short file".into()))?
                    .to_vec())
            }
            Source::File(f) => {
                let mut f = f.borrow_mut();
                f.seek(SeekFrom::Start(off))?;
                let mut buf = vec![0u8; size];
                if read_up_to(&mut *f, &mut buf)? != size {
                    return Err(Error::Corrupt("short file".into()));
                }
                Ok(buf)
            }
        }
    }

    fn btree_page(&self, pgno: u32) -> Result<Page> {
        let data = self.raw_page(pgno)?;
        let hdr = if pgno == 1 { 100 } else { 0 };
        let kind = data[hdr];
        if kind != 0x05 && kind != 0x0D {
            return Err(Error::Corrupt(format!(
                "page {pgno} is not a table b-tree page (type {kind:#x})"
            )));
        }
        let ncells = be16(&data, hdr + 3) as usize;
        let right = if kind == 0x05 {
            be32(&data, hdr + 8)
        } else {
            0
        };
        let page = Page {
            data,
            hdr,
            kind,
            ncells,
            right,
        };
        let usable = self.hdr()?.usable as usize;
        if hdr + page.header_len() + 2 * ncells > usable {
            return Err(Error::Corrupt(format!("page {pgno}: too many cells")));
        }
        Ok(page)
    }

    /// Offset of cell `i`, checked to lie in the cell content area.
    fn cell_offset(&self, p: &Page, i: usize) -> Result<usize> {
        let at = p.hdr + p.header_len() + 2 * i;
        let off = be16(&p.data, at) as usize;
        if off < p.hdr + p.header_len() + 2 * p.ncells || off >= self.hdr()?.usable as usize {
            return Err(Error::Corrupt("cell pointer out of range".into()));
        }
        Ok(off)
    }

    /// Bytes of a table-leaf payload of `p` bytes stored on the page itself
    /// ("1.6. B-tree Pages": X = U-35, M = ((U-12)*32/255)-23,
    /// K = M+((P-M)%(U-4))).
    fn local_payload(&self, p: u64) -> Result<usize> {
        let u = u64::from(self.hdr()?.usable);
        let x = u - 35;
        if p <= x {
            return Ok(p as usize);
        }
        let m = ((u - 12) * 32 / 255) - 23;
        let k = m + ((p - m) % (u - 4));
        Ok(if k <= x { k as usize } else { m as usize })
    }

    /// Reads a table leaf cell: `(rowid, offset of payload, payload size)`.
    fn leaf_cell_head(&self, p: &Page, i: usize) -> Result<(i64, usize, u64)> {
        let off = self.cell_offset(p, i)?;
        let corrupt = || Error::Corrupt("malformed cell".into());
        let usable = self.hdr()?.usable as usize;
        let (size, n1) = record::varint(&p.data[off..usable]).ok_or_else(corrupt)?;
        let (rowid, n2) = record::varint(&p.data[off + n1..usable]).ok_or_else(corrupt)?;
        Ok((rowid as i64, off + n1 + n2, size))
    }

    /// The full payload of a leaf cell, following its overflow chain.
    fn leaf_payload(&self, p: &Page, start: usize, size: u64) -> Result<Vec<u8>> {
        let h = self.hdr()?;
        let usable = h.usable as usize;
        let corrupt = |s: &str| Error::Corrupt(s.to_string());
        let size_us = usize::try_from(size).map_err(|_| corrupt("payload too large"))?;
        if size > u64::from(h.page_count) * u64::from(h.usable) {
            return Err(corrupt("payload larger than the database"));
        }
        let local = self.local_payload(size)?;
        let end = start + local;
        if end > usable {
            return Err(corrupt("cell overflows page"));
        }
        let mut out = Vec::with_capacity(size_us);
        out.extend_from_slice(&p.data[start..end]);
        if local == size_us {
            return Ok(out);
        }
        if end + 4 > usable {
            return Err(corrupt("cell overflows page"));
        }
        let mut next = be32(&p.data, end);
        let mut hops = 0u32;
        while out.len() < size_us {
            hops += 1;
            if next == 0 || hops > h.page_count {
                return Err(corrupt("broken overflow chain"));
            }
            let page = self.raw_page(next)?;
            next = be32(&page, 0);
            let take = (size_us - out.len()).min(usable - 4);
            out.extend_from_slice(&page[4..4 + take]);
        }
        Ok(out)
    }

    fn leaf_row(&self, p: &Page, i: usize) -> Result<Row> {
        let (rowid, start, size) = self.leaf_cell_head(p, i)?;
        let payload = self.leaf_payload(p, start, size)?;
        let values = record::decode_record(&payload, self.hdr()?.text_encoding)?;
        Ok(Row { rowid, values })
    }

    /// Interior cell: `(left child, key)`.
    fn interior_cell(&self, p: &Page, i: usize) -> Result<(u32, i64)> {
        let off = self.cell_offset(p, i)?;
        let usable = self.hdr()?.usable as usize;
        if off + 4 > usable {
            return Err(Error::Corrupt("malformed cell".into()));
        }
        let child = be32(&p.data, off);
        let (key, _) = record::varint(&p.data[off + 4..usable])
            .ok_or_else(|| Error::Corrupt("malformed cell".into()))?;
        Ok((child, key as i64))
    }

    fn walk(&self, pgno: u32, depth: u32, f: &mut dyn FnMut(Row) -> bool) -> Result<bool> {
        if depth > MAX_DEPTH {
            return Err(Error::Corrupt("b-tree too deep".into()));
        }
        let p = self.btree_page(pgno)?;
        if p.is_leaf() {
            for i in 0..p.ncells {
                if !f(self.leaf_row(&p, i)?) {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        for i in 0..p.ncells {
            let (child, _) = self.interior_cell(&p, i)?;
            if !self.walk(child, depth + 1, f)? {
                return Ok(false);
            }
        }
        self.walk(p.right, depth + 1, f)
    }

    /// Visits the rows of the table b-tree rooted at `root` in rowid order
    /// (the order of a plain `SELECT` without `ORDER BY`) until `f` returns
    /// false.
    pub fn for_each_row_in(&self, root: u32, mut f: impl FnMut(Row) -> bool) -> Result<()> {
        self.walk(root, 0, &mut f).map(|_| ())
    }

    pub fn for_each_row(&self, table: &Table, f: impl FnMut(Row) -> bool) -> Result<()> {
        self.for_each_row_in(table.root, f)
    }

    /// All rows of `table` in rowid order.
    pub fn rows(&self, table: &Table) -> Result<Vec<Row>> {
        let mut out = Vec::new();
        self.for_each_row(table, |r| {
            out.push(r);
            true
        })?;
        Ok(out)
    }

    /// The row with `rowid`, found by descending the b-tree (`WHERE
    /// rowid = ?`).
    pub fn row_by_rowid(&self, table: &Table, rowid: i64) -> Result<Option<Row>> {
        let mut pgno = table.root;
        for _ in 0..=MAX_DEPTH {
            let p = self.btree_page(pgno)?;
            if p.is_leaf() {
                for i in 0..p.ncells {
                    let (id, start, size) = self.leaf_cell_head(&p, i)?;
                    if id == rowid {
                        let payload = self.leaf_payload(&p, start, size)?;
                        let values = record::decode_record(&payload, self.hdr()?.text_encoding)?;
                        return Ok(Some(Row { rowid, values }));
                    }
                }
                return Ok(None);
            }
            // Keys in the left subtree of a cell are <= the cell's key.
            let mut next = p.right;
            for i in 0..p.ncells {
                let (child, key) = self.interior_cell(&p, i)?;
                if rowid <= key {
                    next = child;
                    break;
                }
            }
            pgno = next;
        }
        Err(Error::Corrupt("b-tree too deep".into()))
    }

    /// The rows of `sqlite_schema`.
    pub fn schema(&self) -> Result<Vec<SchemaEntry>> {
        if self.header.is_none() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut err = None;
        self.for_each_row_in(1, |r| {
            let text = |i: usize| r.values.get(i).and_then(Value::as_text);
            match (text(0), text(1), text(2)) {
                (Some(kind), Some(name), Some(tbl_name)) => {
                    let rootpage = r.values.get(3).map_or(0, Value::as_i64);
                    out.push(SchemaEntry {
                        kind,
                        name,
                        tbl_name,
                        rootpage,
                        sql: text(4),
                    });
                    true
                }
                _ => {
                    err = Some(Error::Corrupt("malformed schema row".into()));
                    false
                }
            }
        })?;
        match err {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }

    /// Looks up a rowid table by name (ASCII case-insensitive).
    pub fn table(&self, name: &str) -> Result<Table> {
        let e = self
            .schema()?
            .into_iter()
            .find(|e| e.kind == "table" && e.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| Error::NoSuchTable(name.to_string()))?;
        let root = u32::try_from(e.rootpage)
            .ok()
            .filter(|&r| r != 0)
            .ok_or_else(|| {
                Error::Unsupported(format!("table {} has no b-tree (virtual table?)", e.name))
            })?;
        let sql = e
            .sql
            .ok_or_else(|| Error::Corrupt(format!("table {} has no SQL", e.name)))?;
        let def = schema::parse_create_table(&sql)?;
        Ok(Table {
            name: e.name,
            root,
            def,
        })
    }
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}
