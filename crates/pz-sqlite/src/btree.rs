//! Table and index b-tree changes ("1.6. B-tree Pages" of the file format).
//!
//! Pages are handled as [`Node`]s: their cells as raw bytes plus the right
//! child. A changed node is written back defragmented (cells packed from
//! the end of the usable area, no freeblocks, no fragmented bytes).
//!
//! Inserting or deleting a cell is followed by a balance pass from the leaf
//! to the root. A node that no longer fits its page, or a non-root node
//! less than a third full, is redistributed together with up to two
//! neighbours (`balance_nonroot`): their cells, plus the dividers between
//! them, are packed into as few pages as hold them, then evened out from
//! the right. An overfull root moves its content to a new child
//! (`balance_deeper`); an interior root left without cells takes over its
//! only child when that fits (`balance_shallower`).

use crate::pager::Pager;
use crate::record::{decode_record, encode_text, put_varint, varint};
use crate::{be16, be32, Error, Result, TextEncoding, Value};
use std::cmp::Ordering;
use std::collections::HashMap;

/// B-tree recursion limit, as in the reader.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Rowid table (intkey) b-tree.
    Table,
    /// Index b-tree.
    Index,
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub leaf: bool,
    /// Raw cells; interior cells start with their 4-byte left child.
    pub cells: Vec<Vec<u8>>,
    /// Right-most child of an interior node.
    pub right: u32,
}

fn corrupt(s: &str) -> Error {
    Error::Corrupt(s.to_string())
}

fn child_of(cell: &[u8]) -> u32 {
    be32(cell, 0)
}

fn with_child(child: u32, rest: &[u8]) -> Vec<u8> {
    let mut c = child.to_be_bytes().to_vec();
    c.extend_from_slice(rest);
    c
}

/// Bytes a cell occupies on its page (cells are at least 4 bytes).
fn cost(c: &[u8]) -> usize {
    2 + c.len().max(4)
}

pub(crate) fn int_float_cmp(i: i64, r: f64) -> Ordering {
    // sqlite3IntFloatCompare
    if r.is_nan() {
        return Ordering::Greater;
    }
    if r < -9_223_372_036_854_775_808.0 {
        return Ordering::Greater;
    }
    if r >= 9_223_372_036_854_775_808.0 {
        return Ordering::Less;
    }
    let y = r as i64;
    match i.cmp(&y) {
        Ordering::Equal => (i as f64).partial_cmp(&r).unwrap_or(Ordering::Equal),
        o => o,
    }
}

/// Value ordering of SQLite with the BINARY collation: NULL, numbers,
/// text (bytes in the database encoding), blobs.
pub(crate) fn cmp_values(a: &Value, b: &Value, enc: TextEncoding) -> Ordering {
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Integer(_) | Value::Real(_) => 1,
        Value::Text(_) => 2,
        Value::Blob(_) => 3,
    };
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Real(x), Value::Real(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Integer(x), Value::Real(y)) => int_float_cmp(*x, *y),
        (Value::Real(x), Value::Integer(y)) => int_float_cmp(*y, *x).reverse(),
        (Value::Text(x), Value::Text(y)) => match enc {
            TextEncoding::Utf8 => x.as_bytes().cmp(y.as_bytes()),
            _ => encode_text(x, enc).cmp(&encode_text(y, enc)),
        },
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

pub(crate) fn cmp_keys(a: &[Value], b: &[Value], enc: TextEncoding) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let o = cmp_values(x, y, enc);
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

pub(crate) struct BTree {
    pub kind: Kind,
    pub usable: usize,
    pub enc: TextEncoding,
}

/// Modified ancestors not yet written (they may not fit their page yet).
type Pending = HashMap<u32, Node>;

/// `(page, child index)` steps from a root down to a node's parent.
type Path = Vec<(u32, usize)>;

impl BTree {
    pub fn new(kind: Kind, pager: &Pager) -> BTree {
        BTree {
            kind,
            usable: pager.usable as usize,
            enc: pager.enc,
        }
    }

    fn hdr_off(pgno: u32) -> usize {
        if pgno == 1 {
            100
        } else {
            0
        }
    }

    fn header_len(leaf: bool) -> usize {
        if leaf {
            8
        } else {
            12
        }
    }

