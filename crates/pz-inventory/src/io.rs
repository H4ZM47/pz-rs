//! Big-endian `ByteBuffer` reading and writing with the helpers the save
//! code shares: `GameWindow.ReadString`/`WriteString`, `BitHeader`,
//! `Bits.packFloatUnitToByte` and `CRC32`.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use pz_entity::runtime_load::{HostResult, RuntimeHostError};

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

/// The actual heap buffer used by native item comparisons. Its backing bytes
/// outlive clear/position changes; replacement creates a different allocation.
#[derive(Debug)]
pub struct NativeSaveBuffer {
    bytes: Vec<u8>,
    position: usize,
}
pub type NativeSaveBufferRef = Rc<RefCell<NativeSaveBuffer>>;

pub fn new_native_save_buffer() -> NativeSaveBufferRef {
    Rc::new(RefCell::new(NativeSaveBuffer {
        bytes: vec![0; 1024],
        position: 0,
    }))
}
pub fn native_buffer_position(buffer: &NativeSaveBufferRef) -> HostResult<usize> {
    buffer
        .try_borrow()
        .map(|b| b.position)
        .map_err(|_| RuntimeHostError::Borrowed("native comparison buffer position"))
}
pub fn native_buffer_capacity(buffer: &NativeSaveBufferRef) -> HostResult<usize> {
    buffer
        .try_borrow()
        .map(|b| b.bytes.len())
        .map_err(|_| RuntimeHostError::Borrowed("native comparison buffer capacity"))
}
pub fn native_buffer_clear(buffer: &NativeSaveBufferRef) -> HostResult<()> {
    buffer
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("native comparison buffer clear"))?
        .position = 0;
    Ok(())
}
pub fn native_buffer_set_position(buffer: &NativeSaveBufferRef, position: usize) -> HostResult<()> {
    let mut buffer = buffer
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("native comparison buffer position write"))?;
    if position > buffer.bytes.len() {
        return Err(RuntimeHostError::InvalidHostState(
            "native comparison position exceeds capacity",
        ));
    }
    buffer.position = position;
    Ok(())
}
pub fn native_buffer_i32_at(buffer: &NativeSaveBufferRef, at: usize) -> HostResult<i32> {
    let bytes = native_buffer_bytes(
        buffer,
        at,
        at.checked_add(4).ok_or(RuntimeHostError::InvalidHostState(
            "native comparison read range",
        ))?,
    )?;
    Ok(i32::from_be_bytes(bytes.try_into().unwrap()))
}
pub fn native_buffer_bytes(
    buffer: &NativeSaveBufferRef,
    start: usize,
    end: usize,
) -> HostResult<Vec<u8>> {
    let buffer = buffer
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("native comparison buffer read"))?;
    buffer
        .bytes
        .get(start..end)
        .map(<[u8]>::to_vec)
        .ok_or(RuntimeHostError::InvalidHostState(
            "native comparison read range",
        ))
}
/// Native ensureCapacity always allocates a NEW buffer. Its request is
/// position+1024 when remaining<1024, otherwise capacity+1024; allocation is
/// rounded up to a 1024-byte multiple. Only bytes before position are copied.
/// The caller performs the subsequent native clear and TLS publication.
pub fn native_buffer_ensure_capacity(
    buffer: &NativeSaveBufferRef,
) -> HostResult<NativeSaveBufferRef> {
    let buffer = buffer
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("native comparison buffer growth"))?;
    let base = if buffer.bytes.len() - buffer.position < 1024 {
        buffer.position
    } else {
        buffer.bytes.len()
    };
    let capacity = base
        .checked_add(1024)
        .and_then(|n| n.checked_add(1023))
        .map(|n| n / 1024 * 1024)
        .filter(|&n| n <= i32::MAX as usize)
        .ok_or(RuntimeHostError::Unsupported {
            operation: "native comparison buffer Java-int allocation overflow",
            kind: None,
        })?;
    let mut bytes = vec![0; capacity];
    bytes[..buffer.position].copy_from_slice(&buffer.bytes[..buffer.position]);
    Ok(Rc::new(RefCell::new(NativeSaveBuffer {
        bytes,
        position: buffer.position,
    })))
}

/// Internal control flow from existing infallible codec primitives. Only an
/// authenticated BufferOverflow is retried by the native save coordinator.
#[derive(Debug, PartialEq, Eq)]
pub enum NativeWriterFault {
    Overflow,
    Host(RuntimeHostError),
}
// Private payload prevents arbitrary codec panics from becoming overflows.
struct NativeWriterControl(NativeWriterFault);
/// ID-finally may recognize our control flow without exposing its payload
/// type or confusing unrelated Rust panics with native/host failures.
pub(crate) fn is_native_writer_control(payload: &(dyn std::any::Any + Send)) -> bool {
    payload.is::<NativeWriterControl>()
}
fn native_writer_fault(fault: NativeWriterFault) -> ! {
    // No panic hook: this is the codec's immediate nonlocal error carrier.
    std::panic::resume_unwind(Box::new(NativeWriterControl(fault)))
}
pub fn catch_native_overflow<T>(f: impl FnOnce() -> T) -> Result<T, NativeWriterFault> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(payload) => match payload.downcast::<NativeWriterControl>() {
            Ok(control) => Err(control.0),
            Err(payload) => std::panic::resume_unwind(payload),
        },
    }
}

