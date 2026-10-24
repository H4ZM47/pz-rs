//! `zombie.iso.NewMapBinaryFile` (`.pzby`): the binary map pieces basements
//! and basement access pieces are stamped from. Only the power-of-two
//! layout (`pot = true`, 8 × 8 chunks) the game uses for them is read.

use std::io;
use std::path::{Path, PathBuf};

use pz_world::bra::BufferedRandomAccessFile;
use pz_world::GameRand;

/// `NewMapBinaryFile.chunkDim` for `pot = true`.
pub const CHUNK_DIM: i32 = 8;

/// A room of a `.pzby` header (`RoomDef`, coordinates local to the piece).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PzbyRoom {
    pub name: String,
    pub level: i32,
    /// `(x, y, w, h)`.
    pub rects: Vec<(i32, i32, i32, i32)>,
    pub x: i32,
    pub y: i32,
    pub x2: i32,
    pub y2: i32,
}

impl PzbyRoom {
    /// `RoomDef.isEmptyOutside()`.
    pub fn is_empty_outside(&self) -> bool {
        self.name.eq_ignore_ascii_case("emptyoutside")
    }

    /// `RoomDef.CalculateBounds()`.
    fn calculate_bounds(&mut self) {
        self.x = 10_000_000;
        self.y = 10_000_000;
        self.x2 = -1_000_000;
        self.y2 = -1_000_000;
        for &(x, y, w, h) in &self.rects {
            self.x = self.x.min(x);
            self.y = self.y.min(y);
            self.x2 = self.x2.max(x + w);
            self.y2 = self.y2.max(y + h);
        }
    }

    /// `RoomDef.intersects(x, y, w, h)`.
    pub fn intersects(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.rects
            .iter()
            .any(|&(rx, ry, rw, rh)| !(x + w <= rx || x >= rx + rw || y + h <= ry || y >= ry + rh))
    }
}

/// A building of a `.pzby` header (`BuildingDef`): indices into
/// [`PzbyHeader::rooms`] and the bounds `CalculateBounds` sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PzbyBuilding {
    pub rooms: Vec<usize>,
    pub emptyoutside: Vec<usize>,
    pub x: i32,
    pub y: i32,
    pub x2: i32,
    pub y2: i32,
}

impl PzbyBuilding {
    /// `getW()`.
    pub fn w(&self) -> i32 {
        self.x2.wrapping_sub(self.x)
    }
    /// `getH()`.
    pub fn h(&self) -> i32 {
        self.y2.wrapping_sub(self.y)
    }
    /// `getMaxLevel()` (`-100` without rooms).
    pub fn max_level(&self, rooms: &[PzbyRoom]) -> i32 {
        self.rooms
            .iter()
            .map(|&r| rooms[r].level)
            .fold(-100, i32::max)
    }
    /// `getMinLevel()` (`100` without rooms).
    pub fn min_level(&self, rooms: &[PzbyRoom]) -> i32 {
        self.rooms
            .iter()
            .map(|&r| rooms[r].level)
            .fold(100, i32::min)
    }
}

/// `NewMapBinaryFile.Header`.
#[derive(Clone, Debug)]
pub struct PzbyHeader {
    pub path: PathBuf,
    pub version: i32,
    pub used_tile_names: Vec<String>,
    /// Size in chunks.
    pub width: i32,
    pub height: i32,
    pub levels: i32,
    pub rooms: Vec<PzbyRoom>,
    pub buildings: Vec<PzbyBuilding>,
    chunk_table_position: u64,
}

/// `NewMapBinaryFile.ChunkData`: per square of one 8 × 8 chunk, the tile
/// indices (`data[x][y][z]`, `None` when empty) and the attribute bits.
#[derive(Clone, Debug)]
pub struct PzbyChunk {
    pub data: Vec<Option<Vec<i32>>>,
    pub attributes: Vec<i32>,
    levels: i32,
}

