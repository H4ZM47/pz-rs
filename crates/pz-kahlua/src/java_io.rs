//! Big-endian `java.nio.ByteBuffer` / `DataInputStream` / `DataOutputStream`
//! primitives and the two string encodings of `zombie.GameWindow`, as the
//! save files use them.
//!
//! They live in the Kahlua crate because `KahluaTableImpl.save`/`load`
//! (see [`crate::table_save`]) is their lowest-level user; the world and
//! game-time save code reuses them.
//!
//! * `GameWindow.WriteString(ByteBuffer, String)` / `ReadString(ByteBuffer)`
//!   (`GameWindow.StringUTF`): a signed 16-bit byte count, then UTF-8.
//!   `null` and `""` both write a count of 0; a count `<= 0` reads as `""`
//!   without consuming anything else. The count is a `(short)` cast, so a
//!   string longer than 32767 bytes writes a wrapped count, like the
//!   original.
//! * `GameWindow.WriteString(DataOutputStream, String)` /
//!   `ReadString(DataInputStream)`: a 32-bit UTF-16 unit count, then
//!   `writeChars` (UTF-16BE). `null` writes 0; 0 reads as `""`; more than
//!   65536 units is an error.
//!
//! Strings are Rust `String`s: on reading, malformed UTF-8 becomes U+FFFD
//! (the original decodes with `CodingErrorAction.REPLACE`) and unpaired
//! UTF-16 surrogates become U+FFFD (the original keeps them in its Java
//! string).

use std::io;

/// A read position over a byte slice with `ByteBuffer`'s big-endian getters.
/// Running past the end is an `UnexpectedEof` error (`BufferUnderflowException`
/// / `EOFException`).
#[derive(Clone, Debug)]
pub struct JavaReader<'a> {
    buf: &'a [u8],
    pos: usize,
    limit: usize,
    notices: Vec<JavaIoNotice>,
}

/// Native diagnostics are returned as owned data so a host can emit them
/// after its item/table guards end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JavaIoNotice {
    StringLengthBeyondRemaining { num_bytes: usize, remaining: usize },
}

impl JavaIoNotice {
    /// Native channel is DebugType.General.error(Object), not an exception
    /// logger or a thrown BufferUnderflowException.
    pub fn native_message(&self) -> String {
        match self {
            Self::StringLengthBeyondRemaining { num_bytes, remaining } => format!(
                "GameWindow.StringUTF.load> numBytes:{num_bytes} is higher than the remaining bytes in the buffer:{remaining}"
            ),
        }
    }
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "BufferUnderflowException")
}

macro_rules! getter {
    ($name:ident, $t:ty, $n:expr) => {
        pub fn $name(&mut self) -> io::Result<$t> {
            let b = self.take($n)?;
            Ok(<$t>::from_be_bytes(b.try_into().expect("length checked")))
        }
    };
}

