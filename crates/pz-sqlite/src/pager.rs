//! Page cache, free-page list and rollback journal for writing.
//!
//! A write transaction collects modified pages in memory. Commit follows
//! SQLite's rollback-journal protocol ("Atomic Commit In SQLite", file
//! format "4. The Rollback Journal"):
//!
//! 1. the original content of every modified page that exists in the file
//!    is written to `<db>-journal` after a header holding the original page
//!    count, and the journal is synced (and its directory, when it is new);
//! 2. the new pages are written to the database, which is synced;
//! 3. the commit point: the journal is deleted (`journal_mode=DELETE`, the
//!    default), truncated to zero bytes and synced (`TRUNCATE`, which the
//!    game sets on its store connections) or has its header zeroed
//!    (`PERSIST`).
//!
//! A crash before (3) leaves a hot journal. [`Pager::open`] plays it back
//! (as SQLite does on its next open), which restores the database to its
//! state before the transaction; the journal is written in SQLite's format,
//! so `sqlite3` and sqlite-jdbc roll it back the same way.
//!
//! No file locks are taken: the database must not be written by another
//! process at the same time.

use crate::{be32, Error, Header, Result, TextEncoding};
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const JOURNAL_MAGIC: [u8; 8] = [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7];
/// Sector size recorded in journals this pager writes.
const SECTOR: usize = 512;
/// `PENDING_BYTE`: the page holding this offset is never used.
const PENDING_BYTE: u64 = 0x4000_0000;
/// `SQLITE_VERSION_NUMBER` written at offset 96 (the sqlite-jdbc build the
/// game ships writes 3048000; any version is valid there).
pub const VERSION_NUMBER: u32 = 3_048_000;
/// Page size of new databases (`SQLITE_DEFAULT_PAGE_SIZE`).
pub const DEFAULT_PAGE_SIZE: u32 = 4096;

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

fn check_write_format(header: &[u8]) -> Result<()> {
    // A nonzero largest-root field means FULL or INCREMENTAL
    // auto-vacuum, including an otherwise empty one-page file.
    // These formats require reserved ptrmap pages, reverse pointer
    // maintenance and root relocation. The allocator below implements
    // only the ordinary freelist format.
    // https://www.sqlite.org/fileformat.html#ptrmap
    if header.len() >= 100 && &header[..16] == b"SQLite format 3\0" && be32(header, 52) != 0 {
        return Err(Error::Unsupported(
            "writing an auto-vacuum database (FULL or INCREMENTAL)".into(),
        ));
    }
    Ok(())
}

fn journal_path(path: &Path) -> PathBuf {
    let mut j = path.as_os_str().to_owned();
    j.push("-journal");
    PathBuf::from(j)
}

/// Journal page checksum (`pager_cksum`): the nonce plus every 200th byte
/// counted back from `page_size - 200`.
fn cksum(nonce: u32, data: &[u8]) -> u32 {
    let mut c = nonce;
    let mut i = data.len() as isize - 200;
    while i > 0 {
        c = c.wrapping_add(u32::from(data[i as usize]));
        i -= 200;
    }
    c
}

fn nonce() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.finish() as u32
}

fn read_at(f: &mut File, off: u64, buf: &mut [u8]) -> std::io::Result<usize> {
    f.seek(SeekFrom::Start(off))?;
    crate::read_up_to(f, buf)
}

