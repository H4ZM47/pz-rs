//! RIFF container of FMOD Studio banks (`RIFF` ... `FEV `).
//!
//! Banks are a tree of chunks: `LIST` chunks carry a four-character type and
//! nest further chunks; other chunks are leaves. All sizes are little endian
//! and chunks are padded to even sizes.

use std::ops::Range;

#[derive(Clone, Debug)]
pub struct Chunk {
    pub id: [u8; 4],
    /// `LIST` type, when `id == "LIST"`.
    pub list_type: Option<[u8; 4]>,
    /// Payload, as absolute offsets into the file (after any list type).
    pub body: Range<usize>,
    pub children: Vec<Chunk>,
}

impl Chunk {
    pub fn id_str(&self) -> String {
        String::from_utf8_lossy(self.list_type.as_ref().unwrap_or(&self.id)).into_owned()
    }

    /// Depth-first search for the first chunk (or list) with this id.
    pub fn find(&self, id: &[u8; 4]) -> Option<&Chunk> {
        if &self.id == id || self.list_type.as_ref() == Some(id) {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }
}

fn read_chunks(b: &[u8], mut p: usize, end: usize, out: &mut Vec<Chunk>) -> Option<()> {
    while p + 8 <= end {
        let id: [u8; 4] = b[p..p + 4].try_into().ok()?;
        let size = u32::from_le_bytes(b[p + 4..p + 8].try_into().ok()?) as usize;
        let body_end = (p + 8).checked_add(size)?.min(end);
        let mut c = Chunk {
            id,
            list_type: None,
            body: p + 8..body_end,
            children: Vec::new(),
        };
        if &id == b"LIST" && size >= 4 {
            c.list_type = Some(b[p + 8..p + 12].try_into().ok()?);
            c.body.start += 4;
            read_chunks(b, c.body.start, body_end, &mut c.children)?;
        }
        out.push(c);
        p = body_end + (size & 1);
    }
    Some(())
}

/// Parse the whole chunk tree of a bank file.
pub fn parse(b: &[u8]) -> Option<Chunk> {
    if b.get(0..4)? != b"RIFF" || b.get(8..12)? != b"FEV " {
        return None;
    }
    let size = u32::from_le_bytes(b[4..8].try_into().ok()?) as usize;
    let end = (8 + size).min(b.len());
    let mut root = Chunk {
        id: *b"RIFF",
        list_type: Some(*b"FEV "),
        body: 12..end,
        children: vec![],
    };
    read_chunks(b, 12, end, &mut root.children)?;
    Some(root)
}

/// Offset of the FSB5 inside the `SND ` chunk: the payload starts at the
/// next 32-byte-aligned file offset.
pub fn fsb5_range(b: &[u8], root: &Chunk) -> Option<Range<usize>> {
    let snd = root.find(b"SND ")?;
    let start = (snd.body.start + 31) & !31;
    (start < snd.body.end && b.get(start..start + 4)? == b"FSB5").then_some(start..snd.body.end)
}