    fn page_type(&self, leaf: bool) -> u8 {
        match (self.kind, leaf) {
            (Kind::Table, true) => 0x0D,
            (Kind::Table, false) => 0x05,
            (Kind::Index, true) => 0x0A,
            (Kind::Index, false) => 0x02,
        }
    }

    /// Bytes of a `p`-byte payload kept on the b-tree page.
    pub fn local_size(&self, p: usize) -> usize {
        let u = self.usable;
        let x = match self.kind {
            Kind::Table => u - 35,
            Kind::Index => ((u - 12) * 64 / 255) - 23,
        };
        if p <= x {
            return p;
        }
        let m = ((u - 12) * 32 / 255) - 23;
        let k = m + ((p - m) % (u - 4));
        if k <= x {
            k
        } else {
            m
        }
    }

    /// Parsed payload layout of a cell: `(payload size, offset of the local
    /// payload, local size, rowid)`. Not for table interior cells.
    fn payload_layout(&self, cell: &[u8], leaf: bool) -> Result<(usize, usize, usize, i64)> {
        let mut pos = if leaf { 0 } else { 4 };
        let (p, n) = varint(cell.get(pos..).unwrap_or(&[])).ok_or_else(|| corrupt("bad cell"))?;
        pos += n;
        let mut rowid = 0;
        if self.kind == Kind::Table {
            let (r, n) =
                varint(cell.get(pos..).unwrap_or(&[])).ok_or_else(|| corrupt("bad cell"))?;
            rowid = r as i64;
            pos += n;
        }
        let p = usize::try_from(p).map_err(|_| corrupt("payload too large"))?;
        Ok((p, pos, self.local_size(p), rowid))
    }

    /// Length of the cell at `off` of `data`.
    fn cell_len(&self, data: &[u8], off: usize, leaf: bool) -> Result<usize> {
        let b = data
            .get(off..self.usable)
            .ok_or_else(|| corrupt("cell offset"))?;
        if self.kind == Kind::Table && !leaf {
            let (_, n) = varint(b.get(4..).unwrap_or(&[])).ok_or_else(|| corrupt("bad cell"))?;
            return Ok(4 + n);
        }
        let (p, pos, local, _) = self.payload_layout(b, leaf)?;
        let len = pos + local + if local < p { 4 } else { 0 };
        if len > b.len() {
            return Err(corrupt("cell overflows page"));
        }
        Ok(len)
    }

    pub fn read_node(&self, pager: &mut Pager, pgno: u32) -> Result<Node> {
        let data = pager.get(pgno)?;
        let hdr = Self::hdr_off(pgno);
        let t = data[hdr];
        let leaf = match (self.kind, t) {
            (Kind::Table, 0x0D) | (Kind::Index, 0x0A) => true,
            (Kind::Table, 0x05) | (Kind::Index, 0x02) => false,
            _ => {
                return Err(Error::Corrupt(format!(
                    "page {pgno}: unexpected b-tree page type {t:#x}"
                )))
            }
        };
        let hlen = Self::header_len(leaf);
        let n = be16(data, hdr + 3) as usize;
        if hdr + hlen + 2 * n > self.usable {
            return Err(Error::Corrupt(format!("page {pgno}: too many cells")));
        }
        let right = if leaf { 0 } else { be32(data, hdr + 8) };
        let mut cells = Vec::with_capacity(n);
        for i in 0..n {
            let off = be16(data, hdr + hlen + 2 * i) as usize;
            if off < hdr + hlen + 2 * n {
                return Err(corrupt("cell pointer out of range"));
            }
            let len = self.cell_len(data, off, leaf)?;
            cells.push(data[off..off + len].to_vec());
        }
        Ok(Node { leaf, cells, right })
    }

    fn used(node: &Node) -> usize {
        Self::header_len(node.leaf) + node.cells.iter().map(|c| cost(c)).sum::<usize>()
    }

    fn fits(&self, pgno: u32, node: &Node) -> bool {
        Self::hdr_off(pgno) + Self::used(node) <= self.usable
    }

    fn underfull(&self, node: &Node) -> bool {
        Self::used(node) * 3 < self.usable
    }