impl<'a> JavaReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        JavaReader {
            buf,
            pos: 0,
            limit: buf.len(),
            notices: Vec::new(),
        }
    }
    /// The slice must contain the actual initialized buffer capacity, not
    /// merely its current limit. Bytes beyond the limit remain available to
    /// native StringUTF's temporary limit extension.
    pub fn with_limit(buf: &'a [u8], limit: usize) -> io::Result<Self> {
        let mut reader = Self::new(buf);
        reader.set_limit(limit)?;
        Ok(reader)
    }
    pub fn position(&self) -> usize {
        self.pos
    }
    /// `position(p)`; past the limit is an error (`IllegalArgumentException`).
    pub fn set_position(&mut self, pos: usize) -> io::Result<()> {
        if pos > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "IllegalArgumentException: newPosition > limit: ({pos} > {})",
                    self.limit
                ),
            ));
        }
        self.pos = pos;
        Ok(())
    }
    pub fn remaining(&self) -> usize {
        self.limit - self.pos
    }
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }
    pub fn limit(&self) -> usize {
        self.limit
    }
    /// Like ByteBuffer.limit: reducing below position also reduces position.
    /// A rejected limit leaves both fields unchanged.
    pub fn set_limit(&mut self, limit: usize) -> io::Result<()> {
        if limit > self.capacity() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "IllegalArgumentException: newLimit > capacity: ({limit} > {})",
                    self.capacity()
                ),
            ));
        }
        self.limit = limit;
        self.pos = self.pos.min(limit);
        Ok(())
    }
    pub fn take_notices(&mut self) -> Vec<JavaIoNotice> {
        std::mem::take(&mut self.notices)
    }
    pub fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(eof());
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    getter!(get_i8, i8, 1);
    getter!(get_i16, i16, 2);
    getter!(get_u16, u16, 2);
    getter!(get_i32, i32, 4);
    getter!(get_i64, i64, 8);
    getter!(get_f32, f32, 4);
    getter!(get_f64, f64, 8);

    /// `DataInputStream.readBoolean()` (and `ByteBuffer.get() != 0`).
    pub fn get_bool(&mut self) -> io::Result<bool> {
        Ok(self.get_i8()? != 0)
    }

    /// `GameWindow.ReadString(ByteBuffer)`.
    pub fn read_string_utf(&mut self) -> io::Result<String> {
        let n = self.get_i16()?;
        if n <= 0 {
            return Ok(String::new());
        }
        let n = n as usize;
        if self.remaining() < n {
            self.notices
                .push(JavaIoNotice::StringLengthBeyondRemaining {
                    num_bytes: n,
                    remaining: self.remaining(),
                });
        }
        // Native saves the old limit, temporarily sets position+n, copies
        // exactly that range, then restores the old limit. Extension can
        // succeed up to capacity, and restoration can clamp position back.
        let old_limit = self.limit;
        let end = self.pos.checked_add(n).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "IllegalArgumentException: string limit overflow",
            )
        })?;
        self.set_limit(end)?;
        let bytes = self.take(n)?;
        self.set_limit(old_limit)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    /// `GameWindow.ReadString(DataInputStream)`.
    pub fn read_string_chars(&mut self) -> io::Result<String> {
        let len = self.get_i32()?;
        if len == 0 {
            return Ok(String::new());
        }
        if len > 65536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "GameWindow.ReadString: string is too long, corrupted save?",
            ));
        }
        if len < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("NegativeArraySizeException: {len}"),
            ));
        }
        let mut units = Vec::with_capacity(len as usize);
        for _ in 0..len {
            units.push(self.get_u16()?);
        }
        Ok(String::from_utf16_lossy(&units))
    }

    /// `GameWindow.ReadUUID(ByteBuffer)`.
    pub fn read_uuid(&mut self) -> io::Result<JavaUuid> {
        Ok(JavaUuid {
            msb: self.get_i64()?,
            lsb: self.get_i64()?,
        })
    }
}

/// `java.util.UUID` as its two halves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct JavaUuid {
    pub msb: i64,
    pub lsb: i64,
}

impl JavaUuid {
    /// `UUID.randomUUID()` from 16 random bytes (version 4, IETF variant).
    pub fn from_random_bytes(mut b: [u8; 16]) -> JavaUuid {
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        JavaUuid {
            msb: i64::from_be_bytes(b[..8].try_into().unwrap()),
            lsb: i64::from_be_bytes(b[8..].try_into().unwrap()),
        }
    }
}

/// `ByteBuffer.put*` / `DataOutputStream.write*` (both big-endian).
pub trait JavaWrite {
    fn put_i8(&mut self, v: i8);
    fn put_i16(&mut self, v: i16);
    fn put_i32(&mut self, v: i32);
    fn put_i64(&mut self, v: i64);
    fn put_f32(&mut self, v: f32);
    fn put_f64(&mut self, v: f64);
    fn put_bool(&mut self, v: bool) {
        self.put_i8(v as i8);
    }
    /// `GameWindow.WriteString(ByteBuffer, str)`; `None` is `null`.
    fn write_string_utf(&mut self, s: Option<&str>);
    /// `GameWindow.WriteString(DataOutputStream, str)`; `None` is `null`.
    fn write_string_chars(&mut self, s: Option<&str>);
    /// `GameWindow.WriteUUID(ByteBuffer, uuid)`.
    fn write_uuid(&mut self, id: JavaUuid) {
        self.put_i64(id.msb);
        self.put_i64(id.lsb);
    }
}

