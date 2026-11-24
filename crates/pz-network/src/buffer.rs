//! `zombie.core.network.ByteBufferWriter` / `ByteBufferReader`: the
//! big-endian Java `ByteBuffer` view game packet bodies are written with.
//!
//! Strings (`putUTF` / `getUTF`, i.e. `GameWindow.StringUTF`) are a signed
//! 16-bit byte count followed by UTF-8; an empty string is just the count 0,
//! and a non-positive count reads back as the empty string.

use std::fmt;

/// A packet body ran out of bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferUnderflow;

impl fmt::Display for BufferUnderflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("packet body ended early (BufferUnderflowException)")
    }
}

impl std::error::Error for BufferUnderflow {}

pub type ReadResult<T> = Result<T, BufferUnderflow>;

#[derive(Clone, Debug, Default)]
pub struct NetWriter {
    pub buf: Vec<u8>,
}

impl NetWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    pub fn put_byte(&mut self, v: i8) -> &mut Self {
        self.buf.push(v as u8);
        self
    }

    pub fn put_bool(&mut self, v: bool) -> &mut Self {
        self.put_byte(v as i8)
    }

    pub fn put_short(&mut self, v: i16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn put_int(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn put_long(&mut self, v: i64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn put_float(&mut self, v: f32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn put_double(&mut self, v: f64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn put_bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    /// `putUTF`. Java truncates the byte count to a `short`; strings longer
    /// than 32767 bytes cannot be read back by the original either.
    pub fn put_utf(&mut self, s: &str) -> &mut Self {
        self.put_short(s.len() as i16);
        self.put_bytes(s.as_bytes())
    }
}

pub struct NetReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> NetReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn get_bytes(&mut self, n: usize) -> ReadResult<&'a [u8]> {
        if self.remaining() < n {
            return Err(BufferUnderflow);
        }
        let r = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(r)
    }

    fn array<const N: usize>(&mut self) -> ReadResult<[u8; N]> {
        let mut a = [0; N];
        a.copy_from_slice(self.get_bytes(N)?);
        Ok(a)
    }

    pub fn get_byte(&mut self) -> ReadResult<i8> {
        Ok(self.array::<1>()?[0] as i8)
    }

    pub fn get_bool(&mut self) -> ReadResult<bool> {
        Ok(self.get_byte()? != 0)
    }

    pub fn get_short(&mut self) -> ReadResult<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }

    pub fn get_int(&mut self) -> ReadResult<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    pub fn get_long(&mut self) -> ReadResult<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }

    pub fn get_float(&mut self) -> ReadResult<f32> {
        Ok(f32::from_be_bytes(self.array()?))
    }

    pub fn get_double(&mut self) -> ReadResult<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }

    /// `getUTF`. Invalid UTF-8 is replaced, as Java's decoder does.
    pub fn get_utf(&mut self) -> ReadResult<String> {
        let n = self.get_short()?;
        if n <= 0 {
            return Ok(String::new());
        }
        Ok(String::from_utf8_lossy(self.get_bytes(n as usize)?).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf_strings() {
        let mut w = NetWriter::new();
        w.put_utf("").put_utf("Zomboid é");
        assert_eq!(&w.buf[..2], &[0, 0]);
        assert_eq!(&w.buf[2..4], &[0, 10]);
        let mut r = NetReader::new(&w.buf);
        assert_eq!(r.get_utf().unwrap(), "");
        assert_eq!(r.get_utf().unwrap(), "Zomboid é");
        assert_eq!(r.get_utf(), Err(BufferUnderflow));
    }

    #[test]
    fn big_endian_numbers() {
        let mut w = NetWriter::new();
        w.put_int(28)
            .put_short(-2)
            .put_long(1)
            .put_float(1.0)
            .put_bool(true);
        assert_eq!(&w.buf[..4], &[0, 0, 0, 28]);
        let mut r = NetReader::new(&w.buf);
        assert_eq!(r.get_int(), Ok(28));
        assert_eq!(r.get_short(), Ok(-2));
        assert_eq!(r.get_long(), Ok(1));
        assert_eq!(r.get_float(), Ok(1.0));
        assert_eq!(r.get_bool(), Ok(true));
        assert_eq!(r.remaining(), 0);
    }
}
