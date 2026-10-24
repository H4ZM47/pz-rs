//! Minimal reader for `.jar` (zip) archives: the central directory, plus
//! stored and deflated entries.

use std::collections::HashMap;
use std::path::Path;

use crate::{Error, Result};

#[derive(Clone, Copy, Debug)]
struct Entry {
    method: u16,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: u64,
}

/// An opened jar, held in memory.
pub struct Jar {
    data: Vec<u8>,
    entries: HashMap<String, Entry>,
}

fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or(Error::Truncated)
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or(Error::Truncated)
}

const EOCD_SIG: u32 = 0x0605_4b50;
const CEN_SIG: u32 = 0x0201_4b50;
const LOC_SIG: u32 = 0x0403_4b50;

impl Jar {
    pub fn open(path: impl AsRef<Path>) -> Result<Jar> {
        let data = std::fs::read(path).map_err(|e| Error::Io(e.to_string()))?;
        Jar::from_bytes(data)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Jar> {
        // End of central directory: scan back over a possible comment.
        let min = data.len().saturating_sub(22 + 0xFFFF);
        let eocd = (min..=data.len().saturating_sub(22))
            .rev()
            .find(|&i| u32_at(&data, i).ok() == Some(EOCD_SIG))
            .ok_or(Error::NotAZip)?;
        let count = u16_at(&data, eocd + 10)? as usize;
        let mut off = u32_at(&data, eocd + 16)? as usize;
        let mut entries = HashMap::with_capacity(count);
        for _ in 0..count {
            if u32_at(&data, off)? != CEN_SIG {
                return Err(Error::NotAZip);
            }
            let method = u16_at(&data, off + 10)?;
            let compressed_size = u32_at(&data, off + 20)? as u64;
            let uncompressed_size = u32_at(&data, off + 24)? as u64;
            let name_len = u16_at(&data, off + 28)? as usize;
            let extra_len = u16_at(&data, off + 30)? as usize;
            let comment_len = u16_at(&data, off + 32)? as usize;
            let local_header_offset = u32_at(&data, off + 42)? as u64;
            let name = data.get(off + 46..off + 46 + name_len).ok_or(Error::Truncated)?;
            let name = String::from_utf8_lossy(name).into_owned();
            entries.insert(name, Entry { method, compressed_size, uncompressed_size, local_header_offset });
            off += 46 + name_len + extra_len + comment_len;
        }
        Ok(Jar { data, entries })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The uncompressed bytes of an entry such as `a/b/C.class`.
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let e = *self.entries.get(name).ok_or_else(|| Error::Missing(name.to_string()))?;
        let lh = e.local_header_offset as usize;
        if u32_at(&self.data, lh)? != LOC_SIG {
            return Err(Error::NotAZip);
        }
        let start = lh + 30 + u16_at(&self.data, lh + 26)? as usize + u16_at(&self.data, lh + 28)? as usize;
        let raw = self.data.get(start..start + e.compressed_size as usize).ok_or(Error::Truncated)?;
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => {
                let out = miniz_oxide::inflate::decompress_to_vec(raw).map_err(|_| Error::Inflate)?;
                if out.len() as u64 != e.uncompressed_size {
                    return Err(Error::Inflate);
                }
                Ok(out)
            }
            m => Err(Error::Unsupported(format!("zip method {m}"))),
        }
    }
}