    pub fn write_node(&self, pager: &mut Pager, pgno: u32, node: &Node) -> Result<()> {
        if !self.fits(pgno, node) {
            return Err(corrupt("internal error: node does not fit its page"));
        }
        let usable = self.usable;
        let hdr = Self::hdr_off(pgno);
        let hlen = Self::header_len(node.leaf);
        let t = self.page_type(node.leaf);
        let page = pager.get_mut(pgno)?;
        page[hdr..usable].fill(0);
        page[hdr] = t;
        page[hdr + 3..hdr + 5].copy_from_slice(&(node.cells.len() as u16).to_be_bytes());
        let mut top = usable;
        for (i, c) in node.cells.iter().enumerate() {
            top -= c.len().max(4);
            page[top..top + c.len()].copy_from_slice(c);
            let at = hdr + hlen + 2 * i;
            page[at..at + 2].copy_from_slice(&(top as u16).to_be_bytes());
        }
        // 65536 is stored as 0.
        page[hdr + 5..hdr + 7].copy_from_slice(&(top as u16).to_be_bytes());
        if !node.leaf {
            page[hdr + 8..hdr + 12].copy_from_slice(&node.right.to_be_bytes());
        }
        Ok(())
    }

    /// Writes an empty leaf to `pgno` (a new b-tree root).
    pub fn init_root(&self, pager: &mut Pager, pgno: u32) -> Result<()> {
        let n = Node {
            leaf: true,
            cells: Vec::new(),
            right: 0,
        };
        self.write_node(pager, pgno, &n)
    }

    // ---- payloads ---------------------------------------------------------------

    /// Builds a leaf cell for `payload`, writing the part that does not stay
    /// on the page to newly allocated overflow pages.
    fn make_leaf_cell(&self, pager: &mut Pager, rowid: i64, payload: &[u8]) -> Result<Vec<u8>> {
        let p = payload.len();
        let local = self.local_size(p);
        let mut cell = put_varint(p as u64);
        if self.kind == Kind::Table {
            cell.extend(put_varint(rowid as u64));
        }
        cell.extend_from_slice(&payload[..local]);
        if local < p {
            let chunks: Vec<&[u8]> = payload[local..].chunks(self.usable - 4).collect();
            let mut pages = Vec::with_capacity(chunks.len());
            for _ in &chunks {
                pages.push(pager.allocate()?);
            }
            for (i, chunk) in chunks.iter().enumerate() {
                let next = pages.get(i + 1).copied().unwrap_or(0);
                let pg = pager.get_mut(pages[i])?;
                pg[..4].copy_from_slice(&next.to_be_bytes());
                pg[4..4 + chunk.len()].copy_from_slice(chunk);
            }
            cell.extend(pages[0].to_be_bytes());
        }
        Ok(cell)
    }

    /// Overflow pages of a cell's payload.
    fn overflow_pages(&self, pager: &mut Pager, cell: &[u8], leaf: bool) -> Result<Vec<u32>> {
        let (p, pos, local, _) = self.payload_layout(cell, leaf)?;
        if local == p {
            return Ok(Vec::new());
        }
        let n = (p - local).div_ceil(self.usable - 4);
        let mut out = Vec::with_capacity(n);
        let mut pg = be32(cell, pos + local);
        for _ in 0..n {
            if pg == 0 || pg > pager.npages || out.len() > pager.npages as usize {
                return Err(corrupt("broken overflow chain"));
            }
            out.push(pg);
            pg = be32(pager.get(pg)?, 0);
        }
        Ok(out)
    }

    /// The complete payload of a leaf or index cell.
    pub fn payload(&self, pager: &mut Pager, cell: &[u8], leaf: bool) -> Result<Vec<u8>> {
        let (p, pos, local, _) = self.payload_layout(cell, leaf)?;
        let mut out = Vec::with_capacity(p);
        out.extend_from_slice(
            cell.get(pos..pos + local)
                .ok_or_else(|| corrupt("bad cell"))?,
        );
        for pg in self.overflow_pages(pager, cell, leaf)? {
            let take = (p - out.len()).min(self.usable - 4);
            out.extend_from_slice(&pager.get(pg)?[4..4 + take]);
        }
        Ok(out)
    }

    fn free_overflow(&self, pager: &mut Pager, cell: &[u8], leaf: bool) -> Result<()> {
        for pg in self.overflow_pages(pager, cell, leaf)? {
            pager.free(pg)?;
        }
        Ok(())
    }

