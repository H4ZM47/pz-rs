//! The Bink 1 container (`BIK` + revision byte): file header, audio track
//! table, frame index and per-frame packets.
//!
//! Layout (all little endian):
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 4 | `"BIK"` and the revision (`b`..`k`) |
//! | 4 | 4 | file size minus 8 |
//! | 8 | 4 | frame count |
//! | 12 | 4 | largest frame size |
//! | 16 | 4 | (unused here) |
//! | 20 | 4 | width |
//! | 24 | 4 | height |
//! | 28 | 4 | frame rate numerator |
//! | 32 | 4 | frame rate denominator |
//! | 36 | 4 | video flags |
//! | 40 | 4 | audio track count `n` |
//!
//! Then `n` u32 (maximum decoded audio size), `n` (u16 sample rate, u16
//! flags), `n` u32 track ids, and `frames + 1` u32 file offsets: bit 0 of
//! an offset marks a key frame, the last entry is the end of the data.
//! Each frame packet starts with one `u32 size` + data block per audio
//! track; the rest is video.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Video flag: the file has an alpha plane.
pub const FLAG_ALPHA: u32 = 0x0010_0000;
/// Video flag: grayscale (no chroma planes).
pub const FLAG_GRAYSCALE: u32 = 0x0002_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    pub max_decoded_size: u32,
    pub sample_rate: u16,
    pub flags: u16,
    pub id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The revision letter after `BIK` (`b'i'` for the game's files).
    pub revision: u8,
    pub frames: u32,
    pub largest_frame: u32,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub flags: u32,
    pub audio: Vec<AudioTrack>,
}

impl Header {
    pub fn has_alpha(&self) -> bool {
        self.flags & FLAG_ALPHA != 0
    }

    /// Seconds per frame.
    pub fn frame_period(&self) -> f64 {
        if self.fps_num == 0 {
            0.0
        } else {
            f64::from(self.fps_den) / f64::from(self.fps_num)
        }
    }
}

#[derive(Debug)]
pub enum BinkError {
    Io(io::Error),
    /// Not a Bink 1 file (Bink 2 files start with `KB2`).
    NotBink,
    Invalid(&'static str),
}

impl From<io::Error> for BinkError {
    fn from(e: io::Error) -> Self {
        BinkError::Io(e)
    }
}

impl std::fmt::Display for BinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinkError::Io(e) => write!(f, "{e}"),
            BinkError::NotBink => f.write_str("Not a Bink file."),
            BinkError::Invalid(s) => f.write_str(s),
        }
    }
}

fn u32_at(b: &[u8], o: usize) -> Result<u32, BinkError> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or(BinkError::Invalid("truncated header"))
}

fn u16_at(b: &[u8], o: usize) -> Result<u16, BinkError> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes(s.try_into().unwrap())).ok_or(BinkError::Invalid("truncated header"))
}

/// The fixed part of the header.
pub const FIXED_HEADER: usize = 44;
/// Track counts above this are treated as a corrupt header.
const MAX_TRACKS: u32 = 256;

impl Header {
    /// Parses the header from the start of a file. `bytes` must hold at
    /// least the fixed part and the audio table.
    pub fn parse(bytes: &[u8]) -> Result<Header, BinkError> {
        if bytes.len() < 4 || &bytes[..3] != b"BIK" {
            return Err(BinkError::NotBink);
        }
        let revision = bytes[3];
        if !matches!(revision, b'b' | b'd' | b'f' | b'g' | b'h' | b'i' | b'k') {
            return Err(BinkError::NotBink);
        }
        let tracks = u32_at(bytes, 40)?;
        if tracks > MAX_TRACKS {
            return Err(BinkError::Invalid("too many audio tracks"));
        }
        let n = tracks as usize;
        let mut audio = Vec::with_capacity(n);
        for i in 0..n {
            audio.push(AudioTrack {
                max_decoded_size: u32_at(bytes, FIXED_HEADER + 4 * i)?,
                sample_rate: u16_at(bytes, FIXED_HEADER + 4 * n + 4 * i)?,
                flags: u16_at(bytes, FIXED_HEADER + 4 * n + 4 * i + 2)?,
                id: u32_at(bytes, FIXED_HEADER + 8 * n + 4 * i)?,
            });
        }
        let h = Header {
            revision,
            frames: u32_at(bytes, 8)?,
            largest_frame: u32_at(bytes, 12)?,
            width: u32_at(bytes, 20)?,
            height: u32_at(bytes, 24)?,
            fps_num: u32_at(bytes, 28)?,
            fps_den: u32_at(bytes, 32)?,
            flags: u32_at(bytes, 36)?,
            audio,
        };
        if h.frames == 0 || h.width == 0 || h.height == 0 || h.width > 32768 || h.height > 32768 {
            return Err(BinkError::Invalid("bad video size or frame count"));
        }
        Ok(h)
    }

