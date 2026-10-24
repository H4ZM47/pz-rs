//! `chunkdata_X_Y.bin` reader/writer (`zombie.pot.POTChunkData`).
//!
//! At runtime the game hands these files to the native pathfinding library
//! (`MapCollisionData.n_initMetaCell`) without reading them in Java; the
//! only Java parser is the map converter's `POTChunkData`, ported here.
//! Format (big-endian `DataInputStream`): `short version`, then one record
//! per chunk in row-major order: a type byte, followed by `chunkDim²`
//! square bytes when the type is `REGULAR_CHUNK`.
//!
//! Shipped Build 42 files also use a chunk type byte 5, which
//! `POTChunkData` does not know and reads as a regular chunk (consuming 64
//! square bytes); this port keeps that behaviour, so it reproduces the Java
//! reader, not the native runtime's interpretation of those chunks.

use std::io;

pub const BIT_SOLID: u8 = 1;
pub const BIT_WALLN: u8 = 2;
pub const BIT_WALLW: u8 = 4;
pub const BIT_WATER: u8 = 8;
pub const BIT_ROOM: u8 = 16;

pub const EMPTY_CHUNK: usize = 0;
pub const SOLID_CHUNK: usize = 1;
pub const REGULAR_CHUNK: usize = 2;
pub const WATER_CHUNK: usize = 3;
pub const ROOM_CHUNK: usize = 4;
const NUM_CHUNK_TYPES: usize = 5;

/// `POTChunkData.FILE_VERSION`.
pub const FILE_VERSION: i16 = 1;

/// `POTChunkData.Chunk`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub counts: [i32; NUM_CHUNK_TYPES],
    pub bits: Option<Vec<u8>>,
    n_sqrs: i32,
    chunk_dim: i32,
}

fn type_of(bits: u8) -> usize {
    match bits {
        0 => EMPTY_CHUNK,
        1 => SOLID_CHUNK,
        8 => WATER_CHUNK,
        16 => ROOM_CHUNK,
        _ => REGULAR_CHUNK,
    }
}

impl Chunk {
    fn new(chunk_dim: i32) -> Chunk {
        let n_sqrs = chunk_dim * chunk_dim;
        let mut counts = [0; NUM_CHUNK_TYPES];
        counts[EMPTY_CHUNK] = n_sqrs;
        Chunk {
            counts,
            bits: None,
            n_sqrs,
            chunk_dim,
        }
    }

    fn load(&mut self, data: &mut &[u8]) -> io::Result<()> {
        self.counts = [0; NUM_CHUNK_TYPES];
        let ty = read_u8(data)?;
        match ty {
            0 | 1 | 3 | 4 => self.counts[ty as usize] = self.n_sqrs,
            _ => {
                // Any other type byte is read as a regular chunk
                // (the original only asserts `type == 2`).
                let mut bits = vec![0u8; self.n_sqrs as usize];
                for b in bits.iter_mut() {
                    *b = read_u8(data)?;
                    self.counts[type_of(*b)] += 1;
                }
                self.bits = Some(bits);
            }
        }
        Ok(())
    }

    fn save(&self, out: &mut Vec<u8>) {
        let ty = self.get_type();
        out.push(ty as u8);
        if ty == REGULAR_CHUNK {
            // `dos.write(null)` would throw upstream; a regular chunk always
            // has bits once loaded or edited.
            if let Some(bits) = &self.bits {
                out.extend_from_slice(bits);
            }
        }
    }

    /// `getBits(x, y)`.
    pub fn get_bits(&self, x: i32, y: i32) -> u8 {
        if self.counts[EMPTY_CHUNK] == self.n_sqrs {
            return 0;
        }
        if self.counts[SOLID_CHUNK] == self.n_sqrs {
            return BIT_SOLID;
        }
        if self.counts[WATER_CHUNK] == self.n_sqrs {
            return BIT_WATER;
        }
        if self.counts[ROOM_CHUNK] == self.n_sqrs {
            return BIT_ROOM;
        }
        self.bits
            .as_ref()
            .map_or(0, |b| b[(x + y * self.chunk_dim) as usize])
    }

    /// `setBits(x, y, bits)`; returns the old bits (or `bits` when the
    /// uniform type does not change).
    pub fn set_bits(&mut self, x: i32, y: i32, bits: u8) -> u8 {
        let bits_old = self.get_bits(x, y);
        let type_old = type_of(bits_old);
        let type_new = type_of(bits);
        if type_old == type_new && type_old != REGULAR_CHUNK {
            return bits;
        }
        self.counts[type_old] -= 1;
        self.counts[type_new] += 1;
        if self.get_type() == REGULAR_CHUNK {
            let n = self.n_sqrs as usize;
            let b = self.bits.get_or_insert_with(|| vec![bits_old; n]);
            b[(x + y * self.chunk_dim) as usize] = bits;
        } else {
            self.bits = None;
        }
        bits_old
    }

    /// `getType()`.
    pub fn get_type(&self) -> usize {
        for t in [EMPTY_CHUNK, SOLID_CHUNK, WATER_CHUNK, ROOM_CHUNK] {
            if self.counts[t] == self.n_sqrs {
                return t;
            }
        }
        REGULAR_CHUNK
    }
}

fn read_u8(data: &mut &[u8]) -> io::Result<u8> {
    let (&b, rest) = data
        .split_first()
        .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
    *data = rest;
    Ok(b)
}

