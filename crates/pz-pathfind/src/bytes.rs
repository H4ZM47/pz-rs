//! Byte-buffer reading/writing with the semantics of the native
//! `ByteBuffer` wrapper (which honours the Java buffer's byte order and
//! throws `BufferUnderflowException` / `BufferOverflowException`).

use std::fmt;

/// Errors the native entry points report by throwing a Java exception.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathfindError {
    /// `java.nio.BufferUnderflowException`.
    BufferUnderflow,
    /// `java.nio.BufferOverflowException`.
    BufferOverflow,
    /// "invalid chunk coordinates" (`NullPointerException`).
    InvalidChunkCoordinates,
    /// "invalid ChunkSquares x,y" (`runtime_error`).
    InvalidChunkSquares,
}

impl fmt::Display for PathfindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathfindError::BufferUnderflow => write!(f, "buffer underflow"),
            PathfindError::BufferOverflow => write!(f, "buffer overflow"),
            PathfindError::InvalidChunkCoordinates => write!(f, "invalid chunk coordinates"),
            PathfindError::InvalidChunkSquares => write!(f, "invalid ChunkSquares x,y"),
        }
    }
}

impl std::error::Error for PathfindError {}

/// Cursor over a byte slice.
pub struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
    big_endian: bool,
}

impl<'a> ByteReader<'a> {
    pub fn big_endian(data: &'a [u8]) -> Self {
        ByteReader {
            data,
            pos: 0,
            big_endian: true,
        }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], PathfindError> {
        if self.data.len() < self.pos + N {
            return Err(PathfindError::BufferUnderflow);
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&self.data[self.pos..self.pos + N]);
        self.pos += N;
        Ok(out)
    }

    pub fn get_u8(&mut self) -> Result<u8, PathfindError> {
        Ok(self.take::<1>()?[0])
    }

    pub fn get_bool(&mut self) -> Result<bool, PathfindError> {
        Ok(self.get_u8()? != 0)
    }

    pub fn get_i16(&mut self) -> Result<i16, PathfindError> {
        let b = self.take::<2>()?;
        Ok(if self.big_endian {
            i16::from_be_bytes(b)
        } else {
            i16::from_le_bytes(b)
        })
    }

    pub fn get_i32(&mut self) -> Result<i32, PathfindError> {
        let b = self.take::<4>()?;
        Ok(if self.big_endian {
            i32::from_be_bytes(b)
        } else {
            i32::from_le_bytes(b)
        })
    }

    pub fn get_f32(&mut self) -> Result<f32, PathfindError> {
        Ok(f32::from_bits(self.get_i32()? as u32))
    }

    /// The 64-bit square flag word as `updateChunk` reads it: the low
    /// 32-bit half is sign-extended before the high bytes are or-ed in, so
    /// a set bit 31 turns on every bit above it. This reproduces that.
    pub fn get_square_bits(&mut self) -> Result<u64, PathfindError> {
        let b = self.take::<8>()?;
        let (lo, hi) = if self.big_endian {
            (
                i32::from_be_bytes([b[4], b[5], b[6], b[7]]),
                [b[3], b[2], b[1], b[0]],
            )
        } else {
            (
                i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                [b[4], b[5], b[6], b[7]],
            )
        };
        let mut v = lo as i64 as u64;
        v |= (hi[0] as u64) << 32;
        v |= (hi[1] as u64) << 40;
        v |= (hi[2] as u64) << 48;
        v |= (hi[3] as u64) << 56;
        Ok(v)
    }
}

/// Growable big-endian writer with an optional capacity limit (the Java
/// `ByteBuffer` the result is written into).
pub struct ByteWriter {
    pub data: Vec<u8>,
    limit: usize,
}

impl ByteWriter {
    pub fn with_limit(limit: usize) -> Self {
        ByteWriter {
            data: Vec::new(),
            limit,
        }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), PathfindError> {
        if self.data.len() + bytes.len() > self.limit {
            return Err(PathfindError::BufferOverflow);
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }

    pub fn put_i16(&mut self, v: i16) -> Result<(), PathfindError> {
        self.put(&v.to_be_bytes())
    }

    pub fn put_i32(&mut self, v: i32) -> Result<(), PathfindError> {
        self.put(&v.to_be_bytes())
    }

    pub fn put_f32(&mut self, v: f32) -> Result<(), PathfindError> {
        self.put(&v.to_bits().to_be_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_bits_sign_extension() {
        let v: u64 = 0x8000_0001;
        let b = v.to_be_bytes();
        let mut r = ByteReader::big_endian(&b);
        assert_eq!(r.get_square_bits().unwrap(), 0xffff_ffff_8000_0001);
        let v: u64 = 0x0000_0001_0000_0200;
        let b = v.to_be_bytes();
        let mut r = ByteReader::big_endian(&b);
        assert_eq!(r.get_square_bits().unwrap(), v);
    }
}
