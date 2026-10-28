//! Loose sound files the game plays outside the banks (`media/sound/*.ogg`
//! and `.wav`), decoded whole to interleaved floats.

use crate::studio::Pcm;
use std::io::Cursor;

/// A decoded file and its sample rate.
pub struct SoundFile {
    pub pcm: Pcm,
    pub rate: u32,
}

/// Decode Ogg Vorbis or RIFF WAVE bytes.
pub fn decode(bytes: &[u8]) -> Result<SoundFile, String> {
    if bytes.starts_with(b"OggS") {
        decode_ogg(bytes)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        decode_wav(bytes)
    } else {
        Err("not an Ogg or WAVE file".into())
    }
}

fn decode_ogg(bytes: &[u8]) -> Result<SoundFile, String> {
    let mut r =
        lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let channels = r.ident_hdr.audio_channels as u16;
    let rate = r.ident_hdr.audio_sample_rate;
    let mut data = Vec::new();
    while let Some(p) = r.read_dec_packet_itl().map_err(|e| e.to_string())? {
        data.extend(p.into_iter().map(|s| s as f32 / 32768.0));
    }
    Ok(SoundFile {
        pcm: Pcm { channels, data },
        rate,
    })
}

fn decode_wav(bytes: &[u8]) -> Result<SoundFile, String> {
    let mut chunks = crate::reader::Reader::new(bytes.get(12..).ok_or("truncated WAVE header")?);
    let mut fmt = None;
    while chunks.remaining() != 0 {
        let id = chunks.bytes(4).map_err(|_| "truncated WAVE chunk header")?;
        let size = chunks.u32().map_err(|_| "truncated WAVE chunk header")? as usize;
        // Validate the advertised extent before inspecting any format fields.
        // Reader checks both addition overflow and the actual remaining bytes.
        let body = chunks.bytes(size).map_err(|_| "truncated WAVE chunk")?;
        if id == b"fmt " {
            let mut fields = crate::reader::Reader::new(body);
            let mut format = fields.u16().map_err(|_| "truncated WAVE fmt")?;
            let channels = fields.u16().map_err(|_| "truncated WAVE fmt")?;
            let rate = fields.u32().map_err(|_| "truncated WAVE fmt")?;
            fields.u32().map_err(|_| "truncated WAVE fmt")?; // byte rate
            fields.u16().map_err(|_| "truncated WAVE fmt")?; // block alignment
            let bits = fields.u16().map_err(|_| "truncated WAVE fmt")?;
            if format == 0xFFFE {
                let subformat = body.get(24..26).ok_or("truncated WAVE extensible fmt")?;
                format = u16::from_le_bytes([subformat[0], subformat[1]]);
            }
            if channels == 0 {
                return Err("WAVE has zero channels".into());
            }
            if rate == 0 {
                return Err("WAVE has zero sample rate".into());
            }
            fmt = Some((format, channels, rate, bits));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or("WAVE data before fmt")?;
            let data: Vec<f32> = match (format, bits) {
                (1, 8) => body.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
                (1, 16) => body
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                    .collect(),
                (1, 24) => body
                    .chunks_exact(3)
                    .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8388608.0)
                    .collect(),
                (1, 32) => body
                    .chunks_exact(4)
                    .map(|c| i32::from_le_bytes(c.try_into().unwrap()) as f32 / 2147483648.0)
                    .collect(),
                (3, 32) => body
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
                _ => return Err(format!("unsupported WAVE format {format}/{bits}-bit")),
            };
            return Ok(SoundFile {
                pcm: Pcm { channels, data },
                rate,
            });
        }
        if size & 1 != 0 {
            chunks
                .bytes(1)
                .map_err(|_| "truncated WAVE chunk padding")?;
        }
    }
    Err("WAVE without data".into())
}

#[cfg(test)]
mod tests;