/// `POTChunkData`: one cell's collision bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkData {
    /// `true` for 256x256 cells of 8x8 chunks, `false` for the old
    /// 300x300 cells of 10x10 chunks.
    pub pot: bool,
    pub chunk_dim: i32,
    pub chunks_per_cell: i32,
    pub cell_dim: i32,
    pub x: i32,
    pub y: i32,
    pub chunks: Vec<Chunk>,
}

impl ChunkData {
    pub fn new(x: i32, y: i32, pot: bool) -> ChunkData {
        let chunk_dim = if pot { 8 } else { 10 };
        let chunks_per_cell = if pot { 32 } else { 30 };
        let cell_dim = if pot { 256 } else { 300 };
        ChunkData {
            pot,
            chunk_dim,
            chunks_per_cell,
            cell_dim,
            x,
            y,
            chunks: vec![Chunk::new(chunk_dim); (chunks_per_cell * chunks_per_cell) as usize],
        }
    }

    /// `load(file)` from the file's bytes. The version short is read but
    /// not checked (the original only asserts it).
    pub fn load(&mut self, mut data: &[u8]) -> io::Result<i16> {
        let hi = read_u8(&mut data)?;
        let lo = read_u8(&mut data)?;
        let version = i16::from_be_bytes([hi, lo]);
        for chunk in &mut self.chunks {
            chunk.load(&mut data)?;
        }
        Ok(version)
    }

    /// `save(fileName)` as bytes.
    pub fn save(&self) -> Vec<u8> {
        let mut out = FILE_VERSION.to_be_bytes().to_vec();
        for chunk in &self.chunks {
            chunk.save(&mut out);
        }
        out
    }

    pub fn get_min_square_x(&self) -> i32 {
        self.x * self.cell_dim
    }
    pub fn get_min_square_y(&self) -> i32 {
        self.y * self.cell_dim
    }
    pub fn get_max_square_x(&self) -> i32 {
        (self.x + 1) * self.cell_dim - 1
    }
    pub fn get_max_square_y(&self) -> i32 {
        (self.y + 1) * self.cell_dim - 1
    }

    pub fn contains_square(&self, sx: i32, sy: i32) -> bool {
        sx >= self.get_min_square_x()
            && sx <= self.get_max_square_x()
            && sy >= self.get_min_square_y()
            && sy <= self.get_max_square_y()
    }

    fn chunk_index(&self, sx: i32, sy: i32) -> (usize, i32, i32) {
        let lx = sx - self.get_min_square_x();
        let ly = sy - self.get_min_square_y();
        let (cx, cy) = (lx / self.chunk_dim, ly / self.chunk_dim);
        (
            (cx + cy * self.chunks_per_cell) as usize,
            lx % self.chunk_dim,
            ly % self.chunk_dim,
        )
    }

    /// `getSquareBits(x, y)` in world square coordinates.
    pub fn get_square_bits(&self, sx: i32, sy: i32) -> u8 {
        if !self.contains_square(sx, sy) {
            return 0;
        }
        let (i, x, y) = self.chunk_index(sx, sy);
        self.chunks[i].get_bits(x, y)
    }

    /// `setSquareBits(x, y, bits)`; the square must be inside the cell.
    pub fn set_square_bits(&mut self, sx: i32, sy: i32, bits: u8) {
        let (i, x, y) = self.chunk_index(sx, sy);
        self.chunks[i].set_bits(x, y, bits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_uniform_and_regular_chunks() {
        let mut bytes = vec![0, 1];
        // chunk 0 regular: square (1,0) solid, (2,0) wall north, rest empty.
        bytes.push(2);
        let mut sq = vec![0u8; 64];
        sq[1] = BIT_SOLID;
        sq[2] = BIT_WALLN;
        bytes.extend(&sq);
        // chunk 1 water, chunk 2 room, the rest empty.
        bytes.push(3);
        bytes.push(4);
        bytes.extend(std::iter::repeat_n(0u8, 1024 - 3));
        let mut cd = ChunkData::new(2, 3, true);
        assert_eq!(cd.load(&bytes).unwrap(), 1);
        let (x0, y0) = (512, 768);
        assert_eq!(cd.get_square_bits(x0 + 1, y0), BIT_SOLID);
        assert_eq!(cd.get_square_bits(x0 + 2, y0), BIT_WALLN);
        assert_eq!(cd.get_square_bits(x0 + 3, y0), 0);
        assert_eq!(cd.get_square_bits(x0 + 9, y0 + 7), BIT_WATER);
        assert_eq!(cd.get_square_bits(x0 + 16, y0), BIT_ROOM);
        assert_eq!(cd.get_square_bits(x0 - 1, y0), 0);
        assert_eq!(cd.chunks[0].counts, [62, 1, 1, 0, 0]);
        assert_eq!(cd.save(), bytes);
        assert!(cd.load(&bytes[..100]).is_err());
    }

    #[test]
    fn set_bits_switches_representation() {
        let mut cd = ChunkData::new(0, 0, false);
        assert_eq!(cd.chunks.len(), 900);
        cd.set_square_bits(11, 0, BIT_WATER);
        assert_eq!(cd.chunks[1].get_type(), REGULAR_CHUNK);
        for y in 0..10 {
            for x in 0..10 {
                cd.set_square_bits(10 + x, y, BIT_WATER);
            }
        }
        assert_eq!(cd.chunks[1].get_type(), WATER_CHUNK);
        assert!(cd.chunks[1].bits.is_none());
        assert_eq!(cd.get_square_bits(15, 5), BIT_WATER);
    }
}