    /// Rowid of a table cell (leaf or interior).
    fn rowid(cell: &[u8], leaf: bool) -> Result<i64> {
        let b = if leaf {
            let (_, n) = varint(cell).ok_or_else(|| corrupt("bad cell"))?;
            &cell[n..]
        } else {
            cell.get(4..).unwrap_or(&[])
        };
        Ok(varint(b).ok_or_else(|| corrupt("bad cell"))?.0 as i64)
    }

    fn key(&self, pager: &mut Pager, cell: &[u8], leaf: bool) -> Result<Vec<Value>> {
        let p = self.payload(pager, cell, leaf)?;
        decode_record(&p, self.enc)
    }

    // ---- balancing --------------------------------------------------------------

    fn load(&self, pager: &mut Pager, pgno: u32, pending: &mut Pending) -> Result<Node> {
        match pending.remove(&pgno) {
            Some(n) => Ok(n),
            None => self.read_node(pager, pgno),
        }
    }

    /// Writes `node` to `pgno` and rebalances up to the root. `path` holds
    /// `(page, child index)` from the root down to `pgno`'s parent.
    fn balance(
        &self,
        pager: &mut Pager,
        mut path: Path,
        mut pgno: u32,
        mut node: Node,
        pending: &mut Pending,
    ) -> Result<()> {
        loop {
            match path.pop() {
                None => {
                    if !self.fits(pgno, &node) {
                        // balance_deeper
                        let child = pager.allocate()?;
                        pending.insert(
                            pgno,
                            Node {
                                leaf: false,
                                cells: Vec::new(),
                                right: child,
                            },
                        );
                        path.push((pgno, 0));
                        pgno = child;
                        continue;
                    }
                    if !node.leaf && node.cells.is_empty() {
                        // balance_shallower
                        let child = self.read_node(pager, node.right)?;
                        if self.fits(pgno, &child) {
                            pager.free(node.right)?;
                            node = child;
                        }
                    }
                    return self.write_node(pager, pgno, &node);
                }
                Some((ppg, idx)) => {
                    let mut parent = self.load(pager, ppg, pending)?;
                    if self.fits(pgno, &node) && !self.underfull(&node) {
                        self.write_node(pager, pgno, &node)?;
                    } else {
                        self.redistribute(pager, &mut parent, idx, pgno, node, pending)?;
                    }
                    pgno = ppg;
                    node = parent;
                }
            }
        }
    }

    /// Repacks child `idx` of `parent` (whose new content is `node`) with up
    /// to two neighbours and updates the dividers in `parent`.
    fn redistribute(
        &self,
        pager: &mut Pager,
        parent: &mut Node,
        idx: usize,
        pgno: u32,
        node: Node,
        pending: &mut Pending,
    ) -> Result<()> {
        let nch = parent.cells.len() + 1;
        if idx >= nch {
            return Err(corrupt("child index out of range"));
        }
        let w = nch.min(3);
        let s = if nch <= 3 {
            0
        } else {
            idx.saturating_sub(1).min(nch - 3)
        };
        let child_at = |i: usize| {
            if i < parent.cells.len() {
                child_of(&parent.cells[i])
            } else {
                parent.right
            }
        };
        let mut sibs = Vec::with_capacity(w);
        let mut node = Some(node);
        for i in s..s + w {
            if i == idx {
                sibs.push((pgno, node.take().unwrap()));
            } else {
                let pg = child_at(i);
                sibs.push((pg, self.load(pager, pg, pending)?));
            }
        }
        let leaf = sibs[0].1.leaf;
        if sibs.iter().any(|(_, n)| n.leaf != leaf) {
            return Err(corrupt("siblings at different depths"));
        }
        let table_leaf = self.kind == Kind::Table && leaf;

        // Gather the cells in key order, dividers included.
        let mut cells = Vec::new();
        let mut right_final = 0;
        let old: Vec<u32> = sibs.iter().map(|(p, _)| *p).collect();
        for (j, (_, sib)) in sibs.into_iter().enumerate() {
            let sib_right = sib.right;
            cells.extend(sib.cells);
            if j + 1 < w {
                let div = &parent.cells[s + j];
                match (self.kind, leaf) {
                    (Kind::Table, true) => {}
                    (Kind::Index, true) => cells.push(div[4..].to_vec()),
                    (_, false) => cells.push(with_child(sib_right, &div[4..])),
                }
            } else {
                right_final = sib_right;
            }
        }

        let cap = self.usable - Self::header_len(leaf);
        let (groups, promoted) = partition(cells, table_leaf, cap);
        let k = groups.len();

        let mut pgnos = Vec::with_capacity(k);
        for j in 0..k {
            pgnos.push(match old.get(j) {
                Some(&p) => p,
                None => pager.allocate()?,
            });
        }
        for &extra in old.iter().skip(k) {
            pager.free(extra)?;
        }
        let mut divs = Vec::with_capacity(k - 1);
        for (j, g) in groups.into_iter().enumerate() {
            let right = if leaf {
                0
            } else if j + 1 < k {
                child_of(&promoted[j])
            } else {
                right_final
            };
            if j + 1 < k {
                divs.push(match (self.kind, leaf) {
                    (Kind::Table, true) => {
                        let last = g.last().ok_or_else(|| corrupt("empty page in split"))?;
                        let mut c = pgnos[j].to_be_bytes().to_vec();
                        c.extend(put_varint(Self::rowid(last, true)? as u64));
                        c
                    }
                    (Kind::Index, true) => with_child(pgnos[j], &promoted[j]),
                    (_, false) => with_child(pgnos[j], &promoted[j][4..]),
                });
            }
            let n = Node {
                leaf,
                cells: g,
                right,
            };
            self.write_node(pager, pgnos[j], &n)?;
        }
        let had_last_cell = s + w - 1 < parent.cells.len();
        parent.cells.splice(s..s + w - 1, divs);
        let last = pgnos[k - 1];
        if had_last_cell {
            parent.cells[s + k - 1][..4].copy_from_slice(&last.to_be_bytes());
        } else {
            parent.right = last;
        }
        Ok(())
    }

