//! FSB5 sample banks, as embedded in the `SND ` chunk of FMOD Studio banks.
//!
//! Layout (little endian):
//!
//! ```text
//! 0x00 "FSB5"  0x04 version  0x08 sample count  0x0c sample header bytes
//! 0x10 name table bytes  0x14 sample data bytes  0x18 codec ("mode")
//! 0x1c..0x3c reserved / hash (version 1)
//! sample headers, name table, sample data
//! ```
//!
//! Each sample header is a 64-bit packed word followed by optional chunks:
//! bit 0 = more chunks follow, bits 1..5 = rate index, bits 5..7 = channel
//! index, bits 7..34 = data offset / 32, bits 34..64 = sample frames.

use std::fmt;

/// FMOD_SOUND_FORMAT values used as the FSB5 "mode".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Pcm8,
    Pcm16,
    Pcm24,
    Pcm32,
    PcmFloat,
    GcAdpcm,
    ImaAdpcm,
    Vag,
    HeVag,
    Xma,
    Mpeg,
    Celt,
    At9,
    Xwma,
    Vorbis,
    FAdpcm,
    Opus,
    Other(u32),
}

impl Codec {
    fn from_mode(m: u32) -> Codec {
        match m {
            1 => Codec::Pcm8,
            2 => Codec::Pcm16,
            3 => Codec::Pcm24,
            4 => Codec::Pcm32,
            5 => Codec::PcmFloat,
            6 => Codec::GcAdpcm,
            7 => Codec::ImaAdpcm,
            8 => Codec::Vag,
            9 => Codec::HeVag,
            10 => Codec::Xma,
            11 => Codec::Mpeg,
            12 => Codec::Celt,
            13 => Codec::At9,
            14 => Codec::Xwma,
            15 => Codec::Vorbis,
            16 => Codec::FAdpcm,
            17 => Codec::Opus,
            o => Codec::Other(o),
        }
    }
}

/// Sample header chunk types.
pub mod chunk {
    pub const CHANNELS: u8 = 1;
    pub const FREQUENCY: u8 = 2;
    pub const LOOP: u8 = 3;
    pub const VORBIS_DATA: u8 = 11;
}

#[derive(Debug)]
pub enum Error {
    BadMagic,
    Truncated,
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadMagic => write!(f, "not an FSB5 bank"),
            Error::Truncated => write!(f, "truncated FSB5 data"),
            Error::Unsupported(s) => write!(f, "unsupported FSB5 feature: {s}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug)]
pub struct Sample {
    pub name: Option<String>,
    pub frequency: u32,
    pub channels: u32,
    /// Length in sample frames.
    pub frames: u32,
    /// Byte range of the encoded data, relative to the start of the FSB5.
    pub data: std::ops::Range<usize>,
    /// Loop start/end in frames (inclusive end, as stored).
    pub loop_points: Option<(u32, u32)>,
    /// Raw sample header chunks (type, payload range relative to the FSB5).
    pub chunks: Vec<(u8, std::ops::Range<usize>)>,
}

impl Sample {
    pub fn chunk<'a>(&self, fsb: &'a [u8], ty: u8) -> Option<&'a [u8]> {
        self.chunks
            .iter()
            .find(|(t, _)| *t == ty)
            .map(|(_, r)| &fsb[r.clone()])
    }
}

/// A parsed FSB5 header; sample data stays in the caller's buffer.
#[derive(Clone, Debug)]
pub struct Fsb5 {
    pub version: u32,
    pub codec: Codec,
    pub samples: Vec<Sample>,
}

fn u32_at(b: &[u8], o: usize) -> Result<u32, Error> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .ok_or(Error::Truncated)
}

