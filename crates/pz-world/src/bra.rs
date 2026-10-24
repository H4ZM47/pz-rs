//! `zombie.util.BufferedRandomAccessFile`.
//!
//! The buffer boundaries are observable through `getNextLine`: a line that is
//! wholly inside the buffer is decoded as UTF-8 with a trailing `\r` removed,
//! while a line crossing the buffer end is assembled byte by byte as Latin-1
//! and keeps its `\r`. Lot headers are read through this class, so the port
//! keeps the same buffer size and refill rules.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub struct BufferedRandomAccessFile<R = File> {
    inner: R,
    buffer: Vec<u8>,
    buf_end: usize,
    buf_pos: usize,
    real_pos: u64,
}

impl BufferedRandomAccessFile<File> {
    pub fn open(path: &Path, bufsize: usize) -> io::Result<Self> {
        Self::new(File::open(path)?, bufsize)
    }
}

impl<R: Read + Seek> BufferedRandomAccessFile<R> {
    pub fn new(mut inner: R, bufsize: usize) -> io::Result<Self> {
        let real_pos = inner.stream_position()?;
        Ok(BufferedRandomAccessFile {
            inner,
            buffer: vec![0; bufsize],
            buf_end: 0,
            buf_pos: 0,
            real_pos,
        })
    }

    /// `RandomAccessFile.read(byte[], int, int)` on the underlying file:
    /// -1 at end of file, otherwise the number of bytes read.
    fn super_read(&mut self) -> io::Result<i32> {
        let len = self.buffer.len();
        if len == 0 {
            return Ok(0);
        }
        loop {
            match self.inner.read(&mut self.buffer[..len]) {
                Ok(0) => return Ok(-1),
                Ok(n) => return Ok(n as i32),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    fn fill_buffer(&mut self) -> io::Result<i32> {
        let n = self.super_read()?;
        if n >= 0 {
            self.real_pos += n as u64;
            self.buf_end = n as usize;
            self.buf_pos = 0;
        }
        Ok(n)
    }

    fn invalidate(&mut self) -> io::Result<()> {
        self.buf_end = 0;
        self.buf_pos = 0;
        self.real_pos = self.inner.stream_position()?;
        Ok(())
    }

    /// `read()`: next byte, or -1.
    pub fn read(&mut self) -> io::Result<i32> {
        if self.buf_pos >= self.buf_end && self.fill_buffer()? < 0 {
            return Ok(-1);
        }
        if self.buf_end == 0 {
            return Ok(-1);
        }
        let b = self.buffer[self.buf_pos];
        self.buf_pos += 1;
        Ok(b as i32)
    }

    /// `read(byte[] b, int off, int len)`.
    pub fn read_into(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let leftover = self.buf_end - self.buf_pos;
        if b.len() <= leftover {
            b.copy_from_slice(&self.buffer[self.buf_pos..self.buf_pos + b.len()]);
            self.buf_pos += b.len();
            return Ok(b.len());
        }
        for (i, slot) in b.iter_mut().enumerate() {
            let c = self.read()?;
            if c == -1 {
                return Ok(i);
            }
            *slot = c as u8;
        }
        Ok(b.len())
    }

    pub fn file_pointer(&self) -> u64 {
        self.real_pos - self.buf_end as u64 + self.buf_pos as u64
    }

    pub fn seek(&mut self, pos: u64) -> io::Result<()> {
        // `int n = (int)(this.realPos - pos)`
        let n = (self.real_pos as i64).wrapping_sub(pos as i64) as i32;
        if n >= 0 && n as usize <= self.buf_end {
            self.buf_pos = self.buf_end - n as usize;
        } else {
            self.inner.seek(SeekFrom::Start(pos))?;
            self.invalidate()?;
        }
        Ok(())
    }

    /// `IsoLot.readInt`: little-endian int, `EOFException` if any byte is missing.
    pub fn read_int_le(&mut self) -> io::Result<i32> {
        let ch1 = self.read()?;
        let ch2 = self.read()?;
        let ch3 = self.read()?;
        let ch4 = self.read()?;
        if (ch1 | ch2 | ch3 | ch4) < 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(ch1
            .wrapping_add(ch2 << 8)
            .wrapping_add(ch3 << 16)
            .wrapping_add(ch4 << 24))
    }

    /// `getNextLine()`. `Ok(None)` mirrors the Java `null` return.
    pub fn get_next_line(&mut self) -> io::Result<Option<String>> {
        if self.buf_end <= self.buf_pos && self.fill_buffer()? < 0 {
            return Err(io::Error::other("error in filling buffer!"));
        }
        let lineend = self.buffer[self.buf_pos..self.buf_end]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| i + self.buf_pos);
        let Some(lineend) = lineend else {
            let mut input = String::with_capacity(128);
            let mut c;
            loop {
                c = self.read()?;
                if c == -1 || c == 10 {
                    break;
                }
                input.push(c as u8 as char);
            }
            if c == -1 && input.is_empty() {
                return Ok(None);
            }
            return Ok(Some(input));
        };
        let end = if lineend > 0 && self.buffer[lineend - 1] == 13 {
            lineend - 1
        } else {
            lineend
        };
        // `new String(bytes, UTF_8)`; malformed input becomes U+FFFD.
        let s = String::from_utf8_lossy(&self.buffer[self.buf_pos..end]).into_owned();
        self.buf_pos = lineend + 1;
        Ok(Some(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn line_inside_buffer_strips_cr() {
        let mut f = BufferedRandomAccessFile::new(Cursor::new(b"ab\r\ncd\n".to_vec()), 16).unwrap();
        assert_eq!(f.get_next_line().unwrap().as_deref(), Some("ab"));
        assert_eq!(f.get_next_line().unwrap().as_deref(), Some("cd"));
        assert!(f.get_next_line().is_err());
    }

    #[test]
    fn line_crossing_buffer_keeps_cr() {
        // Buffer of 4: "xyz\r" fills it, the newline is in the next refill.
        let mut f = BufferedRandomAccessFile::new(Cursor::new(b"xyz\r\nq".to_vec()), 4).unwrap();
        assert_eq!(f.get_next_line().unwrap().as_deref(), Some("xyz\r"));
        assert_eq!(f.get_next_line().unwrap().as_deref(), Some("q"));
    }

    #[test]
    fn seek_within_buffer() {
        let mut f =
            BufferedRandomAccessFile::new(Cursor::new((0u8..32).collect::<Vec<_>>()), 8).unwrap();
        assert_eq!(f.read().unwrap(), 0);
        f.seek(5).unwrap();
        assert_eq!(f.read().unwrap(), 5);
        f.seek(20).unwrap();
        assert_eq!(f.read().unwrap(), 20);
        assert_eq!(f.file_pointer(), 21);
        f.seek(0).unwrap();
        assert_eq!(f.read_int_le().unwrap(), 0x03020100);
    }
}