    // ---- table b-trees ----------------------------------------------------------

    /// Descends to the leaf that holds (or would hold) `rowid`.
    fn table_descend(&self, pager: &mut Pager, root: u32, rowid: i64) -> Result<(Path, u32, Node)> {
        let mut path = Vec::new();
        let mut pg = root;
        loop {
            if path.len() > MAX_DEPTH {
                return Err(corrupt("b-tree too deep"));
            }
            let n = self.read_node(pager, pg)?;
            if n.leaf {
                return Ok((path, pg, n));
            }
            let mut idx = n.cells.len();
            for (i, c) in n.cells.iter().enumerate() {
                if rowid <= Self::rowid(c, false)? {
                    idx = i;
                    break;
                }
            }
            let child = if idx < n.cells.len() {
                child_of(&n.cells[idx])
            } else {
                n.right
            };
            path.push((pg, idx));
            pg = child;
        }
    }

    fn leaf_position(leaf: &Node, rowid: i64) -> Result<std::result::Result<usize, usize>> {
        let mut ids = Vec::with_capacity(leaf.cells.len());
        for c in &leaf.cells {
            ids.push(Self::rowid(c, true)?);
        }
        Ok(ids.binary_search(&rowid))
    }

    /// Inserts a row; `false` (and no change) when `rowid` exists.
    pub fn table_insert(
        &self,
        pager: &mut Pager,
        root: u32,
        rowid: i64,
        payload: &[u8],
    ) -> Result<bool> {
        let (path, pg, mut leaf) = self.table_descend(pager, root, rowid)?;
        let pos = match Self::leaf_position(&leaf, rowid)? {
            Ok(_) => return Ok(false),
            Err(p) => p,
        };
        let cell = self.make_leaf_cell(pager, rowid, payload)?;
        leaf.cells.insert(pos, cell);
        self.balance(pager, path, pg, leaf, &mut Pending::new())?;
        Ok(true)
    }

    /// Deletes a row; `false` when there is none.
    pub fn table_delete(&self, pager: &mut Pager, root: u32, rowid: i64) -> Result<bool> {
        let (path, pg, mut leaf) = self.table_descend(pager, root, rowid)?;
        let Ok(pos) = Self::leaf_position(&leaf, rowid)? else {
            return Ok(false);
        };
        let cell = leaf.cells.remove(pos);
        self.free_overflow(pager, &cell, true)?;
        self.balance(pager, path, pg, leaf, &mut Pending::new())?;
        Ok(true)
    }

