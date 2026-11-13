//! Byte-level reading and writing in RakNet's wire conventions.
//!
//! RakNet's `BitStream` writes multi-byte integers in network (big-endian)
//! order, except its 24-bit sequence numbers, which go little-endian. Every
//! field the transport uses is byte aligned, so plain byte buffers suffice.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

/// A malformed or truncated wire message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError(pub &'static str);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RakNet decode error: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

pub type DecodeResult<T> = Result<T, DecodeError>;

/// Family value written in a serialized IPv6 address. RakNet copies the
/// sender platform's `sockaddr_in6` bytes, so the leading family field
/// differs by OS and is ignored on read. IPv6 is not yet checked against the
/// original; the oracle runs have used IPv4 only.
pub const AF_INET6_WIRE: u16 = 23;

/// Cursor over a received message.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let r = &self.buf[self.pos..];
        self.pos = self.buf.len();
        r
    }

    pub fn bytes(&mut self, n: usize) -> DecodeResult<&'a [u8]> {
        if self.remaining() < n {
            return Err(DecodeError("truncated"));
        }
        let r = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(r)
    }

    fn array<const N: usize>(&mut self) -> DecodeResult<[u8; N]> {
        let mut a = [0; N];
        a.copy_from_slice(self.bytes(N)?);
        Ok(a)
    }

    pub fn u8(&mut self) -> DecodeResult<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn bool(&mut self) -> DecodeResult<bool> {
        Ok(self.u8()? != 0)
    }

    pub fn u16_be(&mut self) -> DecodeResult<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub fn u16_le(&mut self) -> DecodeResult<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u24_le(&mut self) -> DecodeResult<u32> {
        let [a, b, c] = self.array()?;
        Ok(u32::from_le_bytes([a, b, c, 0]))
    }

    pub fn u32_be(&mut self) -> DecodeResult<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub fn u64_be(&mut self) -> DecodeResult<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    pub fn f32_be(&mut self) -> DecodeResult<f32> {
        Ok(f32::from_be_bytes(self.array()?))
    }

    /// `SystemAddress::Deserialize`: a version byte, then for IPv4 the four
    /// address bytes inverted and a big-endian port, or for IPv6 a raw
    /// `sockaddr_in6`.
    pub fn address(&mut self) -> DecodeResult<SocketAddr> {
        match self.u8()? {
            4 => {
                let [a, b, c, d] = self.array()?;
                let ip = Ipv4Addr::new(!a, !b, !c, !d);
                Ok(SocketAddr::V4(SocketAddrV4::new(ip, self.u16_be()?)))
            }
            6 => {
                let _family = self.u16_le()?;
                let port = self.u16_be()?;
                let flow = self.u32_be()?;
                let ip = Ipv6Addr::from(self.array::<16>()?);
                let scope = u32::from_le_bytes(self.array()?);
                Ok(SocketAddr::V6(SocketAddrV6::new(ip, port, flow, scope)))
            }
            _ => Err(DecodeError("bad address version")),
        }
    }

    pub fn expect_magic(&mut self) -> DecodeResult<()> {
        if self.bytes(super::OFFLINE_MAGIC.len())? == super::OFFLINE_MAGIC {
            Ok(())
        } else {
            Err(DecodeError("bad offline magic"))
        }
    }
}

/// Builder for an outgoing message.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(v as u8)
    }

    pub fn u16_be(&mut self, v: u16) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    pub fn u16_le(&mut self, v: u16) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    pub fn u24_le(&mut self, v: u32) -> &mut Self {
        debug_assert!(v < 1 << 24);
        self.bytes(&v.to_le_bytes()[..3])
    }

    pub fn u32_be(&mut self, v: u32) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    pub fn u64_be(&mut self, v: u64) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    pub fn f32_be(&mut self, v: f32) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    pub fn magic(&mut self) -> &mut Self {
        self.bytes(&super::OFFLINE_MAGIC)
    }

    /// `SystemAddress::Serialize`; see [`Reader::address`].
    pub fn address(&mut self, addr: &SocketAddr) -> &mut Self {
        match addr {
            SocketAddr::V4(a) => {
                let o = a.ip().octets();
                self.u8(4)
                    .bytes(&[!o[0], !o[1], !o[2], !o[3]])
                    .u16_be(a.port())
            }
            SocketAddr::V6(a) => self
                .u8(6)
                .u16_le(AF_INET6_WIRE)
                .u16_be(a.port())
                .u32_be(a.flowinfo())
                .bytes(&a.ip().octets())
                .bytes(&a.scope_id().to_le_bytes()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u24_is_little_endian() {
        let mut w = Writer::new();
        w.u24_le(0x0a0b0c);
        assert_eq!(w.buf, [0x0c, 0x0b, 0x0a]);
        assert_eq!(Reader::new(&w.buf).u24_le(), Ok(0x0a0b0c));
    }

    #[test]
    fn ipv4_address_is_inverted() {
        let addr: SocketAddr = "127.0.0.1:16261".parse().unwrap();
        let mut w = Writer::new();
        w.address(&addr);
        assert_eq!(w.buf, [4, 0x80, 0xff, 0xff, 0xfe, 0x3f, 0x85]);
        assert_eq!(Reader::new(&w.buf).address(), Ok(addr));
    }

    #[test]
    fn ipv6_address_round_trips() {
        let addr: SocketAddr = "[::1]:16262".parse().unwrap();
        let mut w = Writer::new();
        w.address(&addr);
        assert_eq!(w.buf.len(), 1 + 28);
        assert_eq!(Reader::new(&w.buf).address(), Ok(addr));
    }

    #[test]
    fn truncation_is_an_error() {
        assert_eq!(Reader::new(&[1, 2]).u32_be(), Err(DecodeError("truncated")));
    }
}
