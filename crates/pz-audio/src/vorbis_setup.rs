//! Vorbis setup headers for FSB5 Vorbis samples.
//!
//! FSB5 strips the Vorbis identification and setup packets. Each sample's
//! `VORBIS_DATA` chunk starts with the CRC-32 of the setup packet the FMOD
//! encoder used, and the FMOD runtime keeps the matching packets in a table
//! compiled into its core library. The game ships that library
//! (`libfmod.dylib`, `fmod.dll`, `libfmod.so`), so the packets are read from
//! the user's install at runtime rather than distributed here.
//!
//! The runtime table is an array of 32-byte entries:
//!
//! ```text
//! 0x00 u64 data pointer   0x08 u32 packet size   0x0c u32 packet CRC-32
//! 0x10 u64 base pointer   0x18 u32 patch offset  0x1c u32 patch size
//! ```
//!
//! With a null base the data pointer is the whole packet. Otherwise the
//! packet is the first `size` bytes of the base packet with `patch size`
//! bytes at `patch offset` replaced by the data (`fmod_codec_fsbvorbis.cpp`
//! copies them exactly that way). Pointers are virtual addresses; the
//! address-to-file-offset delta is found by trial and every packet is
//! verified against its CRC, so no executable-format parsing is needed and
//! the same code works for Mach-O (including fat binaries), PE and ELF.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *e = c;
        }
        t
    });
    !data.iter().fold(!0u32, |c, &b| {
        t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8)
    })
}

const SETUP_MAGIC: &[u8; 7] = b"\x05vorbis";

/// Setup packets by CRC-32.
#[derive(Default, Clone)]
pub struct SetupHeaders {
    by_crc: HashMap<u32, Vec<u8>>,
}

impl SetupHeaders {
    pub fn get(&self, crc: u32) -> Option<&[u8]> {
        self.by_crc.get(&crc).map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.by_crc.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_crc.is_empty()
    }

    pub fn insert(&mut self, packet: Vec<u8>) {
        self.by_crc.insert(crc32(&packet), packet);
    }

    /// Extract every verifiable setup packet from an FMOD core library image.
    pub fn from_fmod_library(lib: &[u8]) -> SetupHeaders {
        let magic_hits: Vec<usize> = find_all(lib, SETUP_MAGIC);
        let mut out = SetupHeaders::default();
        if magic_hits.is_empty() {
            return out;
        }
        // Candidate address deltas, learnt from entries whose packet verifies.
        let mut deltas: Vec<(i64, u64)> = Vec::new();

        let mut p = 0usize;
        while p + 32 <= lib.len() {
            let e = Entry::read(&lib[p..p + 32]);
            if let Some(e) = e {
                if let std::collections::hash_map::Entry::Vacant(v) = out.by_crc.entry(e.crc) {
                    if let Some(pkt) = e.resolve(lib, &mut deltas, &magic_hits) {
                        v.insert(pkt);
                    }
                }
            }
            p += 8;
        }
        out
    }

    /// Load the setup packets from the FMOD library inside a game install.
    pub fn from_game_dir(dir: &Path) -> std::io::Result<SetupHeaders> {
        let mut out = SetupHeaders::default();
        for lib in fmod_library_candidates(dir) {
            let data = std::fs::read(&lib)?;
            out.by_crc
                .extend(SetupHeaders::from_fmod_library(&data).by_crc);
        }
        Ok(out)
    }
}

/// FMOD core library files shipped with the game, for each desktop platform.
pub fn fmod_library_candidates(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in [
        "",
        "win64",
        "linux64",
        "natives",
        "Contents/Java",
        "Contents/MacOS",
    ] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let is_core = name == "libfmod.dylib"
                || name == "fmod.dll"
                || name == "fmod64.dll"
                || name == "libfmod.so"
                || (name.starts_with("libfmod.so.") && !name.contains("studio"));
            if is_core {
                out.push(e.path());
            }
        }
    }
    out
}

struct Entry {
    data: u64,
    size: usize,
    crc: u32,
    base: u64,
    patch_off: usize,
    patch_len: usize,
}

impl Entry {
    fn read(b: &[u8]) -> Option<Entry> {
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let e = Entry {
            data: u64_at(0),
            size: u32_at(8) as usize,
            crc: u32_at(12),
            base: u64_at(16),
            patch_off: u32_at(24) as usize,
            patch_len: u32_at(28) as usize,
        };
        // Real setup packets are a few KiB.
        let plausible = e.data != 0
            && e.crc != 0
            && (64..=0x10000).contains(&e.size)
            && if e.base == 0 {
                e.patch_off == 0 && e.patch_len == 0
            } else {
                e.patch_len > 0 && e.patch_off + e.patch_len <= e.size
            };
        plausible.then_some(e)
    }

    fn build(&self, lib: &[u8], (delta, mask): (i64, u64)) -> Option<Vec<u8>> {
        let at = |ptr: u64, len: usize| -> Option<&[u8]> {
            let off = usize::try_from(((ptr & mask) as i64).checked_sub(delta)?).ok()?;
            lib.get(off..off.checked_add(len)?)
        };
        let first = at(
            if self.base == 0 { self.data } else { self.base },
            self.size,
        )?;
        if !first.starts_with(SETUP_MAGIC) {
            return None;
        }
        let pkt = if self.base == 0 {
            first.to_vec()
        } else {
            let mut v = first.to_vec();
            v[self.patch_off..self.patch_off + self.patch_len]
                .copy_from_slice(at(self.data, self.patch_len)?);
            v
        };
        (crc32(&pkt) == self.crc).then_some(pkt)
    }

    fn resolve(
        &self,
        lib: &[u8],
        deltas: &mut Vec<(i64, u64)>,
        magic: &[usize],
    ) -> Option<Vec<u8>> {
        for &d in deltas.iter() {
            if let Some(p) = self.build(lib, d) {
                return Some(p);
            }
        }
        // Learn a new delta: the packet start must be one of the magic hits.
        let start = if self.base == 0 { self.data } else { self.base };
        // Arm64 chained fixups keep the target in the low bits.
        for mask in [u64::MAX, (1u64 << 36) - 1] {
            for &h in magic {
                let d = ((start & mask) as i64 - h as i64, mask);
                if deltas.contains(&d) {
                    continue;
                }
                if let Some(p) = self.build(lib, d) {
                    deltas.push(d);
                    return Some(p);
                }
            }
        }
        None
    }
}

fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    hay.windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect()
}