    /// The payload of the row with `rowid`.
    pub fn table_get(&self, pager: &mut Pager, root: u32, rowid: i64) -> Result<Option<Vec<u8>>> {
        let (_, _, leaf) = self.table_descend(pager, root, rowid)?;
        match Self::leaf_position(&leaf, rowid)? {
            Ok(i) => Ok(Some(self.payload(pager, &leaf.cells[i], true)?)),
            Err(_) => Ok(None),
        }
    }

    /// The largest rowid in the table.
    pub fn table_max_rowid(&self, pager: &mut Pager, root: u32) -> Result<Option<i64>> {
        let mut pg = root;
        for _ in 0..=MAX_DEPTH {
            let n = self.read_node(pager, pg)?;
            if n.leaf {
                return n.cells.last().map(|c| Self::rowid(c, true)).transpose();
            }
            pg = n.right;
        }
        Err(corrupt("b-tree too deep"))
    }

    /// Visits every row in rowid order: `(rowid, payload)`.
    pub fn table_scan(
        &self,
        pager: &mut Pager,
        root: u32,
        f: &mut dyn FnMut(i64, Vec<u8>) -> Result<()>,
    ) -> Result<()> {
        self.scan_rec(pager, root, 0, f)
    }

    fn scan_rec(
        &self,
        pager: &mut Pager,
        pg: u32,
        depth: usize,
        f: &mut dyn FnMut(i64, Vec<u8>) -> Result<()>,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(corrupt("b-tree too deep"));
        }
        let n = self.read_node(pager, pg)?;
        if n.leaf {
            for c in &n.cells {
                let id = if self.kind == Kind::Table {
                    Self::rowid(c, true)?
                } else {
                    0
                };
                let p = self.payload(pager, c, true)?;
                f(id, p)?;
            }
            return Ok(());
        }
        for c in &n.cells {
            self.scan_rec(pager, child_of(c), depth + 1, f)?;
            if self.kind == Kind::Index {
                let p = self.payload(pager, c, false)?;
                f(0, p)?;
            }
        }
        self.scan_rec(pager, n.right, depth + 1, f)
    }

    // ---- index b-trees ----------------------------------------------------------

    /// Inserts an index entry (a record ending in the rowid). `false` when
    /// an equal entry exists.
    pub fn index_insert(&self, pager: &mut Pager, root: u32, payload: &[u8]) -> Result<bool> {
        let key = decode_record(payload, self.enc)?;
        let mut path = Vec::new();
        let mut pg = root;
        loop {
            if path.len() > MAX_DEPTH {
                return Err(corrupt("b-tree too deep"));
            }
            let mut n = self.read_node(pager, pg)?;
            let mut idx = n.cells.len();
            for (i, c) in n.cells.iter().enumerate() {
                match cmp_keys(&self.key(pager, c, n.leaf)?, &key, self.enc) {
                    Ordering::Equal => return Ok(false),
                    Ordering::Greater => {
                        idx = i;
                        break;
                    }
                    Ordering::Less => {}
                }
            }
            if n.leaf {
                let cell = self.make_leaf_cell(pager, 0, payload)?;
                n.cells.insert(idx, cell);
                self.balance(pager, path, pg, n, &mut Pending::new())?;
                return Ok(true);
            }
            let child = if idx < n.cells.len() {
                child_of(&n.cells[idx])
            } else {
                n.right
            };
            path.push((pg, idx));
            pg = child;
        }
    }

    /// Deletes the index entry equal to `payload`; `false` when missing.
    pub fn index_delete(&self, pager: &mut Pager, root: u32, payload: &[u8]) -> Result<bool> {
        let key = decode_record(payload, self.enc)?;
        let mut path = Vec::new();
        let mut pg = root;
        loop {
            if path.len() > MAX_DEPTH {
                return Err(corrupt("b-tree too deep"));
            }
            let mut n = self.read_node(pager, pg)?;
            let mut idx = n.cells.len();
            let mut hit = false;
            for (i, c) in n.cells.iter().enumerate() {
                match cmp_keys(&self.key(pager, c, n.leaf)?, &key, self.enc) {
                    Ordering::Less => {}
                    o => {
                        idx = i;
                        hit = o == Ordering::Equal;
                        break;
                    }
                }
            }
            if n.leaf {
                if !hit {
                    return Ok(false);
                }
                let cell = n.cells.remove(idx);
                self.free_overflow(pager, &cell, true)?;
                self.balance(pager, path, pg, n, &mut Pending::new())?;
                return Ok(true);
            }
            if !hit {
                let child = if idx < n.cells.len() {
                    child_of(&n.cells[idx])
                } else {
                    n.right
                };
                path.push((pg, idx));
                pg = child;
                continue;
            }
            // The entry is a divider: replace it with its predecessor, the
            // last entry of the left subtree's right-most leaf.
            let old = n.cells[idx].clone();
            self.free_overflow(pager, &old, false)?;
            let left = child_of(&old);
            let mut pending = Pending::new();
            let n_pg = pg;
            path.push((pg, idx));
            let mut cur = left;
            let mut leaf = loop {
                if path.len() > MAX_DEPTH {
                    return Err(corrupt("b-tree too deep"));
                }
                let m = self.read_node(pager, cur)?;
                if m.leaf {
                    break m;
                }
                path.push((cur, m.cells.len()));
                cur = m.right;
            };
            let pred = leaf
                .cells
                .pop()
                .ok_or_else(|| corrupt("empty index leaf"))?;
            n.cells[idx] = with_child(left, &pred);
            pending.insert(n_pg, n);
            self.balance(pager, path, cur, leaf_take(&mut leaf), &mut pending)?;
            return Ok(true);
        }
    }

    /// Frees every page of the b-tree rooted at `root`, the root included
    /// (`DROP TABLE` / `DROP INDEX` without auto-vacuum).
    pub fn free_tree(&self, pager: &mut Pager, root: u32, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(corrupt("b-tree too deep"));
        }
        let n = self.read_node(pager, root)?;
        for c in &n.cells {
            if !n.leaf {
                self.free_tree(pager, child_of(c), depth + 1)?;
            }
            if n.leaf || self.kind == Kind::Index {
                self.free_overflow(pager, c, n.leaf)?;
            }
        }
        if !n.leaf {
            self.free_tree(pager, n.right, depth + 1)?;
        }
        pager.free(root)
    }
}

