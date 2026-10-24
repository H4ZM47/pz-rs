//! Big-endian `ByteBuffer` reading and writing with the helpers the save
//! code shares: `GameWindow.ReadString`/`WriteString`, `BitHeader`,
//! `Bits.packFloatUnitToByte` and `CRC32`.

use std::fmt;

/// A load or save failure: the original throws (or would read garbage).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveError(pub String);

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SaveError {}

pub type SaveResult<T> = Result<T, SaveError>;

pub fn err<T>(msg: impl Into<String>) -> SaveResult<T> {
    Err(SaveError(msg.into()))
}

/// Reading side of a heap `ByteBuffer` (big-endian).
pub struct Reader<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn bytes(&mut self, n: usize) -> SaveResult<&'a [u8]> {
        if self.remaining() < n {
            return err(format!(
                "BufferUnderflowException at {} (need {n}, have {})",
                self.pos,
                self.remaining()
            ));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn arr<const N: usize>(&mut self) -> SaveResult<[u8; N]> {
        Ok(self.bytes(N)?.try_into().unwrap())
    }

    pub fn i8(&mut self) -> SaveResult<i8> {
        Ok(self.arr::<1>()?[0] as i8)
    }

    /// `get() != 0`.
    pub fn bool(&mut self) -> SaveResult<bool> {
        Ok(self.i8()? != 0)
    }

    pub fn i16(&mut self) -> SaveResult<i16> {
        Ok(i16::from_be_bytes(self.arr()?))
    }

    pub fn i32(&mut self) -> SaveResult<i32> {
        Ok(i32::from_be_bytes(self.arr()?))
    }

    pub fn i64(&mut self) -> SaveResult<i64> {
        Ok(i64::from_be_bytes(self.arr()?))
    }

    /// `getFloat()`, bits preserved.
    pub fn f32(&mut self) -> SaveResult<f32> {
        Ok(f32::from_bits(self.i32()? as u32))
    }

    pub fn f64(&mut self) -> SaveResult<f64> {
        Ok(f64::from_bits(self.i64()? as u64))
    }

    /// `GameWindow.ReadString`: a short byte count and UTF-8 bytes; a count
    /// `<= 0` reads as `""`. Malformed input is replaced like the original
    /// decoder (`CodingErrorAction.REPLACE`).
    pub fn string(&mut self) -> SaveResult<String> {
        let n = self.i16()?;
        if n <= 0 {
            return Ok(String::new());
        }
        let n = (n as usize).min(self.remaining());
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }

    /// `Bits.unpackByteToFloatUnit(get())`.
    pub fn unit_byte(&mut self) -> SaveResult<f32> {
        Ok(unpack_unit(self.i8()?))
    }
}

/// Writing side of a growable heap `ByteBuffer` (big-endian).
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
        self.buf.push(v as u8);
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
        if s.is_empty() {
            self.i16(0);
            return;
        }
        self.i16(s.len() as i16);
        self.bytes(s.as_bytes());
    }

    pub fn opt_string(&mut self, s: Option<&str>) {
        self.string(s.unwrap_or(""));
    }

    /// `put(Bits.packFloatUnitToByte(f))`.
    pub fn unit_byte(&mut self, f: f32) {
        self.i8(pack_unit(f));
    }

    pub fn patch_i8(&mut self, at: usize, v: i8) {
        self.buf[at] = v as u8;
    }

    pub fn patch_i16(&mut self, at: usize, v: i16) {
        self.buf[at..at + 2].copy_from_slice(&v.to_be_bytes());
    }

    pub fn patch_i32(&mut self, at: usize, v: i32) {
        self.buf[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }

    pub fn patch_i64(&mut self, at: usize, v: i64) {
        self.buf[at..at + 8].copy_from_slice(&v.to_be_bytes());
    }

    /// Drops everything from `at` on (`position(at)` before more writes).
    pub fn truncate(&mut self, at: usize) {
        self.buf.truncate(at);
    }
}

/// Java `(int) f` for a float: truncation toward zero, saturating, NaN 0.
pub fn f2i(f: f32) -> i32 {
    f as i32
}

