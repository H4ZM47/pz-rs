//! `zombie.iso.worldgen.WorldGenParams`: the world seed and the per-position
//! random generators derived from it.

use crate::java::{string_hash, JavaRandom};

/// `WorldGenParams.GENERATION_SIZE`: chunks are generated in blocks of
/// `2 × 2`.
pub const GENERATION_SIZE: i32 = 2;
/// `WorldGenParams.GENERATION_SQUARES`: squares per side of a block.
pub const GENERATION_SQUARES: i32 = 16;

/// `WorldGenParams` (seed and procedural-cell bounds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldGenParams {
    pub seed_string: String,
    /// `seedString.hashCode()`.
    pub seed: i32,
    pub min_x_cell: i32,
    pub min_y_cell: i32,
    pub max_x_cell: i32,
    pub max_y_cell: i32,
}

impl WorldGenParams {
    /// `setSeedString(seed)` with the default ±250 cell bounds.
    pub fn new(seed_string: &str) -> Self {
        WorldGenParams {
            seed_string: seed_string.to_string(),
            seed: string_hash(seed_string),
            min_x_cell: -250,
            min_y_cell: -250,
            max_x_cell: 250,
            max_y_cell: 250,
        }
    }

    /// `getSeed()`.
    pub fn seed(&self) -> i64 {
        self.seed as i64
    }

    /// `getRandom(wx, wy, offset)`.
    pub fn random(&self, wx: i32, wy: i32, offset: i64) -> JavaRandom {
        let seed = self.seed as i64;
        let mut r = JavaRandom::new(seed.wrapping_add(offset));
        let wx_rnd = r.next_long();
        let wy_rnd = r.next_long();
        r.set_seed((wx as i64).wrapping_mul(wx_rnd) ^ (wy as i64).wrapping_mul(wy_rnd) ^ seed);
        r
    }
}

/// `WorldGenParams.FILE_MAGIC`.
pub const WORLDGEN_MAGIC: [u8; 4] = *b"WGEN";

/// `IsoWorld.getWorldVersion()` in this build.
const WORLD_VERSION: i32 = 249;

/// What `WorldGenParams.load()` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadResult {
    /// `Result.NOT_PRESENT`: no `map_worldgen.bin`.
    NotPresent,
    /// `Result.DONE` (also after a logged failure, with the fields read up
    /// to it).
    Done,
}

impl WorldGenParams {
    /// The bytes `save()` writes to `map_worldgen.bin`: `"WGEN"`, the world
    /// version, the seed string (`GameWindow.WriteString`) and the four
    /// cell bounds.
    pub fn save_bytes(&self) -> Vec<u8> {
        use pz_kahlua::java_io::JavaWrite;
        let mut out = Vec::new();
        out.extend_from_slice(&WORLDGEN_MAGIC);
        out.put_i32(WORLD_VERSION);
        out.write_string_utf(Some(&self.seed_string));
        out.put_i32(self.min_x_cell);
        out.put_i32(self.min_y_cell);
        out.put_i32(self.max_x_cell);
        out.put_i32(self.max_y_cell);
        out
    }

    /// `load()` from the bytes: each field is set as it is read
    /// (`setSeedString` also sets the seed), so a failure keeps the fields
    /// before it.
    pub fn load_bytes(&mut self, data: &[u8], path: &str) -> std::io::Result<()> {
        let mut r = pz_kahlua::java_io::JavaReader::new(data);
        if r.take(4)? != WORLDGEN_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{path} does not appear to be map_worldgen.bin"),
            ));
        }
        let _world_version = r.get_i32()?;
        let seed = r.read_string_utf()?;
        self.seed = string_hash(&seed);
        self.seed_string = seed;
        self.min_x_cell = r.get_i32()?;
        self.min_y_cell = r.get_i32()?;
        self.max_x_cell = r.get_i32()?;
        self.max_y_cell = r.get_i32()?;
        Ok(())
    }

    /// `WorldGenParams.INSTANCE.save()` (`GameWindow.save`, after
    /// `map_sand.bin`): nothing with saving off. Failures are returned (the
    /// original logs them).
    pub fn save_file(&self, save_dir: &std::path::Path, no_save: bool) -> std::io::Result<()> {
        if no_save {
            return Ok(());
        }
        std::fs::write(save_dir.join("map_worldgen.bin"), self.save_bytes())
    }

    /// `WorldGenParams.INSTANCE.load()`. A read failure is logged and
    /// still `Done`, like the original.
    pub fn load_file(&mut self, save_dir: &std::path::Path) -> LoadResult {
        let path = save_dir.join("map_worldgen.bin");
        if !path.exists() {
            return LoadResult::NotPresent;
        }
        let r = std::fs::read(&path).and_then(|d| self.load_bytes(&d, &path.to_string_lossy()));
        if let Err(e) = r {
            eprintln!("ERROR: WorldGenParams.load: {e}");
        }
        LoadResult::Done
    }
}

/// `WorldGenUtils.getCornerOfGeneration(b)`: the first chunk coordinate of
/// the generation block containing `b`.
pub fn corner_of_generation(b: i32) -> i32 {
    b.div_euclid(GENERATION_SIZE) * GENERATION_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_worldgen_round_trips() {
        let mut p = WorldGenParams::new("seed é");
        p.min_x_cell = -3;
        p.max_y_cell = 40;
        let b = p.save_bytes();
        assert_eq!(&b[..8], b"WGEN\0\0\0\xf9");
        let mut q = WorldGenParams::new("");
        q.load_bytes(&b, "x").unwrap();
        assert_eq!(q, p);
        assert!(q.load_bytes(b"WGEX", "x").is_err());
    }

    #[test]
    fn corners() {
        assert_eq!(corner_of_generation(5), 4);
        assert_eq!(corner_of_generation(-1), -2);
        assert_eq!(corner_of_generation(0), 0);
    }

    #[test]
    fn random_depends_on_position() {
        let p = WorldGenParams::new("abc");
        assert_eq!(p.seed, 96354);
        let a = p.random(1, 2, 0).next_long();
        let b = p.random(1, 2, 0).next_long();
        let c = p.random(2, 1, 0).next_long();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