/// Legacy writers append to their public Vec. Only the native comparison
/// coordinator may construct a bounded view, whose public Vec remains unused.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
    native_buffer: Option<NativeSaveBufferRef>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Same actual buffer, including when nested calls replace the TLS slot.
    /// Must be used inside the native coordinator's control-flow catch.
    pub(crate) fn for_native_buffer(buffer: &NativeSaveBufferRef) -> Self {
        Self {
            buf: Vec::new(),
            native_buffer: Some(buffer.clone()),
        }
    }
    pub(crate) fn is_native_buffer(&self) -> bool {
        self.native_buffer.is_some()
    }
    pub fn pos(&self) -> usize {
        match &self.native_buffer {
            Some(buffer) => native_buffer_position(buffer)
                .unwrap_or_else(|error| native_writer_fault(NativeWriterFault::Host(error))),
            None => self.buf.len(),
        }
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        let Some(buffer) = &self.native_buffer else {
            self.buf.extend_from_slice(bytes);
            return;
        };
        let mut buffer = buffer.try_borrow_mut().unwrap_or_else(|_| {
            native_writer_fault(NativeWriterFault::Host(RuntimeHostError::Borrowed(
                "native comparison buffer write",
            )))
        });
        if bytes.len() > buffer.bytes.len() - buffer.position {
            drop(buffer);
            native_writer_fault(NativeWriterFault::Overflow);
        }
        let start = buffer.position;
        let end = start + bytes.len();
        buffer.bytes[start..end].copy_from_slice(bytes);
        buffer.position = end;
    }

    pub fn i8(&mut self, v: i8) {
        self.bytes(&[v as u8]);
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

    fn patch_bytes(&mut self, at: usize, bytes: &[u8]) {
        let Some(buffer) = &self.native_buffer else {
            self.buf[at..at + bytes.len()].copy_from_slice(bytes);
            return;
        };
        let previous = native_buffer_position(buffer)
            .unwrap_or_else(|error| native_writer_fault(NativeWriterFault::Host(error)));
        native_buffer_set_position(buffer, at)
            .unwrap_or_else(|error| native_writer_fault(NativeWriterFault::Host(error)));
        // Native BitHeader.write/saveWithSize restore position only after
        // the relative put succeeds. Overflow leaves the cursor at `at`.
        let actual = buffer.clone();
        self.bytes(bytes);
        native_buffer_set_position(&actual, previous)
            .unwrap_or_else(|error| native_writer_fault(NativeWriterFault::Host(error)));
    }

    pub fn patch_i8(&mut self, at: usize, v: i8) {
        self.patch_bytes(at, &[v as u8]);
    }

    pub fn patch_i16(&mut self, at: usize, v: i16) {
        self.patch_bytes(at, &v.to_be_bytes());
    }

    pub fn patch_i32(&mut self, at: usize, v: i32) {
        self.patch_bytes(at, &v.to_be_bytes());
    }

    pub fn patch_i64(&mut self, at: usize, v: i64) {
        self.patch_bytes(at, &v.to_be_bytes());
    }

    /// Legacy Vec writers discard bytes. Actual native buffers only move
    /// position, leaving their previous backing bytes intact.
    pub fn truncate(&mut self, at: usize) {
        match &self.native_buffer {
            Some(buffer) => native_buffer_set_position(buffer, at)
                .unwrap_or_else(|error| native_writer_fault(NativeWriterFault::Host(error))),
            None => self.buf.truncate(at),
        }
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

    #[test]
    fn native_overflow_stops_before_later_work_and_preserves_string_prefix() {
        let buffer = new_native_save_buffer();
        native_buffer_set_position(&buffer, 1020).unwrap();
        let mut writer = Writer::for_native_buffer(&buffer);
        let later = std::cell::Cell::new(false);
        assert_eq!(
            catch_native_overflow(|| {
                writer.string("abcd");
                later.set(true);
            }),
            Err(NativeWriterFault::Overflow)
        );
        assert!(!later.get());
        assert_eq!(native_buffer_position(&buffer).unwrap(), 1022);
        assert_eq!(
            native_buffer_bytes(&buffer, 1020, 1024).unwrap(),
            [0, 4, 0, 0]
        );
        assert!(
            writer.buf.is_empty(),
            "native writes must not bypass capacity via Vec"
        );
        assert!(
            buffer.try_borrow_mut().is_ok(),
            "no buffer guard survives unwind"
        );
    }

    #[test]
    fn native_aliases_clear_and_backpatch_preserve_backing_and_failure_cursor() {
        let buffer = new_native_save_buffer();
        let mut first = Writer::for_native_buffer(&buffer);
        let mut second = Writer::for_native_buffer(&buffer);
        catch_native_overflow(|| {
            first.i32(0);
            first.i16(7);
            second.patch_i32(0, 0x1234_5678);
        })
        .unwrap();
        assert_eq!(first.pos(), 6);
        assert_eq!(second.pos(), 6);
        assert_eq!(native_buffer_i32_at(&buffer, 0).unwrap(), 0x1234_5678);
        native_buffer_clear(&buffer).unwrap();
        assert_eq!(first.pos(), 0);
        assert_eq!(native_buffer_i32_at(&buffer, 0).unwrap(), 0x1234_5678);
        catch_native_overflow(|| {
            first.i8(9);
            second.truncate(0);
        })
        .unwrap();
        assert_eq!(
            native_buffer_bytes(&buffer, 0, 6).unwrap(),
            [9, 0x34, 0x56, 0x78, 0, 7]
        );
        assert_eq!(
            catch_native_overflow(|| second.patch_i32(1022, 12)),
            Err(NativeWriterFault::Overflow)
        );
        assert_eq!(
            first.pos(),
            1022,
            "failed native relative patch must not restore old cursor"
        );
        assert_eq!(native_buffer_bytes(&buffer, 1022, 1024).unwrap(), [0, 0]);
    }

    #[test]
    fn native_growth_replaces_allocation_and_copies_only_current_prefix() {
        let old = new_native_save_buffer();
        let mut local = Writer::for_native_buffer(&old);
        catch_native_overflow(|| local.bytes(&[1, 2, 3, 4, 5])).unwrap();
        native_buffer_set_position(&old, 3).unwrap();
        let grown = native_buffer_ensure_capacity(&old).unwrap();
        assert!(!Rc::ptr_eq(&old, &grown));
        assert_eq!(native_buffer_capacity(&old).unwrap(), 1024);
        assert_eq!(native_buffer_capacity(&grown).unwrap(), 2048);
        assert_eq!(native_buffer_position(&old).unwrap(), 3);
        assert_eq!(native_buffer_position(&grown).unwrap(), 3);
        assert_eq!(native_buffer_bytes(&grown, 0, 5).unwrap(), [1, 2, 3, 0, 0]);
        native_buffer_clear(&grown).unwrap();
        catch_native_overflow(|| {
            local.i8(8);
            local.patch_i16(0, 0x0607);
        })
        .unwrap();
        assert_eq!(native_buffer_bytes(&old, 0, 5).unwrap(), [6, 7, 3, 8, 5]);
        assert_eq!(native_buffer_bytes(&grown, 0, 5).unwrap(), [1, 2, 3, 0, 0]);
        assert_eq!(native_buffer_position(&old).unwrap(), 4);
        assert_eq!(native_buffer_position(&grown).unwrap(), 0);
        // remaining>=1024 grows from capacity, even with an empty cursor.
        let again = native_buffer_ensure_capacity(&grown).unwrap();
        assert_eq!(native_buffer_capacity(&again).unwrap(), 3072);
        assert_eq!(native_buffer_position(&again).unwrap(), 0);
    }

    #[test]
    fn native_buffer_borrow_is_typed_and_unrelated_panics_are_not_overflow() {
        let buffer = new_native_save_buffer();
        let mut writer = Writer::for_native_buffer(&buffer);
        let guard = buffer.borrow_mut();
        assert!(matches!(
            native_buffer_position(&buffer),
            Err(RuntimeHostError::Borrowed(_))
        ));
        assert!(matches!(
            native_buffer_capacity(&buffer),
            Err(RuntimeHostError::Borrowed(_))
        ));
        assert!(matches!(
            native_buffer_clear(&buffer),
            Err(RuntimeHostError::Borrowed(_))
        ));
        assert!(matches!(
            native_buffer_ensure_capacity(&buffer),
            Err(RuntimeHostError::Borrowed(_))
        ));
        let later = std::cell::Cell::new(false);
        assert!(matches!(
            catch_native_overflow(|| {
                writer.i32(3);
                later.set(true);
            }),
            Err(NativeWriterFault::Host(RuntimeHostError::Borrowed(_)))
        ));
        assert!(!later.get());
        let control =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.i32(3))).unwrap_err();
        assert!(is_native_writer_control(control.as_ref()));
        assert!(!is_native_writer_control(&0x1234_u32));
        assert!(matches!(
            catch_native_overflow(|| std::panic::resume_unwind(control)),
            Err(NativeWriterFault::Host(RuntimeHostError::Borrowed(_)))
        ));
        drop(guard);
        assert_eq!(native_buffer_position(&buffer).unwrap(), 0);
        let panic = std::panic::catch_unwind(|| {
            let _ = catch_native_overflow(|| std::panic::resume_unwind(Box::new(0x1234_u32)));
        })
        .unwrap_err();
        assert_eq!(*panic.downcast::<u32>().unwrap(), 0x1234);
        assert!(native_buffer_set_position(&buffer, 1025).is_err());
        assert!(native_buffer_i32_at(&buffer, 1022).is_err());
        assert!(native_buffer_bytes(&buffer, 2, 1).is_err());
    }
}
