//! Big-endian `ByteBuffer` reading and writing as the entity save code uses
//! it: `GameWindow.ReadString`/`WriteString`, `BitHeader`, `ByteBlock` and
//! `Bits.packFloatUnitToByte`.

use std::fmt;

/// A load failure: the original throws here (or reads past the data).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityError(pub String);

impl fmt::Display for EntityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EntityError {}

pub type Result<T> = std::result::Result<T, EntityError>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(EntityError(msg.into()))
}

/// Reading side of a heap `ByteBuffer`.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// `position(p)`.
    pub fn seek(&mut self, p: usize) -> Result<()> {
        if p > self.buf.len() {
            return err("BufferUnderflowException");
        }
        self.pos = p;
        Ok(())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return err("BufferUnderflowException");
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().unwrap())
    }

    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.arr::<1>()?[0] as i8)
    }

    /// `get() != 0`.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.i8()? != 0)
    }

    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.arr()?))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.arr()?))
    }

    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.arr()?))
    }

    /// `getFloat()`, bits preserved.
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.i32()? as u32))
    }

    /// `getDouble()`, bits preserved.
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.i64()? as u64))
    }

    /// `GameWindow.ReadString`: a short byte count and UTF-8 bytes; a count
    /// `<= 0` reads as `""`. Malformed input is replaced like the original's
    /// decoder (`CodingErrorAction.REPLACE`).
    pub fn string(&mut self) -> Result<String> {
        let n = self.i16()?;
        if n <= 0 {
            return Ok(String::new());
        }
        let n = (n as usize).min(self.remaining());
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }
}

/// Writing side of a growable heap `ByteBuffer`.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Writer { buf: Vec::new() }
    }

    pub fn pos(&self) -> usize {
        self.buf.len()
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    pub fn i8(&mut self, v: i8) {
        self.buf.push(v as u8);
    }

    /// `put(b ? 1 : 0)`.
    pub fn bool(&mut self, v: bool) {
        self.i8(v as i8);
    }

    pub fn i16(&mut self, v: i16) {
        self.bytes(&v.to_be_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.bytes(&v.to_be_bytes());
    }

    pub fn i64(&mut self, v: i64) {
        self.bytes(&v.to_be_bytes());
    }

    pub fn f32(&mut self, v: f32) {
        self.i32(v.to_bits() as i32);
    }

    pub fn f64(&mut self, v: f64) {
        self.i64(v.to_bits() as i64);
    }

    /// `GameWindow.WriteString`: `null` and `""` both write a zero count.
    pub fn string(&mut self, s: &str) {
        let b = s.as_bytes();
        self.i16(b.len() as i16);
        self.bytes(b);
    }

    /// `GameWindow.WriteString` of a nullable string.
    pub fn opt_string(&mut self, s: Option<&str>) {
        self.string(s.unwrap_or(""));
    }

    pub(crate) fn patch_i32(&mut self, at: usize, v: i32) {
        self.buf[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }

    /// `ByteBlock.Start(Save)`: a length placeholder; returns the start.
    pub fn block_start(&mut self) -> usize {
        self.i32(0);
        self.pos()
    }

    /// `ByteBlock.End(Save)`: patches the length.
    pub fn block_end(&mut self, start: usize) {
        let len = (self.pos() - start) as i32;
        self.patch_i32(start - 4, len);
    }
}

/// `BitHeader` sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderSize {
    Byte,
    Short,
    Integer,
}

/// `BitHeader.allocRead`: the header is read where the data starts.
pub fn read_header(r: &mut Reader<'_>, size: HeaderSize) -> Result<i64> {
    Ok(match size {
        HeaderSize::Byte => r.i8()? as i64,
        HeaderSize::Short => r.i16()? as i64,
        HeaderSize::Integer => r.i32()? as i64,
    })
}

/// `BitHeader.allocWrite`: a header placeholder, patched by [`HeaderWrite::write`].
pub struct HeaderWrite {
    start: usize,
    size: HeaderSize,
    flags: i64,
}

impl HeaderWrite {
    pub fn start(w: &mut Writer, size: HeaderSize) -> Self {
        let start = w.pos();
        match size {
            HeaderSize::Byte => w.i8(0),
            HeaderSize::Short => w.i16(0),
            HeaderSize::Integer => w.i32(0),
        }
        HeaderWrite {
            start,
            size,
            flags: 0,
        }
    }

    pub fn add(&mut self, f: i64) {
        self.flags |= f;
    }

    pub fn write(self, w: &mut Writer) {
        let at = self.start;
        match self.size {
            HeaderSize::Byte => w.buf[at] = self.flags as u8,
            HeaderSize::Short => {
                w.buf[at..at + 2].copy_from_slice(&(self.flags as i16).to_be_bytes())
            }
            HeaderSize::Integer => w.patch_i32(at, self.flags as i32),
        }
    }
}

/// `Bits.unpackByteToFloatUnit`.
pub fn unpack_unit(b: i8) -> f32 {
    (b as i32 + 128) as f32 / 255.0
}

/// `Bits.packFloatUnitToByte` (out-of-range values clamp, as outside debug).
pub fn pack_unit(f: f32) -> i8 {
    let f = if !(0.0..=1.0).contains(&f) {
        f.clamp(0.0, 1.0)
    } else {
        f
    };
    (f * 255.0 + -128.0) as i8
}

/// `ByteBlock.Start(Load)`: the block length and the data start.
pub fn block_load(r: &mut Reader<'_>) -> Result<(usize, usize)> {
    let len = r.i32()?;
    let start = r.pos();
    if len < 0 {
        return err("DataBlock possible corruption.");
    }
    Ok((start, len as usize))
}
