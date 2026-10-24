//! Decoding FSB5 Vorbis samples with `lewton`.

use crate::fsb5::{self, Sample};
use crate::vorbis_setup::SetupHeaders;
use lewton::audio::{read_audio_packet_generic, PreviousWindowRight};
use lewton::header::{read_header_ident, read_header_setup, IdentHeader, SetupHeader};
use lewton::samples::InterleavedSamples;
use std::fmt;

/// FMOD always encodes with 256/2048-sample blocks.
const BLOCKSIZE_EXPS: (u8, u8) = (8, 11);

#[derive(Debug)]
pub enum Error {
    NotVorbis,
    MissingSetup(u32),
    Lewton(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotVorbis => write!(f, "sample has no Vorbis data chunk"),
            Error::MissingSetup(c) => write!(f, "no Vorbis setup header with CRC {c:08x}"),
            Error::Lewton(e) => write!(f, "vorbis: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The identification packet FSB5 omits, rebuilt from the sample header.
fn ident_packet(channels: u8, rate: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(30);
    v.extend_from_slice(b"\x01vorbis");
    v.extend_from_slice(&0u32.to_le_bytes());
    v.push(channels);
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&[0; 12]);
    v.push(BLOCKSIZE_EXPS.0 | (BLOCKSIZE_EXPS.1 << 4));
    v.push(1);
    v
}

/// The setup header CRC stored at the start of a sample's Vorbis chunk.
pub fn setup_crc(fsb: &[u8], s: &Sample) -> Option<u32> {
    let c = s.chunk(fsb, fsb5::chunk::VORBIS_DATA)?;
    Some(u32::from_le_bytes(c.get(0..4)?.try_into().ok()?))
}

/// Streaming decoder for one FSB5 Vorbis sample.
pub struct Decoder<'a> {
    ident: IdentHeader,
    setup: SetupHeader,
    pwr: PreviousWindowRight,
    packets: Box<dyn Iterator<Item = &'a [u8]> + 'a>,
    /// Frames still to emit (decoding past the end is trimmed).
    remaining: u64,
    pub channels: u16,
    pub rate: u32,
}

impl<'a> Decoder<'a> {
    pub fn new(fsb: &'a [u8], s: &Sample, setups: &SetupHeaders) -> Result<Decoder<'a>, Error> {
        let crc = setup_crc(fsb, s).ok_or(Error::NotVorbis)?;
        let setup = setups.get(crc).ok_or(Error::MissingSetup(crc))?;
        let channels = s.channels as u8;
        let ident = read_header_ident(&ident_packet(channels, s.frequency))
            .map_err(|e| Error::Lewton(format!("{e:?}")))?;
        let setup = read_header_setup(setup, channels, BLOCKSIZE_EXPS)
            .map_err(|e| Error::Lewton(format!("{e:?}")))?;
        Ok(Decoder {
            ident,
            setup,
            pwr: PreviousWindowRight::new(),
            packets: Box::new(fsb5::vorbis_packets(&fsb[s.data.clone()])),
            remaining: s.frames as u64,
            channels: channels as u16,
            rate: s.frequency,
        })
    }

    /// Next block of interleaved samples, or `None` at the end.
    pub fn next_block(&mut self) -> Result<Option<Vec<f32>>, Error> {
        while self.remaining > 0 {
            let Some(pkt) = self.packets.next() else {
                return Ok(None);
            };
            let out: InterleavedSamples<f32> =
                read_audio_packet_generic(&self.ident, &self.setup, pkt, &mut self.pwr)
                    .map_err(|e| Error::Lewton(format!("{e:?}")))?;
            let mut v = out.samples;
            if v.is_empty() {
                continue;
            }
            let frames = (v.len() / self.channels as usize) as u64;
            if frames > self.remaining {
                v.truncate(self.remaining as usize * self.channels as usize);
            }
            self.remaining -= frames.min(self.remaining);
            return Ok(Some(v));
        }
        Ok(None)
    }

    /// Decode the whole sample to interleaved f32.
    pub fn decode_all(mut self) -> Result<Vec<f32>, Error> {
        let mut out = Vec::with_capacity(self.remaining as usize * self.channels as usize);
        while let Some(b) = self.next_block()? {
            out.extend_from_slice(&b);
        }
        Ok(out)
    }
}
