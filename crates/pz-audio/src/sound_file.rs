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
    let u16le = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let u32le = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let mut fmt = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32le(pos + 4) as usize;
        let body = pos + 8;
        let end = (body + size).min(bytes.len());
        if id == b"fmt " && size >= 16 {
            // (format, channels, rate, bits)
            let mut format = u16le(body);
            if format == 0xFFFE && size >= 26 {
                format = u16le(body + 24);
            }
            fmt = Some((format, u16le(body + 2), u32le(body + 4), u16le(body + 14)));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or("WAVE data before fmt")?;
            let d = &bytes[body..end];
            let data: Vec<f32> = match (format, bits) {
                (1, 8) => d.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
                (1, 16) => d
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                    .collect(),
                (1, 24) => d
                    .chunks_exact(3)
                    .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8388608.0)
                    .collect(),
                (1, 32) => d
                    .chunks_exact(4)
                    .map(|c| i32::from_le_bytes(c.try_into().unwrap()) as f32 / 2147483648.0)
                    .collect(),
                (3, 32) => d
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
                _ => return Err(format!("unsupported WAVE format {format}/{bits}-bit")),
            };
            return Ok(SoundFile {
                pcm: Pcm {
                    channels: channels.max(1),
                    data,
                },
                rate,
            });
        }
        pos = body + size + (size & 1);
    }
    Err("WAVE without data".into())
}