impl PzbyChunk {
    /// `data[x][y][z]`.
    pub fn tiles(&self, x: i32, y: i32, z: i32) -> Option<&[i32]> {
        self.data[((x * CHUNK_DIM + y) * self.levels + z) as usize].as_deref()
    }
    /// `attributes[z][x + y * chunkDim]`: 1 keeps existing floors, 2 walls,
    /// 4 everything else.
    pub fn attribute(&self, x: i32, y: i32, z: i32) -> i32 {
        self.attributes[(z * CHUNK_DIM * CHUNK_DIM + x + y * CHUNK_DIM) as usize]
    }
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn line(r: &mut BufferedRandomAccessFile) -> io::Result<String> {
    r.get_next_line()?
        .ok_or_else(|| bad("NullPointerException: missing string"))
}

impl PzbyHeader {
    /// `loadHeader(fileName)`. Each room draws `Rand.Next(2)` for its
    /// `lightsActive`, as the original does.
    pub fn load(path: &Path, rand: &mut dyn GameRand) -> io::Result<PzbyHeader> {
        let mut r = BufferedRandomAccessFile::open(path, 4096)?;
        let magic = [r.read()?, r.read()?, r.read()?, r.read()?];
        if magic != [80, 90, 66, 89] {
            return Err(bad("unrecognized file format"));
        }
        let version = r.read_int_le()?;
        let n = r.read_int_le()?;
        let mut used_tile_names = Vec::new();
        for _ in 0..n {
            used_tile_names.push(pz_world::java::java_trim(&line(&mut r)?).to_string());
        }
        let width = r.read_int_le()?;
        let height = r.read_int_le()?;
        let levels = r.read_int_le()?;
        let num_rooms = r.read_int_le()?;
        let mut rooms = Vec::new();
        for _ in 0..num_rooms {
            let name = line(&mut r)?;
            let level = r.read_int_le()?;
            let nr = r.read_int_le()?;
            let mut rects = Vec::new();
            for _ in 0..nr {
                let x = r.read_int_le()?;
                let y = r.read_int_le()?;
                let w = r.read_int_le()?;
                let h = r.read_int_le()?;
                rects.push((x, y, w, h));
            }
            let mut room = PzbyRoom {
                name,
                level,
                rects,
                x: 0,
                y: 0,
                x2: 0,
                y2: 0,
            };
            room.calculate_bounds();
            let no = r.read_int_le()?;
            for _ in 0..no * 3 {
                r.read_int_le()?;
            }
            rand.next(2);
            rooms.push(room);
        }
        let num_buildings = r.read_int_le()?;
        let mut buildings = Vec::new();
        for _ in 0..num_buildings {
            let nr = r.read_int_le()?;
            let mut b = PzbyBuilding {
                rooms: Vec::new(),
                emptyoutside: Vec::new(),
                x: 0,
                y: 0,
                x2: 0,
                y2: 0,
            };
            for _ in 0..nr {
                let id = r.read_int_le()?;
                let i = usize::try_from(id)
                    .ok()
                    .filter(|&i| i < rooms.len())
                    .ok_or_else(|| bad("NullPointerException: room id"))?;
                if rooms[i].is_empty_outside() {
                    b.emptyoutside.push(i);
                } else {
                    b.rooms.push(i);
                }
            }
            b.calculate_bounds(&rooms);
            buildings.push(b);
        }
        Ok(PzbyHeader {
            path: path.to_path_buf(),
            version,
            used_tile_names,
            width,
            height,
            levels,
            rooms,
            buildings,
            chunk_table_position: r.file_pointer(),
        })
    }

    /// A header with the given rooms and one building of all of them (for
    /// tests of code that only reads headers).
    #[cfg(test)]
    pub(crate) fn synthetic(levels: i32, rooms: Vec<PzbyRoom>) -> PzbyHeader {
        let mut rooms = rooms;
        for r in &mut rooms {
            r.calculate_bounds();
        }
        let mut b = PzbyBuilding {
            rooms: (0..rooms.len()).collect(),
            emptyoutside: Vec::new(),
            x: 0,
            y: 0,
            x2: 0,
            y2: 0,
        };
        b.calculate_bounds(&rooms);
        PzbyHeader {
            path: PathBuf::new(),
            version: 1,
            used_tile_names: Vec::new(),
            width: 1,
            height: 1,
            levels,
            rooms,
            buildings: vec![b],
            chunk_table_position: 0,
        }
    }

