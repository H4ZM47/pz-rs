//! Primitive readers for the serialised data inside Studio bank chunks.
//!
//! Counts are u16 values; when bit 15 is set, the count continues with a
//! second u16 holding bits 15 and up. Arrays of structures are written as
//! `count << 1 | 1` followed by a u16 element size, so newer runtimes can add
//! fields: readers take the fields they know and skip the rest.

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    pub const NULL: Guid = Guid([0; 16]);

    /// Parse `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` (braces optional).
    pub fn parse(s: &str) -> Option<Guid> {
        let h: String = s
            .trim_matches(|c| c == '{' || c == '}')
            .split('-')
            .collect();
        if h.len() != 32 {
            return None;
        }
        let mut raw = [0u8; 16];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = u8::from_str_radix(h.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        // Data1..Data3 are stored little endian.
        raw[0..4].reverse();
        raw[4..6].reverse();
        raw[6..8].reverse();
        Some(Guid(raw))
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        write!(
            f,
            "{{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            u16::from_le_bytes([b[4], b[5]]),
            u16::from_le_bytes([b[6], b[7]]),
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncated;

impl fmt::Display for Truncated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "truncated bank chunk")
    }
}

impl std::error::Error for Truncated {}

pub type Result<T> = std::result::Result<T, Truncated>;

#[derive(Clone)]
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Truncated)?;
        let b = self.data.get(self.pos..end).ok_or(Truncated)?;
        self.pos = end;
        Ok(b)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().unwrap())
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u24(&mut self) -> Result<u32> {
        let b: [u8; 3] = self.array()?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], 0]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub fn guid(&mut self) -> Result<Guid> {
        Ok(Guid(self.array()?))
    }

    pub fn count(&mut self) -> Result<usize> {
        let lo = self.u16()? as usize;
        if lo & 0x8000 == 0 {
            return Ok(lo);
        }
        Ok((lo & 0x7fff) | ((self.u16()? as usize) << 15))
    }

    /// Header of an array of structures: (count, element size).
    pub fn struct_array(&mut self) -> Result<(usize, usize)> {
        let c = self.count()?;
        if c & 1 == 0 {
            // Unsized form: elements are read field by field.
            return Ok((c >> 1, 0));
        }
        Ok((c >> 1, self.u16()? as usize))
    }

    /// Read an array of structures, giving each element its own reader.
    pub fn read_structs<T>(
        &mut self,
        mut f: impl FnMut(&mut Reader<'a>) -> Result<T>,
    ) -> Result<Vec<T>> {
        let (n, size) = self.struct_array()?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            if size == 0 {
                out.push(f(self)?);
            } else {
                let mut r = Reader::new(self.bytes(size)?);
                out.push(f(&mut r)?);
            }
        }
        Ok(out)
    }
}