impl JavaWrite for Vec<u8> {
    fn put_i8(&mut self, v: i8) {
        self.push(v as u8);
    }
    fn put_i16(&mut self, v: i16) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn put_i32(&mut self, v: i32) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn put_i64(&mut self, v: i64) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn put_f32(&mut self, v: f32) {
        self.extend_from_slice(&v.to_bits().to_be_bytes());
    }
    fn put_f64(&mut self, v: f64) {
        self.extend_from_slice(&v.to_bits().to_be_bytes());
    }
    fn write_string_utf(&mut self, s: Option<&str>) {
        match s {
            None | Some("") => self.put_i16(0),
            Some(s) => {
                self.put_i16(s.len() as i16);
                self.extend_from_slice(s.as_bytes());
            }
        }
    }
    fn write_string_chars(&mut self, s: Option<&str>) {
        match s {
            None => self.put_i32(0),
            Some(s) => {
                let units: Vec<u16> = s.encode_utf16().collect();
                self.put_i32(units.len() as i32);
                for u in units {
                    self.extend_from_slice(&u.to_be_bytes());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_encodings() {
        let mut out = Vec::new();
        out.write_string_utf(Some("h\u{e9}"));
        out.write_string_utf(None);
        out.write_string_utf(Some(""));
        out.write_string_chars(Some("h\u{1F600}"));
        out.write_string_chars(None);
        assert_eq!(
            out,
            [
                0, 3, b'h', 0xc3, 0xa9, 0, 0, 0, 0, //
                0, 0, 0, 3, 0, b'h', 0xd8, 0x3d, 0xde, 0x00, //
                0, 0, 0, 0
            ]
        );
        let mut r = JavaReader::new(&out);
        assert_eq!(r.read_string_utf().unwrap(), "h\u{e9}");
        assert_eq!(r.read_string_utf().unwrap(), "");
        assert_eq!(r.read_string_utf().unwrap(), "");
        assert_eq!(r.read_string_chars().unwrap(), "h\u{1F600}");
        assert_eq!(r.read_string_chars().unwrap(), "");
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn negative_utf_length_reads_empty_without_consuming() {
        let mut r = JavaReader::new(&[0xff, 0xff, 7]);
        assert_eq!(r.read_string_utf().unwrap(), "");
        assert_eq!(r.position(), 2);
        let long = "x".repeat(40000);
        let mut out = Vec::new();
        out.write_string_utf(Some(&long));
        assert_eq!(&out[..2], &(40000u16 as i16).to_be_bytes());
    }

    #[test]
    fn malformed_utf8_is_replaced_and_overlong_chars_rejected() {
        let mut r = JavaReader::new(&[0, 2, 0xff, b'a']);
        assert_eq!(r.read_string_utf().unwrap(), "\u{fffd}a");
        let mut r = JavaReader::new(&[0, 1, 0, 1]);
        assert!(r.read_string_chars().is_err());
        let mut r = JavaReader::new(&[0, 5, 0, 1]);
        assert!(r.read_string_utf().is_err());
    }

    #[test]
    fn random_uuid_bits() {
        let u = JavaUuid::from_random_bytes([0xff; 16]);
        assert_eq!((u.msb >> 12) & 0xf, 4);
        assert_eq!((u.lsb as u64) >> 62, 2);
    }

    #[test]
    fn native_utf_can_extend_to_capacity_then_restoring_limit_clamps_position() {
        let bytes = [0, 4, b'a', b'b', b'c', b'd', 99];
        let mut reader = JavaReader::with_limit(&bytes, 4).unwrap();
        assert_eq!(reader.capacity(), 7);
        assert_eq!(reader.read_string_utf().unwrap(), "abcd");
        assert_eq!(reader.limit(), 4);
        assert_eq!(reader.position(), 4);
        assert_eq!(reader.remaining(), 0);
        assert_eq!(
            reader.take_notices(),
            vec![JavaIoNotice::StringLengthBeyondRemaining {
                num_bytes: 4,
                remaining: 2
            }]
        );
        assert!(reader.get_i8().is_err());
        assert_eq!(reader.position(), 4);
    }

    #[test]
    fn rejected_native_utf_limit_keeps_consumed_short_and_original_limit() {
        let mut reader = JavaReader::new(&[0, 5, b'a', b'b']);
        let error = reader.read_string_utf().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().starts_with("IllegalArgumentException:"));
        assert_eq!(reader.position(), 2);
        assert_eq!(reader.limit(), 4);
        assert_eq!(
            reader.take_notices(),
            vec![JavaIoNotice::StringLengthBeyondRemaining {
                num_bytes: 5,
                remaining: 2
            }]
        );
    }

    #[test]
    fn short_count_underflow_does_not_advance_or_invent_a_notice() {
        let mut reader = JavaReader::new(&[0]);
        assert_eq!(
            reader.read_string_utf().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(reader.position(), 0);
        assert!(reader.take_notices().is_empty());
    }
}