    /// `loadChunk(header, chunkX, chunkY)`.
    pub fn load_chunk(&self, chunk_x: i32, chunk_y: i32) -> io::Result<PzbyChunk> {
        let mut r = BufferedRandomAccessFile::open(&self.path, 4096)?;
        let index = chunk_y * self.width + chunk_x;
        r.seek(
            self.chunk_table_position
                .wrapping_add((index as i64 * 8) as u64),
        )?;
        let pos = r.read_int_le()?;
        r.seek(pos as u32 as u64)?;
        let levels = self.levels.max(0);
        let n = (CHUNK_DIM * CHUNK_DIM * levels) as usize;
        let mut attributes = vec![0; n];
        let mut data = vec![None; n];
        let mut skip = 0;
        for z in 0..levels {
            for x in 0..CHUNK_DIM {
                for y in 0..CHUNK_DIM {
                    let i = (z * CHUNK_DIM * CHUNK_DIM + x + y * CHUNK_DIM) as usize;
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    let count = r.read_int_le()?;
                    if count == -1 {
                        skip = r.read_int_le()?;
                        if skip > 0 {
                            skip -= 1;
                            continue;
                        }
                    }
                    attributes[i] = if count > 1 { r.read_int_le()? } else { 0 };
                }
            }
        }
        skip = 0;
        for z in 0..levels {
            for x in 0..CHUNK_DIM {
                for y in 0..CHUNK_DIM {
                    let i = ((x * CHUNK_DIM + y) * levels + z) as usize;
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    let count = r.read_int_le()?;
                    if count == -1 {
                        skip = r.read_int_le()?;
                        if skip > 0 {
                            skip -= 1;
                            continue;
                        }
                    }
                    if count > 1 {
                        let mut v = Vec::with_capacity(count as usize - 1);
                        for _ in 1..count {
                            v.push(r.read_int_le()?);
                        }
                        data[i] = Some(v);
                    }
                }
            }
        }
        Ok(PzbyChunk {
            data,
            attributes,
            levels,
        })
    }
}

impl PzbyBuilding {
    /// `BuildingDef.CalculateBounds` (the bounds part).
    fn calculate_bounds(&mut self, rooms: &[PzbyRoom]) {
        self.x = i32::MAX;
        self.y = i32::MAX;
        self.x2 = i32::MIN;
        self.y2 = i32::MIN;
        for &r in self.rooms.iter().chain(self.emptyoutside.iter()) {
            for &(x, y, w, h) in &rooms[r].rects {
                self.x = self.x.min(x);
                self.y = self.y.min(y);
                self.x2 = self.x2.max(x + w);
                self.y2 = self.y2.max(y + h);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Count(i32);
    impl GameRand for Count {
        fn next(&mut self, _max: i32) -> i32 {
            self.0 += 1;
            0
        }
    }

    fn le(v: &mut Vec<u8>, i: i32) {
        v.extend_from_slice(&i.to_le_bytes());
    }

    /// A one-chunk, one-level piece with a floor tile at (1, 2) and a
    /// run-length skip over the other squares.
    fn sample(dir: &Path) -> PathBuf {
        let mut v = b"PZBY".to_vec();
        le(&mut v, 1);
        le(&mut v, 1);
        v.extend_from_slice(b"floor_0\n");
        for i in [1, 1, 1, 1] {
            le(&mut v, i);
        }
        v.extend_from_slice(b"room\n");
        for i in [0, 1, 0, 0, 3, 4, 0, 1, 1, 0] {
            le(&mut v, i);
        }
        let table = v.len() as i32;
        le(&mut v, table + 4);
        // attributes: (0,0)..(1,1) empty via skip of 10, square 10 = x 1, y 2
        // has two ints and attribute 1, then skip the remaining 53.
        for i in [-1, 10, 2, 1, -1, 53] {
            le(&mut v, i);
        }
        for i in [-1, 10, 2, 0, -1, 53] {
            le(&mut v, i);
        }
        let p = dir.join("sample.pzby");
        std::fs::write(&p, v).unwrap();
        p
    }

    #[test]
    fn header_and_chunk() {
        let dir = std::env::temp_dir().join(format!("pzby-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut rand = Count(0);
        let h = PzbyHeader::load(&sample(&dir), &mut rand).unwrap();
        assert_eq!(rand.0, 1);
        assert_eq!(h.used_tile_names, ["floor_0"]);
        assert_eq!((h.width, h.height, h.levels), (1, 1, 1));
        assert_eq!(h.buildings[0].w(), 3);
        assert_eq!(h.buildings[0].max_level(&h.rooms), 0);
        let c = h.load_chunk(0, 0).unwrap();
        assert_eq!(c.attribute(1, 2, 0), 1);
        assert_eq!(c.tiles(1, 2, 0), Some(&[0][..]));
        assert_eq!(c.tiles(0, 0, 0), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
