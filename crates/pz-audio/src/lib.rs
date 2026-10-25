//! Audio for the port: FMOD Studio banks decoded and played in pure Rust.
//!
//! The original drives FMOD Studio through `fmod.javafmod` from
//! `zombie.audio.*`, `zombie.SoundManager` and `zombie.GameSounds`. This
//! crate replaces the FMOD runtime: it reads the game's `.bank` files
//! (RIFF `FEV ` containers holding the Studio event data and an FSB5 sample
//! bank) and decodes their Vorbis samples without any FMOD code.

pub mod bank;
pub mod dsp;
pub mod fsb5;
#[cfg(feature = "output")]
pub mod output;
pub mod reader;
pub mod riff;
pub mod strings;
pub mod studio;
pub mod vorbis;
pub mod vorbis_setup;
pub mod world_sound;

#[cfg(test)]
mod tests;

use std::path::Path;

/// A memory-mapped Studio bank file.
pub struct BankFile {
    map: memmap2::Mmap,
    pub root: riff::Chunk,
    fsb_range: Option<std::ops::Range<usize>>,
    pub fsb: Option<fsb5::Fsb5>,
}

impl BankFile {
    pub fn open(path: &Path) -> std::io::Result<BankFile> {
        let f = std::fs::File::open(path)?;
        // SAFETY: game banks are read-only assets; nothing truncates them while mapped.
        let map = unsafe { memmap2::Mmap::map(&f)? };
        let bad = |m: &str| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: {m}", path.display()),
            )
        };
        let root = riff::parse(&map).ok_or_else(|| bad("not an FMOD Studio bank"))?;
        let fsb_range = riff::fsb5_range(&map, &root);
        let fsb = match &fsb_range {
            Some(r) => Some(fsb5::Fsb5::parse(&map[r.clone()]).map_err(|e| bad(&e.to_string()))?),
            None => None,
        };
        Ok(BankFile {
            map,
            root,
            fsb_range,
            fsb,
        })
    }

    /// Payload of the first chunk (or list) with this id.
    pub fn chunk(&self, id: &[u8; 4]) -> Option<&[u8]> {
        self.root.find(id).map(|c| &self.map[c.body.clone()])
    }

    /// The Studio objects stored in this bank.
    pub fn model(&self) -> reader::Result<bank::BankModel> {
        bank::BankModel::parse(&self.map, &self.root)
    }

    /// The string table, for `*.strings.bank` files.
    pub fn strings(&self) -> Option<strings::StringTable> {
        strings::StringTable::parse(self.chunk(b"STDT")?).ok()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.map
    }

    /// The FSB5 bytes (sample ranges in [`fsb5::Sample`] are relative to this).
    pub fn fsb_bytes(&self) -> &[u8] {
        self.fsb_range.clone().map_or(&[][..], |r| &self.map[r])
    }

    /// Decoder for sample `index` of the embedded FSB5.
    pub fn decoder<'a>(
        &'a self,
        index: usize,
        setups: &vorbis_setup::SetupHeaders,
    ) -> Result<vorbis::Decoder<'a>, vorbis::Error> {
        let s = self
            .fsb
            .as_ref()
            .and_then(|f| f.samples.get(index))
            .ok_or(vorbis::Error::NotVorbis)?;
        vorbis::Decoder::new(self.fsb_bytes(), s, setups)
    }
}