/// Plays back a hot journal into `db` (`pager_playback` for a hot
/// journal). Returns whether anything was rolled back.
fn playback(db: &mut File, jpath: &Path) -> Result<bool> {
    let mut j = match File::open(jpath) {
        Ok(j) => j,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let jlen = j.metadata()?.len();
    let mut hdr = [0u8; 28];
    if jlen < 28 || read_at(&mut j, 0, &mut hdr)? < 28 || hdr[..8] != JOURNAL_MAGIC {
        // Empty or zeroed: not a hot journal.
        return Ok(false);
    }
    let mut off: u64 = 0;
    let mut first = true;
    let mut orig_size = 0u32;
    loop {
        let mut hdr = [0u8; 28];
        if off + 28 > jlen || read_at(&mut j, off, &mut hdr)? < 28 || hdr[..8] != JOURNAL_MAGIC {
            break;
        }
        let mut nrec = be32(&hdr, 8);
        let nonce = be32(&hdr, 12);
        let sector = be32(&hdr, 20) as u64;
        let psize = be32(&hdr, 24) as usize;
        if !(32..=65536).contains(&sector)
            || !sector.is_power_of_two()
            || !(512..=65536).contains(&psize)
            || !psize.is_power_of_two()
        {
            break;
        }
        if first {
            // The database returns to its original size before the pages
            // are restored; pages past it are skipped.
            orig_size = be32(&hdr, 16);
            db.set_len(u64::from(orig_size) * psize as u64)?;
        }
        off += sector;
        let rec = 8 + psize as u64;
        if nrec == 0xffff_ffff {
            nrec = ((jlen - off.min(jlen)) / rec) as u32;
        }
        let mut page = vec![0u8; psize];
        let mut stop = false;
        for _ in 0..nrec {
            let mut b4 = [0u8; 4];
            if off + rec > jlen || read_at(&mut j, off, &mut b4)? < 4 {
                stop = true;
                break;
            }
            let pgno = u32::from_be_bytes(b4);
            read_at(&mut j, off + 4, &mut page)?;
            read_at(&mut j, off + 4 + psize as u64, &mut b4)?;
            off += rec;
            let pending = (PENDING_BYTE / psize as u64) as u32 + 1;
            if pgno == 0 || pgno == pending || cksum(nonce, &page) != u32::from_be_bytes(b4) {
                stop = true;
                break;
            }
            if pgno <= orig_size {
                db.seek(SeekFrom::Start(u64::from(pgno - 1) * psize as u64))?;
                db.write_all(&page)?;
            }
        }
        first = false;
        if stop {
            break;
        }
        // The next header starts on a sector boundary.
        off = off.div_ceil(sector) * sector;
    }
    db.sync_all()?;
    Ok(true)
}

/// Removes a played-back journal, as a new connection (in the default
/// `journal_mode=DELETE`) does.
fn finish_journal(jpath: &Path) -> Result<()> {
    match std::fs::remove_file(jpath) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Syncs the directory holding `path` so a new directory entry survives.
fn sync_dir(path: &Path) {
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
}

/// `PRAGMA journal_mode` values this pager implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalMode {
    Delete,
    Truncate,
    Persist,
}

pub(crate) struct Savepoint {
    dirty: BTreeMap<u32, Vec<u8>>,
    npages: u32,
}

pub(crate) struct Pager {
    file: File,
    path: PathBuf,
    pub page_size: u32,
    pub usable: u32,
    pub enc: TextEncoding,
    /// Pages in the file as last committed.
    db_size: u32,
    /// Pages in the database including this transaction's changes.
    pub npages: u32,
    /// Committed page contents read so far.
    clean: HashMap<u32, Vec<u8>>,
    /// Pages changed by the open transaction.
    dirty: BTreeMap<u32, Vec<u8>>,
    pub journal_mode: JournalMode,
}

impl Pager {
    /// Opens (creating when missing) `path` for reading and writing, after
    /// rolling back a hot journal.
    pub fn open(path: &Path) -> Result<Pager> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        // Refuse an identifiable unsupported format before recovery can
        // modify its database or remove its hot journal. Its owner must
        // recover it with a writer that supports that format.
        let mut header = [0u8; 100];
        let read = read_at(&mut file, 0, &mut header)?;
        check_write_format(&header[..read])?;
        let jpath = journal_path(path);
        if playback(&mut file, &jpath)? {
            finish_journal(&jpath)?;
        }
        let len = file.metadata()?.len();
        let mut p = Pager {
            file,
            path: path.to_path_buf(),
            page_size: DEFAULT_PAGE_SIZE,
            usable: DEFAULT_PAGE_SIZE,
            enc: TextEncoding::Utf8,
            db_size: 0,
            npages: 0,
            clean: HashMap::new(),
            dirty: BTreeMap::new(),
            journal_mode: JournalMode::Delete,
        };
        if len > 0 {
            let mut b = vec![0u8; 100];
            let n = read_at(&mut p.file, 0, &mut b)?;
            b.truncate(n);
            let h = Header::parse(&b, len)?;
            if h.write_version != 1 || h.read_version != 1 {
                return Err(Error::Unsupported(
                    "writing a WAL-mode or newer-format database".into(),
                ));
            }
            // Recovery may have restored a different page-one header.
            check_write_format(&b)?;
            p.page_size = h.page_size;
            p.usable = h.usable;
            p.enc = h.text_encoding;
            p.db_size = h.page_count;
            p.npages = h.page_count;
        }
        Ok(p)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Page `pgno` as the open transaction sees it.
    pub fn get(&mut self, pgno: u32) -> Result<&[u8]> {
        if pgno == 0 || pgno > self.npages {
            return Err(Error::Corrupt(format!("page {pgno} out of range")));
        }
        if self.dirty.contains_key(&pgno) {
            return Ok(&self.dirty[&pgno]);
        }
        self.load_clean(pgno)?;
        Ok(&self.clean[&pgno])
    }

    fn load_clean(&mut self, pgno: u32) -> Result<()> {
        if self.clean.contains_key(&pgno) {
            return Ok(());
        }
        let size = self.page_size as usize;
        let mut buf = vec![0u8; size];
        if pgno <= self.db_size {
            let off = u64::from(pgno - 1) * u64::from(self.page_size);
            if read_at(&mut self.file, off, &mut buf)? != size {
                return Err(Error::Corrupt("short file".into()));
            }
        }
        self.clean.insert(pgno, buf);
        Ok(())
    }

    /// Page `pgno` for writing.
    pub fn get_mut(&mut self, pgno: u32) -> Result<&mut Vec<u8>> {
        if pgno == 0 || pgno > self.npages {
            return Err(Error::Corrupt(format!("page {pgno} out of range")));
        }
        if !self.dirty.contains_key(&pgno) {
            self.load_clean(pgno)?;
            let p = self.clean[&pgno].clone();
            self.dirty.insert(pgno, p);
        }
        Ok(self.dirty.get_mut(&pgno).unwrap())
    }

    pub fn in_write(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Initialises page 1 of an empty database: the header and an empty
    /// `sqlite_schema` leaf, as SQLite writes a new file.
    pub fn init_empty(&mut self) -> Result<()> {
        if self.npages != 0 {
            return Ok(());
        }
        self.npages = 1;
        let ps = self.page_size;
        let h = self.get_mut(1)?;
        h.fill(0);
        h[..16].copy_from_slice(b"SQLite format 3\0");
        let ps16 = if ps == 65536 { 1u16 } else { ps as u16 };
        h[16..18].copy_from_slice(&ps16.to_be_bytes());
        h[18] = 1;
        h[19] = 1;
        h[20] = 0;
        h[21] = 64;
        h[22] = 32;
        h[23] = 32;
        // Schema format 4, UTF-8.
        put32(h, 44, 4);
        put32(h, 56, 1);
        // Empty table leaf for sqlite_schema; the content area starts at the
        // end of the page.
        h[100] = 0x0D;
        let top = if ps == 65536 { 0 } else { ps as u16 };
        h[105..107].copy_from_slice(&top.to_be_bytes());
        Ok(())
    }

    pub fn header_u32(&mut self, at: usize) -> Result<u32> {
        Ok(be32(self.get(1)?, at))
    }

    pub fn set_header_u32(&mut self, at: usize, v: u32) -> Result<()> {
        put32(self.get_mut(1)?, at, v);
        Ok(())
    }

    fn pending_page(&self) -> u32 {
        (PENDING_BYTE / u64::from(self.page_size)) as u32 + 1
    }

    /// Allocates a page from the free list, or at the end of the file. The
    /// page is returned zeroed and dirty.
    pub fn allocate(&mut self) -> Result<u32> {
        let count = self.header_u32(36)?;
        let trunk = self.header_u32(32)?;
        let pgno = if count > 0 && trunk != 0 {
            let max = self.usable / 4 - 2;
            let t = self.get(trunk)?;
            let next = be32(t, 0);
            let leaves = be32(t, 4);
            if leaves > max {
                return Err(Error::Corrupt("free-list trunk overfull".into()));
            }
            let pg = if leaves > 0 {
                let leaf = be32(t, 8 + 4 * (leaves as usize - 1));
                put32(self.get_mut(trunk)?, 4, leaves - 1);
                leaf
            } else {
                self.set_header_u32(32, next)?;
                trunk
            };
            self.set_header_u32(36, count - 1)?;
            if pg == 0 || pg > self.npages {
                return Err(Error::Corrupt("free-list page out of range".into()));
            }
            pg
        } else {
            self.npages += 1;
            if self.npages == self.pending_page() {
                self.npages += 1;
            }
            self.npages
        };
        self.get_mut(pgno)?.fill(0);
        Ok(pgno)
    }

    /// Adds `pgno` to the free list (`freePage2`): as a leaf of the first
    /// trunk while it has room, otherwise as a new trunk.
    pub fn free(&mut self, pgno: u32) -> Result<()> {
        if pgno < 2 || pgno > self.npages {
            return Err(Error::Corrupt(format!("freeing page {pgno}")));
        }
        let count = self.header_u32(36)?;
        let trunk = self.header_u32(32)?;
        self.set_header_u32(36, count + 1)?;
        if trunk != 0 {
            let leaves = be32(self.get(trunk)?, 4);
            // SQLite keeps 8 slots spare for older readers.
            if leaves < self.usable / 4 - 8 {
                let t = self.get_mut(trunk)?;
                put32(t, 4, leaves + 1);
                put32(t, 8 + 4 * leaves as usize, pgno);
                return Ok(());
            }
        }
        let p = self.get_mut(pgno)?;
        p.fill(0);
        put32(p, 0, trunk);
        self.set_header_u32(32, pgno)
    }

    /// The state of the open transaction, to undo one statement.
    pub fn savepoint(&self) -> Savepoint {
        Savepoint {
            dirty: self.dirty.clone(),
            npages: self.npages,
        }
    }

    pub fn restore(&mut self, sp: Savepoint) {
        self.dirty = sp.dirty;
        self.npages = sp.npages;
    }

    /// Discards the open transaction.
    pub fn rollback(&mut self) {
        self.dirty.clear();
        self.npages = self.db_size;
    }

    /// Commits the open transaction atomically (see the module docs).
    pub fn commit(&mut self) -> Result<()> {
        if self.dirty.is_empty() {
            return Ok(());
        }
        let r = self.commit_inner();
        if r.is_err() {
            // The journal (if written) still holds the original pages; the
            // next open rolls it back. Forget the cached file contents.
            self.clean.clear();
            self.rollback();
        }
        r
    }

    fn commit_inner(&mut self) -> Result<()> {
        // Header bookkeeping (pager_incr_changecounter / pager_write_pagelist).
        let counter = self.header_u32(24)?.wrapping_add(1);
        let npages = self.npages;
        let h = self.get_mut(1)?;
        put32(h, 24, counter);
        put32(h, 28, npages);
        put32(h, 92, counter);
        put32(h, 96, VERSION_NUMBER);

        let size = self.page_size as usize;
        let jpath = journal_path(&self.path);
        let journaled: Vec<u32> = self
            .dirty
            .keys()
            .copied()
            .filter(|&p| p <= self.db_size)
            .collect();
        let new_journal = !jpath.exists();
        let mut j = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&jpath)?;
        let nonce = nonce();
        let mut buf = vec![0u8; SECTOR.max(28)];
        buf[..8].copy_from_slice(&JOURNAL_MAGIC);
        put32(&mut buf, 8, journaled.len() as u32);
        put32(&mut buf, 12, nonce);
        put32(&mut buf, 16, self.db_size);
        put32(&mut buf, 20, SECTOR as u32);
        put32(&mut buf, 24, self.page_size);
        for &pg in &journaled {
            self.load_clean(pg)?;
            let orig = &self.clean[&pg];
            buf.extend_from_slice(&pg.to_be_bytes());
            buf.extend_from_slice(orig);
            buf.extend_from_slice(&cksum(nonce, orig).to_be_bytes());
        }
        j.write_all(&buf)?;
        j.sync_all()?;
        if new_journal {
            sync_dir(&jpath);
        }

        for (i, (&pg, data)) in self.dirty.iter().enumerate() {
            #[cfg(test)]
            if CRASH_AFTER.with(|c| c.get()) == Some(i) {
                return Err(Error::Io(std::io::Error::other("simulated crash")));
            }
            #[cfg(not(test))]
            let _ = i;
            self.file
                .seek(SeekFrom::Start(u64::from(pg - 1) * size as u64))?;
            self.file.write_all(data)?;
        }
        let want = u64::from(self.npages) * size as u64;
        if self.file.metadata()?.len() != want {
            self.file.set_len(want)?;
        }
        self.file.sync_all()?;

        // Commit point.
        match self.journal_mode {
            JournalMode::Truncate => {
                j.set_len(0)?;
                j.sync_all()?;
                drop(j);
            }
            JournalMode::Persist => {
                j.seek(SeekFrom::Start(0))?;
                j.write_all(&[0u8; 28])?;
                j.sync_all()?;
                drop(j);
            }
            JournalMode::Delete => {
                drop(j);
                std::fs::remove_file(&jpath)?;
            }
        }

        self.db_size = self.npages;
        let dirty = std::mem::take(&mut self.dirty);
        self.clean.extend(dirty);
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// Fails a commit after writing this many database pages (tests).
    pub(crate) static CRASH_AFTER: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn write_hot_journal_for_test(path: &Path, pages: &[(u32, Vec<u8>)], orig: u32) {
    let size = pages[0].1.len();
    let nonce = 0x1234_5678;
    let mut buf = vec![0u8; SECTOR];
    buf[..8].copy_from_slice(&JOURNAL_MAGIC);
    put32(&mut buf, 8, pages.len() as u32);
    put32(&mut buf, 12, nonce);
    put32(&mut buf, 16, orig);
    put32(&mut buf, 20, SECTOR as u32);
    put32(&mut buf, 24, size as u32);
    for (pg, d) in pages {
        buf.extend_from_slice(&pg.to_be_bytes());
        buf.extend_from_slice(d);
        buf.extend_from_slice(&cksum(nonce, d).to_be_bytes());
    }
    std::fs::write(journal_path(path), buf).unwrap();
}

#[cfg(test)]
pub(crate) fn read_file(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}
