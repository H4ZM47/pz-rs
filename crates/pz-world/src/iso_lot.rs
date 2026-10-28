//! `IsoLot`: one 8x8 chunk's tile data read from a `world_X_Y.lotpack`.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use pz_core::debug::{self, StackElement};

use crate::bra::BufferedRandomAccessFile;
use crate::java::io_throwable;
use crate::map_files::{header_name, lotpack_name, LotHeader, MapFiles, LOTPACK_MAGIC};

/// Squares per chunk level times the 64 supported levels (-32..=31).
pub const OFFSET_TABLE_LEN: usize = 4096;

pub struct IsoLot {
    pub max_level: i32,
    pub min_level: i32,
    pub wx: i32,
    pub wy: i32,
    /// Per square `x + y*8 + (z - info.minLevel)*64`: index into `data`, or -1.
    pub offset_in_data: Box<[i32; OFFSET_TABLE_LEN]>,
    /// For each used square: tile count, then indices into `info.tilesUsed`.
    pub data: Vec<i32>,
    /// Header name of the `LotHeader` this lot was read with (`info`).
    pub info: Option<String>,
    /// Index of the `MapFiles` the data came from.
    pub map_files: usize,
    in_file: Option<BufferedRandomAccessFile<File>>,
    last_used_path: PathBuf,
    version: i32,
}

impl Default for IsoLot {
    fn default() -> Self {
        IsoLot {
            max_level: 0,
            min_level: 0,
            wx: 0,
            wy: 0,
            offset_in_data: Box::new([0; OFFSET_TABLE_LEN]),
            data: Vec::new(),
            info: None,
            map_files: 0,
            in_file: None,
            last_used_path: PathBuf::new(),
            version: 0,
        }
    }
}

impl IsoLot {
    pub fn info<'a>(&self, map_files: &'a [MapFiles]) -> Option<&'a LotHeader> {
        map_files
            .get(self.map_files)?
            .info_headers
            .get(self.info.as_deref()?)
    }

    /// Tile indices for a square, or `None` if the square is empty.
    pub fn square_tiles(&self, square_xyz: usize) -> Option<&[i32]> {
        let off = *self.offset_in_data.get(square_xyz)?;
        if off == -1 {
            return None;
        }
        let n = self.data[off as usize];
        if n <= 0 {
            return None;
        }
        Some(&self.data[off as usize + 1..off as usize + 1 + n as usize])
    }

    /// `IsoLot.load(mapFiles, cX, cY, wX, wY, ch)`. Errors are logged and
    /// leave the lot empty, like the original's catch block.
    pub fn load(
        &mut self,
        map_files: &[MapFiles],
        mf: usize,
        c_x: i32,
        c_y: i32,
        w_x: i32,
        w_y: i32,
    ) {
        let header = header_name(c_x, c_y);
        let files = &map_files[mf];
        self.map_files = mf;
        self.info = files
            .info_headers
            .contains_key(&header)
            .then_some(header.clone());
        self.wx = w_x;
        self.wy = w_y;
        let Some(info) = files.info_headers.get(&header) else {
            // NPE on `this.info.minLevel` upstream, caught by the same handler.
            self.fail(io::Error::other("NullPointerException"));
            return;
        };
        if let Err(e) = self.load_inner(files, info, c_x, c_y) {
            self.fail(e);
        }
    }

    fn fail(&mut self, e: io::Error) {
        self.offset_in_data.fill(-1);
        self.data.clear();
        // `ExceptionLogger.logException(ex)`.
        debug::log_exception(
            &io_throwable(&e),
            Some(&StackElement::new("zombie.iso.IsoLot", "load")),
        );
    }

    fn load_inner(
        &mut self,
        files: &MapFiles,
        info: &LotHeader,
        c_x: i32,
        c_y: i32,
    ) -> io::Result<()> {
        let pack = files
            .info_file_names
            .get(&lotpack_name(c_x, c_y))
            .ok_or_else(|| io::Error::other("NullPointerException"))?;
        let abs = std::path::absolute(pack).unwrap_or_else(|_| pack.clone());
        if self.in_file.is_none() || self.last_used_path != abs {
            self.in_file = None;
            let mut f = BufferedRandomAccessFile::open(Path::new(&abs), 4096)?;
            self.last_used_path = abs;
            let mut magic = [0u8; 4];
            f.read_into(&mut magic)?;
            if magic == LOTPACK_MAGIC {
                self.version = f.read_int_le()?;
                if self.version < 0 || self.version > 1 {
                    return Err(io::Error::other(format!(
                        "Unsupported version {}",
                        self.version
                    )));
                }
            } else {
                f.seek(0)?;
                self.version = 0;
            }
            self.in_file = Some(f);
        }
        let f = self.in_file.as_mut().unwrap();
        let mut skip = 0;
        let lwx = self.wx - c_x * 32;
        let lwy = self.wy - c_y * 32;
        let index = (lwx * 32 + lwy) as i64;
        f.seek(((if self.version >= 1 { 8 } else { 0 }) + 4 + index * 8) as u64)?;
        let pos = f.read_int_le()?;
        f.seek(pos as u32 as u64)?;
        self.data.clear();
        let min_z = info.min_level.max(-32);
        let max_z = info.max_level.min(31);
        self.min_level = 0;
        let mut used_z = 0;
        for z in min_z..=max_z {
            for x in 0..8 {
                for y in 0..8 {
                    let square_xyz = (x + y * 8 + (z - info.min_level) * 64) as usize;
                    self.offset_in_data[square_xyz] = -1;
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    let count = f.read_int_le()?;
                    if count == -1 {
                        skip = f.read_int_le()?;
                        if skip > 0 {
                            skip -= 1;
                            continue;
                        }
                    }
                    if count <= 1 {
                        continue;
                    }
                    self.offset_in_data[square_xyz] = self.data.len() as i32;
                    self.data.push(count - 1);
                    self.min_level = z.min(self.min_level);
                    used_z = z.max(used_z);
                    let _room = f.read_int_le()?;
                    for _ in 1..count {
                        let d = f.read_int_le()?;
                        self.data.push(d);
                    }
                }
            }
        }
        self.max_level = used_z + 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_core::debug::{DebugType, LogSeverity, MemorySink};

    #[test]
    fn load_failure_goes_to_exception_logger() {
        let sink = MemorySink::new();
        debug::with(|d| {
            d.set_sink(sink.clone());
            d.set_severity(DebugType::General, LogSeverity::General);
        });
        let files = [MapFiles::new("Test", PathBuf::from("/nonexistent"), 0, 0)];
        let mut lot = IsoLot::default();
        lot.load(&files, 0, 0, 0, 0, 0);
        assert!(lot.data.is_empty());
        assert!(lot.offset_in_data.iter().all(|&o| o == -1));
        assert_eq!(
            sink.log().stderr(),
            "ERROR: General      f:0> IsoLot.load> Exception thrown\n\
             \tjava.lang.NullPointerException at No Stack Trace Available.\n\
             \tStack trace:\n"
        );
        assert_eq!(debug::with(|d| d.exception_count), 1);
    }
}
