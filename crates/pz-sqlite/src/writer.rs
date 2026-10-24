//! A minimal SQLite file writer for building test databases (feature
//! `writer`): rowid tables only, built bottom-up into fresh pages. It
//! follows the file-format rules the reader implements (header fields, cell
//! layout, overflow thresholds); the crate's tests check its output with the
//! `sqlite3` CLI when that is installed.

use crate::{TextEncoding, Value};

pub fn varint(v: u64) -> Vec<u8> {
    if v > 0x00ff_ffff_ffff_ffff {
        // Nine bytes: 8 x 7 bits, then 8 bits.
        let mut out = Vec::with_capacity(9);
        let hi = v >> 8;
        for i in (0..8).rev() {
            out.push(0x80 | ((hi >> (7 * i)) & 0x7f) as u8);
        }
        out.push(v as u8);
        return out;
    }
    let mut groups = vec![(v & 0x7f) as u8];
    let mut rest = v >> 7;
    while rest > 0 {
        groups.push(0x80 | (rest & 0x7f) as u8);
        rest >>= 7;
    }
    groups.reverse();
    groups
}

fn int_serial(i: i64) -> (u64, Vec<u8>) {
    let b = i.to_be_bytes();
    match i {
        0 => (8, vec![]),
        1 => (9, vec![]),
        -128..=127 => (1, b[7..].to_vec()),
        -32768..=32767 => (2, b[6..].to_vec()),
        -8_388_608..=8_388_607 => (3, b[5..].to_vec()),
        -2_147_483_648..=2_147_483_647 => (4, b[4..].to_vec()),
        -140_737_488_355_328..=140_737_488_355_327 => (5, b[2..].to_vec()),
        _ => (6, b.to_vec()),
    }
}