fn u64_at(b: &[u8], o: usize) -> Result<u64, Error> {
    b.get(o..o + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        .ok_or(Error::Truncated)
}

fn rate_from_index(i: u32) -> u32 {
    match i {
        0 => 4000,
        1 => 8000,
        2 => 11000,
        3 => 11025,
        4 => 16000,
        5 => 22050,
        6 => 24000,
        7 => 32000,
        8 => 44100,
        9 => 48000,
        10 => 96000,
        _ => 44100,
    }
}

impl Fsb5 {
    /// Parse the FSB5 starting at `b[0]`.
    pub fn parse(b: &[u8]) -> Result<Fsb5, Error> {
        if b.get(0..4) != Some(b"FSB5") {
            return Err(Error::BadMagic);
        }
        let version = u32_at(b, 4)?;
        let count = u32_at(b, 8)? as usize;
        let shdr_size = u32_at(b, 12)? as usize;
        let name_size = u32_at(b, 16)? as usize;
        let data_size = u32_at(b, 20)? as usize;
        let codec = Codec::from_mode(u32_at(b, 24)?);
        let header_size = match version {
            0 => 0x40,
            1 => 0x3c,
            _ => return Err(Error::Unsupported("version")),
        };
        let names_at = header_size + shdr_size;
        let data_at = names_at + name_size;
        let data_end = data_at + data_size;
        if b.len() < data_end {
            return Err(Error::Truncated);
        }

        let mut samples = Vec::with_capacity(count);
        let mut p = header_size;
        for _ in 0..count {
            let w = u64_at(b, p)?;
            p += 8;
            let mut frequency = rate_from_index(((w >> 1) & 0xf) as u32);
            let mut channels = [1, 2, 6, 8][((w >> 5) & 3) as usize];
            let offset = (((w >> 7) & 0x7ff_ffff) as usize) * 32;
            let frames = ((w >> 34) & 0x3fff_ffff) as u32;
            let mut chunks = Vec::new();
            let mut loop_points = None;
            let mut more = w & 1 != 0;
            while more {
                let c = u32_at(b, p)?;
                p += 4;
                more = c & 1 != 0;
                let size = ((c >> 1) & 0xff_ffff) as usize;
                let ty = ((c >> 25) & 0x7f) as u8;
                let r = p..p + size;
                if r.end > names_at {
                    return Err(Error::Truncated);
                }
                match ty {
                    chunk::CHANNELS => channels = b[p] as u32,
                    chunk::FREQUENCY => frequency = u32_at(b, p)?,
                    chunk::LOOP => loop_points = Some((u32_at(b, p)?, u32_at(b, p + 4)?)),
                    _ => {}
                }
                chunks.push((ty, r));
                p += size;
            }
            samples.push(Sample {
                name: None,
                frequency,
                channels,
                frames,
                data: data_at + offset..data_at + offset,
                loop_points,
                chunks,
            });
        }

        // Data ranges: each sample runs to the next sample's offset.
        let mut starts: Vec<usize> = samples.iter().map(|s| s.data.start).collect();
        starts.push(data_end);
        starts.sort_unstable();
        for s in &mut samples {
            let i = starts.partition_point(|&x| x <= s.data.start);
            s.data.end = starts.get(i).copied().unwrap_or(data_end);
        }

        if name_size > 0 {
            for (i, s) in samples.iter_mut().enumerate() {
                let off = names_at + u32_at(b, names_at + i * 4)? as usize;
                let end = b[off..data_at]
                    .iter()
                    .position(|&c| c == 0)
                    .map_or(data_at, |n| off + n);
                s.name = Some(String::from_utf8_lossy(&b[off..end]).into_owned());
            }
        }

        Ok(Fsb5 {
            version,
            codec,
            samples,
        })
    }
}

/// Split FSB5 Vorbis sample data into packets: each is a u16 length then bytes.
pub fn vorbis_packets(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut p = 0usize;
    std::iter::from_fn(move || {
        let len = u16::from_le_bytes(data.get(p..p + 2)?.try_into().unwrap()) as usize;
        if len == 0 {
            return None;
        }
        let pkt = data.get(p + 2..p + 2 + len)?;
        p += 2 + len;
        Some(pkt)
    })
}
