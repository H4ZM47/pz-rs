//! Connected traffic: datagrams carrying ACKs, NAKs or message frames.
//!
//! Every connected datagram starts with one header byte whose top bit is set
//! (`DatagramHeaderFormat`). An ACK or NAK datagram then holds a range list
//! of datagram numbers; a data datagram holds a 24-bit datagram number and
//! one or more frames (`InternalPacket` headers followed by their payload).

use super::codec::{DecodeError, DecodeResult, Reader, Writer};
use super::Reliability;

pub const FLAG_VALID: u8 = 0x80;
pub const FLAG_ACK: u8 = 0x40;
/// On an ACK: a 4-byte arrival-rate float follows (`hasBAndAS`).
pub const FLAG_HAS_B_AND_AS: u8 = 0x20;
pub const FLAG_NAK: u8 = 0x20;
pub const FLAG_PACKET_PAIR: u8 = 0x10;
pub const FLAG_CONTINUOUS_SEND: u8 = 0x08;
pub const FLAG_NEEDS_B_AND_AS: u8 = 0x04;

/// Bytes of a data datagram before its first frame.
pub const DATA_HEADER_SIZE: usize = 4;

/// Whether a datagram is connected traffic (as opposed to an offline
/// message).
pub fn is_connected(buf: &[u8]) -> bool {
    buf.first().is_some_and(|b| b & FLAG_VALID != 0)
}

/// An inclusive range of datagram numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub min: u32,
    pub max: u32,
}

/// `DataStructures::RangeList<DatagramSequenceNumberType>::Serialize`.
fn write_ranges(w: &mut Writer, ranges: &[Range]) {
    w.u16_be(ranges.len() as u16);
    for r in ranges {
        let single = r.min == r.max;
        w.bool(single).u24_le(r.min);
        if !single {
            w.u24_le(r.max);
        }
    }
}

fn read_ranges(r: &mut Reader) -> DecodeResult<Vec<Range>> {
    let n = r.u16_be()?;
    let mut out = Vec::with_capacity(usize::from(n).min(r.remaining()));
    for _ in 0..n {
        let single = r.bool()?;
        let min = r.u24_le()?;
        let max = if single { min } else { r.u24_le()? };
        if max < min {
            return Err(DecodeError("ack range max below min"));
        }
        out.push(Range { min, max });
    }
    Ok(out)
}

/// Collapses sorted, de-duplicated datagram numbers into ranges.
pub fn ranges_from_sorted(numbers: &[u32]) -> Vec<Range> {
    let mut out: Vec<Range> = Vec::new();
    for &n in numbers {
        match out.last_mut() {
            Some(r) if r.max.wrapping_add(1) == n => r.max = n,
            _ => out.push(Range { min: n, max: n }),
        }
    }
    out
}

/// Split-packet fields of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub count: u32,
    pub id: u16,
    pub index: u32,
}