/// `Bits.packFloatUnitToByte` (clamped outside `[0, 1]` as in a release
/// build).
pub fn pack_unit(f: f32) -> i8 {
    // PZMath.clamp(f, 0, 1) outside the range; NaN passes through and
    // casts to 0.
    let f = if f.is_nan() { f } else { f.clamp(0.0, 1.0) };
    f2i(f * 255.0 + -128.0) as i8
}

/// `Bits.unpackByteToFloatUnit`.
pub fn unpack_unit(b: i8) -> f32 {
    (b as i32 - -128) as f32 / 255.0
}

/// `ImmutableColor(int r, int g, int b)` component and its `get*Byte()`.
pub fn color_from_byte(b: i8) -> f32 {
    (b as u8) as f32 / 255.0
}

pub fn color_to_byte(c: f32) -> i8 {
    (f2i(c * 255.0) & 0xFF) as u8 as i8
}

/// A `BitHeader` being written: a placeholder of its size is written at
/// allocation and the flags are patched in by `write()`.
pub struct BitHeaderWrite {
    start: usize,
    size: usize,
    pub flags: u64,
}

impl BitHeaderWrite {
    pub fn alloc(w: &mut Writer, size: usize) -> Self {
        let start = w.pos();
        w.bytes(&[0u8; 8][..size]);
        BitHeaderWrite {
            start,
            size,
            flags: 0,
        }
    }

    pub fn add(&mut self, f: u64) {
        self.flags |= f;
    }

    pub fn write(&self, w: &mut Writer) {
        match self.size {
            1 => w.patch_i8(self.start, self.flags as i8),
            2 => w.patch_i16(self.start, self.flags as i16),
            4 => w.patch_i32(self.start, self.flags as i32),
            _ => w.patch_i64(self.start, self.flags as i64),
        }
    }

    /// `output.position(bits.getStartPosition())` for an empty header whose
    /// placeholder is the last thing written.
    pub fn discard(&self, w: &mut Writer) {
        w.truncate(self.start);
    }

    pub fn start(&self) -> usize {
        self.start
    }
}

/// `BitHeaderRead` flags (sign-extended like the Java field).
pub fn read_header(r: &mut Reader<'_>, size: usize) -> SaveResult<u64> {
    Ok(match size {
        1 => r.i8()? as i64 as u64,
        2 => r.i16()? as i64 as u64,
        4 => r.i32()? as i64 as u64,
        _ => r.i64()? as u64,
    })
}

/// `Bits.hasFlags`: all of `f` set.
pub fn has(flags: u64, f: u64) -> bool {
    flags & f == f
}

/// `java.util.zip.CRC32` over `data`.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *e = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_round_trip() {
        let mut w = Writer::new();
        w.i8(-3);
        w.i16(-2);
        w.i32(0x1234_5678);
        w.i64(-1);
        w.f32(f32::from_bits(0x7fc0_0001));
        w.string("héllo");
        w.string("");
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.i8().unwrap(), -3);
        assert_eq!(r.i16().unwrap(), -2);
        assert_eq!(r.i32().unwrap(), 0x1234_5678);
        assert_eq!(r.i64().unwrap(), -1);
        assert_eq!(r.f32().unwrap().to_bits(), 0x7fc0_0001);
        assert_eq!(r.string().unwrap(), "héllo");
        assert_eq!(r.string().unwrap(), "");
        assert!(r.i8().is_err());
    }

    #[test]
    fn unit_and_color_bytes() {
        // Every byte survives unpack/pack (checked: the float math is exact).
        let lossy = (-128..=127)
            .filter(|&b| pack_unit(unpack_unit(b as i8)) != b as i8)
            .count();
        assert_eq!(lossy, 0);
        assert_eq!(pack_unit(1.0), 127);
        assert_eq!(pack_unit(0.0), -128);
        assert_eq!(pack_unit(2.0), 127);
        assert_eq!(color_to_byte(1.0), -1);
        assert_eq!(color_to_byte(color_from_byte(0)), 0);
    }

    #[test]
    fn crc_and_headers() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let mut w = Writer::new();
        let mut h = BitHeaderWrite::alloc(&mut w, 2);
        w.i8(9);
        h.add(0x8001);
        h.write(&mut w);
        assert_eq!(w.buf, vec![0x80, 0x01, 9]);
        let mut r = Reader::new(&w.buf);
        let f = read_header(&mut r, 2).unwrap();
        assert!(has(f, 0x8000) && has(f, 1) && !has(f, 2));
    }
}