    /// Bytes taken by the header and the audio table (where the frame
    /// index starts).
    pub fn size(&self) -> usize {
        FIXED_HEADER + 12 * self.audio.len()
    }
}

/// One entry of the frame index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameEntry {
    pub offset: u32,
    pub size: u32,
    pub key: bool,
}

/// The frame index: `frames` entries built from `frames + 1` offsets.
pub fn parse_index(bytes: &[u8], frames: u32) -> Result<Vec<FrameEntry>, BinkError> {
    let n = frames as usize;
    if bytes.len() < 4 * (n + 1) {
        return Err(BinkError::Invalid("truncated frame index"));
    }
    let raw: Vec<u32> = (0..=n).map(|i| u32_at(bytes, 4 * i).unwrap()).collect();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = raw[i] & !1;
        let end = raw[i + 1] & !1;
        if end < start {
            return Err(BinkError::Invalid("frame offsets go backwards"));
        }
        out.push(FrameEntry { offset: start, size: end - start, key: raw[i] & 1 != 0 });
    }
    Ok(out)
}

/// A frame packet split into its audio blocks and the video data.
#[derive(Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    pub audio: Vec<&'a [u8]>,
    pub video: &'a [u8],
}

/// Splits a frame packet for a file with `tracks` audio tracks.
pub fn split_packet(data: &[u8], tracks: usize) -> Result<Packet<'_>, BinkError> {
    let mut pos = 0usize;
    let mut audio = Vec::with_capacity(tracks);
    for _ in 0..tracks {
        let size = u32_at(data, pos).map_err(|_| BinkError::Invalid("truncated audio packet"))? as usize;
        pos += 4;
        let block = data.get(pos..pos + size).ok_or(BinkError::Invalid("audio packet past frame end"))?;
        audio.push(block);
        pos += size;
    }
    Ok(Packet { audio, video: &data[pos..] })
}

/// Where frames are read from.
pub trait Source: Read + Seek {}
impl<T: Read + Seek> Source for T {}

/// An open Bink file: header, index and the stream frames are read from.
pub struct BinkFile {
    pub header: Header,
    pub index: Vec<FrameEntry>,
    file: Box<dyn Source>,
}

impl BinkFile {
    pub fn open(path: &Path) -> Result<BinkFile, BinkError> {
        BinkFile::from_source(Box::new(File::open(path)?))
    }

    /// Reads the header and index from the start of `file`.
    pub fn from_source(mut file: Box<dyn Source>) -> Result<BinkFile, BinkError> {
        let mut fixed = vec![0u8; FIXED_HEADER];
        file.read_exact(&mut fixed).map_err(|_| BinkError::NotBink)?;
        if &fixed[..3] != b"BIK" {
            return Err(BinkError::NotBink);
        }
        let tracks = u32_at(&fixed, 40)?;
        if tracks > MAX_TRACKS {
            return Err(BinkError::Invalid("too many audio tracks"));
        }
        fixed.resize(FIXED_HEADER + 12 * tracks as usize, 0);
        file.read_exact(&mut fixed[FIXED_HEADER..])?;
        let header = Header::parse(&fixed)?;
        let mut idx = vec![0u8; 4 * (header.frames as usize + 1)];
        file.read_exact(&mut idx)?;
        let index = parse_index(&idx, header.frames)?;
        Ok(BinkFile { header, index, file })
    }