pub fn encode_text(s: &str, enc: TextEncoding) -> Vec<u8> {
    match enc {
        TextEncoding::Utf8 => s.as_bytes().to_vec(),
        TextEncoding::Utf16le => s.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        TextEncoding::Utf16be => s.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

pub fn record(values: &[Value], enc: TextEncoding) -> Vec<u8> {
    let mut types = Vec::new();
    let mut body = Vec::new();
    for v in values {
        let (t, b) = match v {
            Value::Null => (0, vec![]),
            Value::Integer(i) => int_serial(*i),
            Value::Real(r) => (7, r.to_bits().to_be_bytes().to_vec()),
            Value::Text(s) => {
                let b = encode_text(s, enc);
                (13 + 2 * b.len() as u64, b)
            }
            Value::Blob(b) => (12 + 2 * b.len() as u64, b.clone()),
        };
        types.extend(varint(t));
        body.extend(b);
    }
    // The header length includes its own varint.
    let mut n = 1;
    while varint((types.len() + n) as u64).len() > n {
        n += 1;
    }
    let mut out = varint((types.len() + n) as u64);
    out.extend(types);
    out.extend(body);
    out
}

pub struct Writer {
    pub page_size: u32,
    pub reserved: u8,
    pub enc: TextEncoding,
    /// Page contents; index 0 is page 1.
    pages: Vec<Vec<u8>>,
    schema: Vec<(i64, Vec<u8>)>,
}

impl Writer {
    pub fn new(page_size: u32, reserved: u8, enc: TextEncoding) -> Self {
        Writer {
            page_size,
            reserved,
            enc,
            pages: vec![vec![0; page_size as usize]],
            schema: Vec::new(),
        }
    }

    fn usable(&self) -> usize {
        (self.page_size - u32::from(self.reserved)) as usize
    }

    fn alloc(&mut self) -> u32 {
        self.pages.push(vec![0; self.page_size as usize]);
        self.pages.len() as u32
    }

    fn page_mut(&mut self, pgno: u32) -> &mut Vec<u8> {
        &mut self.pages[pgno as usize - 1]
    }

    /// A table leaf cell, spilling to overflow pages per the file format.
    fn leaf_cell(&mut self, rowid: i64, payload: &[u8]) -> Vec<u8> {
        let u = self.usable();
        let p = payload.len();
        let x = u - 35;
        let local = if p <= x {
            p
        } else {
            let m = ((u - 12) * 32 / 255) - 23;
            let k = m + ((p - m) % (u - 4));
            if k <= x {
                k
            } else {
                m
            }
        };
        let mut cell = varint(p as u64);
        cell.extend(varint(rowid as u64));
        cell.extend_from_slice(&payload[..local]);
        if local < p {
            let chunks: Vec<&[u8]> = payload[local..].chunks(u - 4).collect();
            let first = self.pages.len() as u32 + 1;
            for (i, chunk) in chunks.iter().enumerate() {
                let pg = self.alloc();
                let next = if i + 1 < chunks.len() { pg + 1 } else { 0 };
                let page = self.page_mut(pg);
                page[..4].copy_from_slice(&next.to_be_bytes());
                page[4..4 + chunk.len()].copy_from_slice(chunk);
            }
            cell.extend(first.to_be_bytes());
        }
        cell
    }

    /// Groups cells into pages whose content fits.
    fn pack(&self, cells: Vec<(i64, Vec<u8>)>, header_len: usize) -> Vec<Vec<(i64, Vec<u8>)>> {
        let mut groups = vec![Vec::new()];
        let mut used = header_len;
        for c in cells {
            let need = c.1.len() + 2;
            if used + need > self.usable() && !groups.last().unwrap().is_empty() {
                groups.push(Vec::new());
                used = header_len;
            }
            used += need;
            groups.last_mut().unwrap().push(c);
        }
        groups
    }

    fn write_node(&mut self, pgno: u32, kind: u8, cells: &[(i64, Vec<u8>)], right: u32) {
        let usable = self.usable();
        let hdr = if pgno == 1 { 100 } else { 0 };
        let hlen = if kind == 0x0D { 8 } else { 12 };
        let page = self.page_mut(pgno);
        let mut top = usable;
        page[hdr] = kind;
        page[hdr + 3..hdr + 5].copy_from_slice(&(cells.len() as u16).to_be_bytes());
        for (i, (_, c)) in cells.iter().enumerate() {
            top -= c.len();
            page[top..top + c.len()].copy_from_slice(c);
            let at = hdr + hlen + 2 * i;
            page[at..at + 2].copy_from_slice(&(top as u16).to_be_bytes());
        }
        assert!(hdr + hlen + 2 * cells.len() <= top, "node overflows page");
        // 65536 is stored as 0 in the content-start field.
        page[hdr + 5..hdr + 7].copy_from_slice(&(top as u16).to_be_bytes());
        if kind == 0x05 {
            page[hdr + 8..hdr + 12].copy_from_slice(&right.to_be_bytes());
        }
    }

    /// Builds a table b-tree from `(rowid, values)` sorted by rowid and
    /// returns its root page (page 1 when `on_page1`).
    fn btree(&mut self, rows: &[(i64, Vec<u8>)], on_page1: bool) -> u32 {
        let cells: Vec<(i64, Vec<u8>)> = rows
            .iter()
            .map(|(id, p)| (*id, self.leaf_cell(*id, p)))
            .collect();
        let mut kind = 0x0D;
        let mut level = self.pack(cells, 8);
        loop {
            let hlen = if kind == 0x0D { 8 } else { 12 };
            if level.len() == 1 {
                let size: usize = level[0].iter().map(|c| c.1.len() + 2).sum::<usize>() + hlen;
                if !on_page1 || size + 100 <= self.usable() {
                    let cells = level.pop().unwrap();
                    let (right, cells) = if kind == 0x05 {
                        let mut cells = cells;
                        let last = cells.pop().unwrap();
                        (u32::from_be_bytes(last.1[..4].try_into().unwrap()), cells)
                    } else {
                        (0, cells)
                    };
                    let pg = if on_page1 { 1 } else { self.alloc() };
                    self.write_node(pg, kind, &cells, right);
                    return pg;
                }
                // Too big for page 1: split in two.
                let mut a = level.pop().unwrap();
                let b = a.split_off(a.len() / 2);
                level = vec![a, b];
            }
            // Write this level and make the parent level's cells.
            let mut parents = Vec::new();
            for group in level {
                let max = group.last().unwrap().0;
                let pg = self.alloc();
                if kind == 0x0D {
                    self.write_node(pg, kind, &group, 0);
                } else {
                    let mut group = group;
                    let last = group.pop().unwrap();
                    let right = u32::from_be_bytes(last.1[..4].try_into().unwrap());
                    self.write_node(pg, kind, &group, right);
                }
                let mut cell = pg.to_be_bytes().to_vec();
                cell.extend(varint(max as u64));
                parents.push((max, cell));
            }
            kind = 0x05;
            level = self.pack(parents, 12);
        }
    }

    /// Adds a table. `rows` must be sorted by rowid.
    pub fn table(&mut self, name: &str, sql: &str, rows: &[(i64, Vec<Value>)]) {
        let enc = self.enc;
        let payloads: Vec<(i64, Vec<u8>)> =
            rows.iter().map(|(id, v)| (*id, record(v, enc))).collect();
        let root = self.btree(&payloads, false);
        let n = self.schema.len() as i64 + 1;
        let entry = vec![
            Value::Text("table".into()),
            Value::Text(name.into()),
            Value::Text(name.into()),
            Value::Integer(i64::from(root)),
            Value::Text(sql.into()),
        ];
        self.schema.push((n, record(&entry, enc)));
    }

    pub fn finish(mut self) -> Vec<u8> {
        let schema = std::mem::take(&mut self.schema);
        self.btree(&schema, true);
        let count = self.pages.len() as u32;
        let enc = match self.enc {
            TextEncoding::Utf8 => 1u32,
            TextEncoding::Utf16le => 2,
            TextEncoding::Utf16be => 3,
        };
        let ps = if self.page_size == 65536 {
            1u16
        } else {
            self.page_size as u16
        };
        let reserved = self.reserved;
        let h = self.page_mut(1);
        h[..16].copy_from_slice(b"SQLite format 3\0");
        h[16..18].copy_from_slice(&ps.to_be_bytes());
        h[18] = 1;
        h[19] = 1;
        h[20] = reserved;
        h[21] = 64;
        h[22] = 32;
        h[23] = 32;
        h[24..28].copy_from_slice(&1u32.to_be_bytes());
        h[28..32].copy_from_slice(&count.to_be_bytes());
        h[40..44].copy_from_slice(&1u32.to_be_bytes());
        h[44..48].copy_from_slice(&4u32.to_be_bytes());
        h[56..60].copy_from_slice(&enc.to_be_bytes());
        h[92..96].copy_from_slice(&1u32.to_be_bytes());
        h[96..100].copy_from_slice(&3_045_000u32.to_be_bytes());
        self.pages.concat()
    }
}