fn leaf_take(n: &mut Node) -> Node {
    std::mem::replace(
        n,
        Node {
            leaf: true,
            cells: Vec::new(),
            right: 0,
        },
    )
}

/// Splits `cells` into page groups of at most `cap` bytes each. Except for
/// table leaves, the cell between two groups moves up as their divider
/// (`promoted[j]` sits between group `j` and `j + 1`). Groups are filled
/// from the left, then evened out from the right so no group is empty.
#[allow(clippy::type_complexity)]
fn partition(
    cells: Vec<Vec<u8>>,
    table_leaf: bool,
    cap: usize,
) -> (Vec<Vec<Vec<u8>>>, Vec<Vec<u8>>) {
    let mut groups: Vec<Vec<Vec<u8>>> = vec![Vec::new()];
    let mut sizes = vec![0usize];
    let mut promoted = Vec::new();
    for c in cells {
        let last = groups.len() - 1;
        if sizes[last] + cost(&c) > cap && !groups[last].is_empty() {
            if table_leaf {
                sizes.push(cost(&c));
                groups.push(vec![c]);
            } else {
                promoted.push(c);
                sizes.push(0);
                groups.push(Vec::new());
            }
            continue;
        }
        sizes[last] += cost(&c);
        groups[last].push(c);
    }
    for j in (1..groups.len()).rev() {
        loop {
            if groups[j - 1].len() <= 1 {
                break;
            }
            let out = cost(groups[j - 1].last().unwrap());
            let inc = if table_leaf {
                out
            } else {
                cost(&promoted[j - 1])
            };
            let need = groups[j].is_empty();
            if sizes[j] + inc > cap || (!need && sizes[j] + inc > sizes[j - 1] - out) {
                break;
            }
            let c = groups[j - 1].pop().unwrap();
            sizes[j - 1] -= out;
            sizes[j] += inc;
            if table_leaf {
                groups[j].insert(0, c);
            } else {
                let p = std::mem::replace(&mut promoted[j - 1], c);
                groups[j].insert(0, p);
            }
        }
    }
    (groups, promoted)
}