    /// Reads frame `i` (0-based).
    pub fn read_frame(&mut self, i: u32) -> Result<Vec<u8>, BinkError> {
        let e = *self.index.get(i as usize).ok_or(BinkError::Invalid("frame out of range"))?;
        self.file.seek(SeekFrom::Start(u64::from(e.offset)))?;
        let mut buf = vec![0u8; e.size as usize];
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A container with `frames` frames of the given packets.
    pub(crate) fn build(revision: u8, w: u32, h: u32, tracks: &[(u16, u16)], packets: &[(bool, Vec<u8>)]) -> Vec<u8> {
        let n = tracks.len();
        let mut out = b"BIK".to_vec();
        out.push(revision);
        let frames = packets.len() as u32;
        for v in [0, frames, 0, frames, w, h, 30, 1, 0, n as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for _ in tracks {
            out.extend_from_slice(&4096u32.to_le_bytes());
        }
        for (rate, flags) in tracks {
            out.extend_from_slice(&rate.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
        }
        for i in 0..n {
            out.extend_from_slice(&(i as u32).to_le_bytes());
        }
        let mut off = (out.len() + 4 * (packets.len() + 1)) as u32;
        // Offsets are even (bit 0 is the key flag).
        off += off & 1;
        let mut index = Vec::new();
        // Frames are padded to even sizes (bit 0 of an offset is the key
        // flag).
        let padded: Vec<Vec<u8>> = packets
            .iter()
            .map(|(_, p)| {
                let mut p = p.clone();
                p.resize(p.len() + (p.len() & 1), 0);
                p
            })
            .collect();
        for ((key, _), p) in packets.iter().zip(&padded) {
            index.extend_from_slice(&(off | u32::from(*key)).to_le_bytes());
            off += p.len() as u32;
        }
        index.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&index);
        if out.len() & 1 == 1 {
            out.push(0);
        }
        for p in &padded {
            out.extend_from_slice(p);
        }
        let size = out.len() as u32 - 8;
        out[4..8].copy_from_slice(&size.to_le_bytes());
        out
    }

    #[test]
    fn header_and_index() {
        let pkts = vec![(true, vec![1, 2, 3, 4]), (false, vec![5, 6, 7])];
        let file = build(b'i', 64, 32, &[(48000, 0x7000)], &pkts);
        let h = Header::parse(&file).unwrap();
        assert_eq!((h.revision, h.frames, h.width, h.height), (b'i', 2, 64, 32));
        assert_eq!(h.audio, vec![AudioTrack { max_decoded_size: 4096, sample_rate: 48000, flags: 0x7000, id: 0 }]);
        assert_eq!(h.size(), 56);
        let idx = parse_index(&file[h.size()..], h.frames).unwrap();
        assert_eq!(idx.len(), 2);
        assert!(idx[0].key && !idx[1].key);
        assert_eq!((idx[0].size, idx[1].size), (4, 4));
        assert_eq!(&file[idx[1].offset as usize..][..4], &[5, 6, 7, 0]);
        assert!((h.frame_period() - 1.0 / 30.0).abs() < 1e-12);
    }

    #[test]
    fn rejects_other_formats() {
        assert!(matches!(Header::parse(b"KB2n\0\0\0\0"), Err(BinkError::NotBink)));
        assert!(matches!(Header::parse(b"BIKz"), Err(BinkError::NotBink)));
        let mut file = build(b'i', 64, 32, &[], &[(true, vec![0; 4])]);
        file[20..24].copy_from_slice(&0u32.to_le_bytes());
        assert!(matches!(Header::parse(&file), Err(BinkError::Invalid(_))));
    }

    #[test]
    fn packets_split_audio_then_video() {
        let mut p = Vec::new();
        p.extend_from_slice(&3u32.to_le_bytes());
        p.extend_from_slice(&[9, 9, 9]);
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&[7, 7]);
        let s = split_packet(&p, 2).unwrap();
        assert_eq!(s.audio, vec![&[9u8, 9, 9][..], &[][..]]);
        assert_eq!(s.video, &[7, 7]);
        assert!(split_packet(&p[..5], 1).is_err());
    }

    #[test]
    fn open_reads_frames() {
        let dir = std::env::temp_dir().join(format!(
            "pz-video-bink-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.bik");
        let pkts = vec![(true, vec![1, 2, 3, 4, 5, 6]), (false, vec![8; 10])];
        std::fs::write(&path, build(b'i', 16, 16, &[], &pkts)).unwrap();
        let mut f = BinkFile::open(&path).unwrap();
        assert_eq!(f.read_frame(1).unwrap(), vec![8; 10]);
        assert_eq!(f.read_frame(0).unwrap(), vec![1, 2, 3, 4, 5, 6]);
        assert!(f.read_frame(2).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Every video the game ships parses, and its index covers the file.
    #[test]
    fn game_videos_parse() {
        let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
        let videos = Path::new(&dir).join("media/videos");
        let mut files = Vec::new();
        for d in [videos.clone(), videos.join("survival_guide")] {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.extension().is_some_and(|x| x == "bik") {
                    files.push(p);
                }
            }
        }
        assert!(files.len() > 20);
        for p in files {
            let mut f = BinkFile::open(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            let len = std::fs::metadata(&p).unwrap().len();
            let last = f.index.last().unwrap();
            assert_eq!(u64::from(last.offset + last.size), len, "{}", p.display());
            assert!(f.index[0].key);
            let first = f.read_frame(0).unwrap();
            let pk = split_packet(&first, f.header.audio.len()).unwrap();
            assert!(!pk.video.is_empty());
        }
    }
}
