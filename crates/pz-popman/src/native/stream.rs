//! The library's `DataInputStream` / `DataOutputStream` (big-endian, like
//! Java's; `writeUTF` writes a 16-bit byte length and the raw UTF-8 bytes).

use std::io;
use std::path::Path;

/// Reads a whole file the way the library's `DataInputStream` sees it.
pub struct DataInput {
    data: Vec<u8>,
    pos: usize,
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "EOFException")
}

impl DataInput {
    pub fn open(path: &Path) -> io::Result<DataInput> {
        Ok(DataInput { data: std::fs::read(path)?, pos: 0 })
    }

    pub fn from_bytes(data: Vec<u8>) -> DataInput {
        DataInput { data, pos: 0 }
    }

    fn take<const K: usize>(&mut self) -> io::Result<[u8; K]> {
        let end = self.pos.checked_add(K).ok_or_else(eof)?;
        let s = self.data.get(self.pos..end).ok_or_else(eof)?;
        self.pos = end;
        Ok(s.try_into().unwrap())
    }

    pub fn read_byte(&mut self) -> io::Result<i8> {
        Ok(self.take::<1>()?[0] as i8)
    }

    pub fn read_short(&mut self) -> io::Result<i16> {
        Ok(i16::from_be_bytes(self.take()?))
    }

    pub fn read_int(&mut self) -> io::Result<i32> {
        Ok(i32::from_be_bytes(self.take()?))
    }

    pub fn read_float(&mut self) -> io::Result<f32> {
        Ok(f32::from_bits(self.read_int()? as u32))
    }

    pub fn read_utf(&mut self) -> io::Result<Vec<u8>> {
        let n = self.read_short()?;
        // `std::string::resize` with the sign-extended length.
        let n = n as i64 as usize;
        let end = self.pos.checked_add(n).ok_or_else(eof)?;
        let s = self.data.get(self.pos..end).ok_or_else(eof)?.to_vec();
        self.pos = end;
        Ok(s)
    }
}

/// Collects the bytes `DataOutputStream` would write.
#[derive(Default)]
pub struct DataOutput {
    pub buf: Vec<u8>,
}

impl DataOutput {
    pub fn write_byte(&mut self, v: i32) {
        self.buf.push(v as u8);
    }

    pub fn write_short(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_int(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_float(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_bits().to_be_bytes());
    }

    pub fn write_utf(&mut self, s: &[u8]) {
        self.write_short(s.len() as u16 as i16);
        self.buf.extend_from_slice(s);
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, &self.buf)
    }
}