/// One message inside a data datagram (`InternalPacket`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub reliability: Reliability,
    pub reliable_index: u32,
    pub sequencing_index: u32,
    pub ordering_index: u32,
    pub ordering_channel: u8,
    pub split: Option<Split>,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(reliability: Reliability, payload: Vec<u8>) -> Self {
        Self {
            reliability,
            reliable_index: 0,
            sequencing_index: 0,
            ordering_index: 0,
            ordering_channel: 0,
            split: None,
            payload,
        }
    }

    /// Header size for a frame with these settings
    /// (`GetMessageHeaderLengthBits` / 8).
    pub fn header_size(reliability: Reliability, split: bool) -> usize {
        let mut n = 1 + 2;
        if reliability.is_reliable() {
            n += 3;
        }
        if reliability.is_sequenced() {
            n += 3;
        }
        if reliability.is_ordered() {
            n += 4;
        }
        if split {
            n += 10;
        }
        n
    }

    pub fn encoded_size(&self) -> usize {
        Self::header_size(self.reliability, self.split.is_some()) + self.payload.len()
    }

    pub fn encode(&self, w: &mut Writer) {
        // ACK-receipt variants travel as their base reliability.
        let wire = match self.reliability {
            Reliability::UnreliableWithAckReceipt => Reliability::Unreliable,
            Reliability::ReliableWithAckReceipt => Reliability::Reliable,
            Reliability::ReliableOrderedWithAckReceipt => Reliability::ReliableOrdered,
            r => r,
        };
        w.u8(((wire as u8) << 5) | if self.split.is_some() { 0x10 } else { 0 });
        w.u16_be((self.payload.len() * 8) as u16);
        if wire.is_reliable() {
            w.u24_le(self.reliable_index);
        }
        if wire.is_sequenced() {
            w.u24_le(self.sequencing_index);
        }
        if wire.is_ordered() {
            w.u24_le(self.ordering_index).u8(self.ordering_channel);
        }
        if let Some(s) = self.split {
            w.u32_be(s.count).u16_be(s.id).u32_be(s.index);
        }
        w.bytes(&self.payload);
    }

    pub fn decode(r: &mut Reader) -> DecodeResult<Self> {
        let flags = r.u8()?;
        let reliability = Reliability::from_bits(flags >> 5);
        let has_split = flags & 0x10 != 0;
        let bits = r.u16_be()?;
        if bits == 0 {
            return Err(DecodeError("empty frame"));
        }
        let mut f = Frame::new(reliability, Vec::new());
        if reliability.is_reliable() {
            f.reliable_index = r.u24_le()?;
        }
        if reliability.is_sequenced() {
            f.sequencing_index = r.u24_le()?;
        }
        if reliability.is_ordered() {
            f.ordering_index = r.u24_le()?;
            f.ordering_channel = r.u8()?;
        }
        if has_split {
            let count = r.u32_be()?;
            let id = r.u16_be()?;
            let index = r.u32_be()?;
            if count == 0 || index >= count {
                return Err(DecodeError("bad split fields"));
            }
            f.split = Some(Split { count, id, index });
        }
        f.payload = r.bytes(usize::from(bits).div_ceil(8))?.to_vec();
        Ok(f)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Datagram {
    Ack {
        ranges: Vec<Range>,
        arrival_rate: Option<f32>,
    },
    Nak {
        ranges: Vec<Range>,
    },
    Data {
        flags: u8,
        number: u32,
        frames: Vec<Frame>,
    },
}

impl Datagram {
    /// A data datagram with the flags RakNet sets on ordinary sends.
    pub fn data(number: u32, frames: Vec<Frame>) -> Self {
        Self::Data {
            flags: FLAG_VALID | FLAG_NEEDS_B_AND_AS,
            number,
            frames,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Ack {
                ranges,
                arrival_rate,
            } => {
                match arrival_rate {
                    Some(rate) => w
                        .u8(FLAG_VALID | FLAG_ACK | FLAG_HAS_B_AND_AS)
                        .f32_be(*rate),
                    None => w.u8(FLAG_VALID | FLAG_ACK),
                };
                write_ranges(&mut w, ranges);
            }
            Self::Nak { ranges } => {
                w.u8(FLAG_VALID | FLAG_NAK);
                write_ranges(&mut w, ranges);
            }
            Self::Data {
                flags,
                number,
                frames,
            } => {
                w.u8(FLAG_VALID | (flags & 0x1c)).u24_le(*number);
                for f in frames {
                    f.encode(&mut w);
                }
            }
        }
        w.into_vec()
    }

    pub fn decode(buf: &[u8]) -> DecodeResult<Self> {
        let mut r = Reader::new(buf);
        let flags = r.u8()?;
        if flags & FLAG_VALID == 0 {
            return Err(DecodeError("not a connected datagram"));
        }
        if flags & FLAG_ACK != 0 {
            let arrival_rate = if flags & FLAG_HAS_B_AND_AS != 0 {
                Some(r.f32_be()?)
            } else {
                None
            };
            return Ok(Self::Ack {
                ranges: read_ranges(&mut r)?,
                arrival_rate,
            });
        }
        if flags & FLAG_NAK != 0 {
            return Ok(Self::Nak {
                ranges: read_ranges(&mut r)?,
            });
        }
        let number = r.u24_le()?;
        let mut frames = Vec::new();
        while r.remaining() > 0 {
            frames.push(Frame::decode(&mut r)?);
        }
        Ok(Self::Data {
            flags,
            number,
            frames,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_ranges_round_trip() {
        let ranges = ranges_from_sorted(&[1, 2, 3, 7, 9, 10]);
        assert_eq!(
            ranges,
            [
                Range { min: 1, max: 3 },
                Range { min: 7, max: 7 },
                Range { min: 9, max: 10 }
            ]
        );
        let d = Datagram::Ack {
            ranges,
            arrival_rate: None,
        };
        let bytes = d.encode();
        assert_eq!(&bytes[..3], &[0xc0, 0, 3]);
        assert_eq!(&bytes[3..10], &[0, 1, 0, 0, 3, 0, 0]);
        assert_eq!(&bytes[10..14], &[1, 7, 0, 0]);
        assert_eq!(Datagram::decode(&bytes), Ok(d));
        let nak = Datagram::Nak {
            ranges: vec![Range { min: 4, max: 4 }],
        };
        assert_eq!(nak.encode()[0], 0xa0);
        assert_eq!(Datagram::decode(&nak.encode()), Ok(nak));
    }

    #[test]
    fn frame_headers_follow_reliability() {
        let mut f = Frame::new(Reliability::ReliableOrdered, b"abc".to_vec());
        f.reliable_index = 0x010203;
        f.ordering_index = 5;
        f.ordering_channel = 3;
        let mut w = Writer::new();
        f.encode(&mut w);
        assert_eq!(w.buf, [0x60, 0, 24, 3, 2, 1, 5, 0, 0, 3, b'a', b'b', b'c']);
        assert_eq!(f.encoded_size(), w.buf.len());

        let mut s = Frame::new(Reliability::UnreliableSequenced, vec![1]);
        s.sequencing_index = 2;
        s.ordering_index = 1;
        s.split = Some(Split {
            count: 2,
            id: 7,
            index: 1,
        });
        let mut w = Writer::new();
        s.encode(&mut w);
        assert_eq!(
            w.buf.len(),
            Frame::header_size(Reliability::UnreliableSequenced, true) + 1
        );
        assert_eq!(Frame::decode(&mut Reader::new(&w.buf)), Ok(s));
    }

    #[test]
    fn ack_receipt_variants_travel_as_base() {
        let f = Frame::new(Reliability::ReliableOrderedWithAckReceipt, vec![9]);
        let mut w = Writer::new();
        f.encode(&mut w);
        assert_eq!(w.buf[0] >> 5, Reliability::ReliableOrdered as u8);
    }

    #[test]
    fn data_datagram_round_trip() {
        let mut a = Frame::new(Reliability::Reliable, vec![0x09, 1, 2]);
        a.reliable_index = 0;
        let b = Frame::new(Reliability::Unreliable, vec![0x00; 9]);
        let d = Datagram::data(0x123456, vec![a, b]);
        let bytes = d.encode();
        assert_eq!(&bytes[..4], &[0x84, 0x56, 0x34, 0x12]);
        assert!(is_connected(&bytes));
        assert_eq!(Datagram::decode(&bytes), Ok(d));
    }

    #[test]
    fn bad_frames_are_rejected() {
        assert!(Datagram::decode(&[0x84, 0, 0, 0, 0x00, 0, 0]).is_err());
        assert!(Datagram::decode(&[0x84, 0, 0, 0, 0x00, 0, 16, 1]).is_err());
    }
}
