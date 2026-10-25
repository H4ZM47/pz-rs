//! `mcd::MapCollisionData`: the native collision grid behind
//! `zombie.MapCollisionData` (one byte of `BIT_*` flags per square, stored
//! per 8x8 chunk with uniform chunks compressed to a type), its
//! `chunkdata_X_Y.bin` files, and the native copy of the meta grid
//! (`IsoWorld::instance` in the library) holding each cell's map
//! `chunkdata` files and per-chunk zombie intensity.

use super::stream::{DataInput, DataOutput};
use super::wall_follower::WallFollower;
use super::GameState;
use std::io;
use std::path::PathBuf;

pub const BIT_SOLID: u8 = 1;
pub const BIT_WALLN: u8 = 2;
pub const BIT_WALLW: u8 = 4;
pub const BIT_WATER: u8 = 8;
pub const BIT_ROOM: u8 = 16;
/// Squares no map defines (`0x20`), as `chunkdata` files store them.
pub const BIT_NULL: u8 = 32;
/// The mask zombies treat as impassable: solid, water or null.
pub const BLOCK_MASK: u8 = 0x19;

/// `IsoMetaCell` of the native meta grid.
#[derive(Clone)]
pub struct NativeMetaCell {
    /// `chunkdata_X_Y.bin` of every map that has this cell, in mod order.
    pub chunkdata: Vec<PathBuf>,
    /// Zombie intensity per chunk (`x + y * 32`).
    pub intensity: [u8; 1024],
}

/// The native `IsoMetaGrid` filled by `n_initMetaGrid/Cell/Chunk`.
#[derive(Clone, Default)]
pub struct NativeMetaGrid {
    pub min_x: i32,
    pub min_y: i32,
    pub width: i32,
    pub height: i32,
    pub cells: Vec<Option<Box<NativeMetaCell>>>,
}

fn floor_f(v: f32) -> i32 {
    let t = v as i32;
    t - ((v < t as f32) as i32)
}

impl NativeMetaGrid {
    /// `n_initMetaGrid`.
    pub fn init(&mut self, min_x: i32, min_y: i32, width: i32, height: i32) {
        self.min_x = min_x;
        self.min_y = min_y;
        self.width = width;
        self.height = height;
        let n = (width.max(0) as usize) * (height.max(0) as usize);
        self.cells = vec![None; n];
    }

    fn index(&self, cx: i32, cy: i32) -> Option<usize> {
        let x = cx - self.min_x;
        let y = cy - self.min_y;
        (x >= 0 && y >= 0 && x < self.width && y < self.height).then(|| (x + self.width * y) as usize)
    }

    pub fn get_cell_data(&self, cx: i32, cy: i32) -> Option<&NativeMetaCell> {
        self.index(cx, cy).and_then(|i| self.cells[i].as_deref())
    }

    pub fn get_or_create_cell_data(&mut self, cx: i32, cy: i32) -> Option<&mut NativeMetaCell> {
        let i = self.index(cx, cy)?;
        Some(
            self.cells[i]
                .get_or_insert_with(|| Box::new(NativeMetaCell { chunkdata: Vec::new(), intensity: [0; 1024] })),
        )
    }

    /// `n_initMetaCell`.
    pub fn init_meta_cell(&mut self, cx: i32, cy: i32, file: Option<PathBuf>) {
        let Some(file) = file else { return };
        if let Some(c) = self.get_or_create_cell_data(cx, cy) {
            c.chunkdata.push(file);
        }
    }

    /// `n_initMetaChunk`.
    pub fn init_meta_chunk(&mut self, cx: i32, cy: i32, wx: i32, wy: i32, intensity: u8) {
        if let Some(c) = self.get_or_create_cell_data(cx, cy) {
            if wx < 32 && wy < 32 && (wx | wy) >= 0 {
                c.intensity[(wx + wy * 32) as usize] = intensity;
            }
        }
    }

    /// `IsoMetaGrid::getChunkData(wx, wy)`: the chunk's zombie intensity.
    pub fn get_chunk_data(&self, wx: i32, wy: i32) -> Option<u8> {
        let cx = floor_f((wx << 3) as f32 / 256.0);
        let cy = floor_f((wy << 3) as f32 / 256.0);
        let cell = self.get_cell_data(cx, cy)?;
        let x = wx.wrapping_sub(cx.wrapping_mul(32));
        let y = wy.wrapping_sub(cy.wrapping_mul(32));
        (x < 32 && y < 32 && (x | y) >= 0).then(|| cell.intensity[(x + y * 32) as usize])
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }
}

/// `mcd::Chunk`: per-type square counts plus the 64 square bytes when the
/// chunk is not uniform.
#[derive(Clone)]
pub struct McdChunk {
    /// Empty, solid, regular, water, room, null.
    pub counts: [u8; 6],
    pub squares: Vec<u8>,
}

pub fn type_of(b: u8) -> usize {
    match b {
        0 => 0,
        1 => 1,
        8 => 3,
        16 => 4,
        32 => 5,
        _ => 2,
    }
}

fn type_bits(t: usize) -> u8 {
    [0, 1, 0, 8, 16, 32][t]
}

impl McdChunk {
    fn empty() -> McdChunk {
        McdChunk { counts: [64, 0, 0, 0, 0, 0], squares: Vec::new() }
    }

    /// The uniform type, or `None` for a regular chunk.
    fn uniform(&self) -> Option<usize> {
        [0usize, 1, 3, 4, 5].into_iter().find(|&t| self.counts[t] == 64)
    }

    /// `getType()`.
    pub fn get_type(&self) -> usize {
        self.uniform().unwrap_or(2)
    }

    pub fn get_bits(&self, x: i32, y: i32) -> u8 {
        match self.uniform() {
            Some(t) => type_bits(t),
            None => self.squares[(x + y * 8) as usize],
        }
    }

    fn ensure_squares(&mut self) {
        if self.squares.len() != 64 {
            self.squares = vec![0; 64];
        }
    }

    /// `setBits`: returns the previous bits.
    pub fn set_bits(&mut self, x: i32, y: i32, bits: u8) -> u8 {
        let (old, old_type, old_uniform) = match self.uniform() {
            Some(t) => (type_bits(t), t, true),
            None => {
                let b = self.squares[(y * 8 + x) as usize];
                let t = type_of(b);
                (b, t, t != 2)
            }
        };
        let new_type = type_of(bits);
        if new_type == old_type && old_uniform && new_type != 2 {
            return old;
        }
        // (A regular-type square only returns early when both are uniform
        // types; two different regular bit values still get written.)
        let was_uniform = self.uniform().is_some();
        self.counts[old_type] = self.counts[old_type].wrapping_sub(1);
        self.counts[new_type] = self.counts[new_type].wrapping_add(1);
        if self.uniform().is_some() {
            self.squares.clear();
            return old;
        }
        if self.squares.len() != 64 {
            self.squares = vec![old; 64];
        } else if was_uniform {
            self.squares.fill(old);
        }
        self.squares[(y * 8 + x) as usize] = bits;
        old
    }

    /// `setData`: returns whether anything changed.
    pub fn set_data(&mut self, sq: &[u8; 64]) -> bool {
        let old_type = self.get_type();
        let mut c = [0i32; 6];
        let mut new_type = 2;
        let mut regular = false;
        for &b in sq.iter() {
            let t = type_of(b);
            if t == 2 {
                regular = true;
                break;
            }
            c[t] += 1;
        }
        if !regular {
            new_type = [0usize, 1, 3, 4, 5].into_iter().find(|&t| c[t] == 64).unwrap_or(2);
        }
        if old_type != 2 && old_type == new_type {
            return false;
        }
        self.counts = [0; 6];
        if new_type != 2 {
            self.squares.clear();
            self.counts[new_type] = 64;
            return true;
        }
        self.ensure_squares();
        self.squares.copy_from_slice(sq);
        for &b in sq.iter() {
            self.counts[type_of(b)] += 1;
        }
        true
    }

    /// `isBlocked(mask)`: every square has a bit of `mask`.
    pub fn is_blocked(&self, mask: u8) -> bool {
        match self.uniform() {
            Some(0) => false,
            Some(t) => type_bits(t) & mask != 0,
            None => self.squares.iter().all(|&b| b & mask != 0),
        }
    }

    /// `loadFromCellBits`.
    fn load_from_cell_bits(&mut self, x0: i32, y0: i32, buf: &[u8]) {
        self.counts = [0; 6];
        let base = (x0 + y0 * 256) as usize;
        for r in 0..8 {
            for c in 0..8 {
                self.counts[type_of(buf[base + r * 256 + c])] += 1;
            }
        }
        if self.uniform().is_some() {
            return;
        }
        self.ensure_squares();
        for r in 0..8 {
            for c in 0..8 {
                self.squares[r * 8 + c] = buf[base + r * 256 + c];
            }
        }
    }

    fn save(&self, out: &mut DataOutput) {
        match self.uniform() {
            Some(t) => out.write_byte(t as i32),
            None => {
                out.write_byte(2);
                for &b in &self.squares {
                    out.write_byte(b as i8 as i32);
                }
            }
        }
    }
}

/// `mcd::Cell`.
pub struct McdCell {
    pub x: i16,
    pub y: i16,
    pub chunks: Vec<McdChunk>,
    pub loaded: bool,
    pub has_data: bool,
    pub dirty: bool,
    pub last_access: i64,
}

impl McdCell {
    fn new(x: i32, y: i32) -> McdCell {
        McdCell {
            x: x as i16,
            y: y as i16,
            chunks: Vec::new(),
            loaded: false,
            has_data: false,
            dirty: false,
            last_access: 0,
        }
    }

    fn file_name(&self, gs: &GameState) -> PathBuf {
        gs.save_file(&gs.subdir_chunkdata, &format!("chunkdata_{}_{}.bin", self.x, self.y))
    }

    /// `Cell::load(path)`: reads a `chunkdata` file into the chunks.
    fn load_file(&mut self, path: &std::path::Path) -> bool {
        let Ok(mut inp) = DataInput::open(path) else { return false };
        self.read_chunks(&mut inp).is_ok()
    }

    fn read_chunks(&mut self, inp: &mut DataInput) -> io::Result<()> {
        let version = inp.read_short()? as u16;
        if version > 1 {
            return Err(io::Error::other(format!("unknown file version {version}")));
        }
        for i in 0..1024 {
            let ch = &mut self.chunks[i];
            ch.counts = [0; 6];
            let t = inp.read_byte()? as u8;
            match t {
                0 | 1 | 3 | 4 | 5 => ch.counts[t as usize] = 64,
                _ => {
                    ch.ensure_squares();
                    for k in 0..64 {
                        let b = inp.read_byte()? as u8;
                        ch.squares[k] = b;
                        ch.counts[type_of(b)] += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// `Cell::loadNonNullSquares(path, buf)`.
    fn load_non_null_squares(path: &std::path::Path, buf: &mut [u8]) -> bool {
        let Ok(mut inp) = DataInput::open(path) else { return false };
        Self::merge_non_null(&mut inp, buf).is_ok()
    }

    fn merge_non_null(inp: &mut DataInput, buf: &mut [u8]) -> io::Result<()> {
        let version = inp.read_short()? as u16;
        if version > 1 {
            return Err(io::Error::other(format!("unknown file version {version}")));
        }
        for cy in 0..32usize {
            for cx in 0..32usize {
                let t = inp.read_byte()? as u8;
                let fill = match t {
                    0 => Some(0u8),
                    1 => Some(1),
                    3 => Some(8),
                    4 => Some(16),
                    5 => None,
                    _ => {
                        for u in 0..64usize {
                            let b = inp.read_byte()? as u8;
                            let i = (cy * 8 + (u >> 3)) * 256 + cx * 8 + (u & 7);
                            if buf[i] == BIT_NULL {
                                buf[i] = b;
                            }
                        }
                        continue;
                    }
                };
                if let Some(v) = fill {
                    for r in 0..8 {
                        for c in 0..8 {
                            let i = (cy * 8 + r) * 256 + cx * 8 + c;
                            if buf[i] == BIT_NULL {
                                buf[i] = v;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// `Cell::load()`.
    fn load(&mut self, gs: &GameState, meta: &NativeMetaGrid) {
        self.loaded = true;
        self.chunks = vec![McdChunk::empty(); 1024];
        let path = self.file_name(gs);
        self.has_data = self.load_file(&path);
        if self.has_data {
            return;
        }
        let Some(mc) = meta.get_cell_data(self.x as i32, self.y as i32) else { return };
        match mc.chunkdata.len() {
            0 => {}
            1 => self.has_data = self.load_file(&mc.chunkdata[0]),
            _ => {
                let mut buf = vec![BIT_NULL; 0x10000];
                for f in &mc.chunkdata {
                    let ok = Self::load_non_null_squares(f, &mut buf);
                    self.has_data |= ok;
                }
                for cy in 0..32 {
                    for cx in 0..32 {
                        self.chunks[cy * 32 + cx].load_from_cell_bits(cx as i32 * 8, cy as i32 * 8, &buf);
                    }
                }
                self.has_data = true;
            }
        }
    }

    /// `Cell::save()`.
    fn save(&self, gs: &GameState) {
        if !(self.loaded && self.has_data && !gs.no_save) {
            return;
        }
        let mut out = DataOutput::default();
        out.write_short(1);
        for ch in &self.chunks {
            ch.save(&mut out);
        }
        let _ = out.save(&self.file_name(gs));
    }

    /// `Cell::unload()`.
    fn unload(&mut self, gs: &GameState) {
        if self.dirty {
            self.dirty = false;
            self.save(gs);
        }
        self.chunks.clear();
        self.loaded = false;
    }

    fn chunk_index(&self, x: i32, y: i32) -> usize {
        ((((y >> 3) & 31) << 5) | ((x >> 3) & 31)) as usize
    }
}

/// A queued `findPath2` request.
#[derive(Clone, Copy, Debug)]
pub struct PathTask {
    pub sx: i32,
    pub sy: i32,
    pub ex: i32,
    pub ey: i32,
    /// The popman `RepopulateTask` waiting for the result.
    pub task: u32,
}

/// One `mcd::PassToWorker`: chunk and square updates from the main thread.
#[derive(Default, Clone)]
pub struct McdPassToWorker {
    pub chunks: Vec<(i32, i32, [u8; 64])>,
    pub squares: Vec<(i32, i32, u8)>,
}

impl McdPassToWorker {
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty() && self.squares.is_empty()
    }
}

/// `mcd::MapCollisionData`.
#[derive(Default)]
pub struct Mcd {
    pub min_x: i32,
    pub min_y: i32,
    pub width: i32,
    pub height: i32,
    pub cells: Vec<McdCell>,
    /// Loaded cells (indices into `cells`) in load order.
    pub loaded: Vec<usize>,
    /// Pending `findPath2` requests (taken from the back).
    pub path_tasks: Vec<PathTask>,
    pub current: Option<PathTask>,
    pub wall_follower: Box<WallFollower>,
    /// `mcd::passToWorkerQ` and the main thread's current batch.
    pub queue: std::collections::VecDeque<McdPassToWorker>,
    pub pending: McdPassToWorker,
}

impl Mcd {
    /// `MapCollisionData::init`.
    pub fn init(&mut self, min_x: i32, min_y: i32, width: i32, height: i32) {
        self.min_x = min_x;
        self.min_y = min_y;
        self.width = width;
        self.height = height;
        self.cells.clear();
        for y in 0..height.max(0) {
            for x in 0..width.max(0) {
                self.cells.push(McdCell::new(min_x + x, min_y + y));
            }
        }
    }

    fn cell_index_world(&self, x: i32, y: i32) -> Option<usize> {
        let ux = x.wrapping_sub(self.min_x.wrapping_mul(256));
        if ux < 0 {
            return None;
        }
        let uy = y.wrapping_sub(self.min_y.wrapping_mul(256));
        if uy < 0 {
            return None;
        }
        let cx = ux >> 8;
        let cy = uy >> 8;
        if cx >= self.width || cy >= self.height {
            return None;
        }
        Some((cx + self.width * cy) as usize)
    }

    fn cell_index_chunk(&self, wx: i32, wy: i32) -> Option<usize> {
        self.cell_index_world(wx.wrapping_mul(8), wy.wrapping_mul(8))
    }

    /// `loadIfNeeded`: touches the cell and loads it, first unloading idle
    /// cells when more than 20 are loaded.
    pub fn load_if_needed(&mut self, ci: usize, gs: &GameState, meta: &NativeMetaGrid) {
        self.cells[ci].last_access = gs.now();
        if self.cells[ci].loaded {
            return;
        }
        if self.loaded.len() > 20 {
            let now = gs.now();
            let mut budget = self.loaded.len() as i32 - 20;
            let mut i = 0usize;
            while i < self.loaded.len() {
                let c = self.loaded[i];
                if self.cells[c].last_access + 1000 < now {
                    self.cells[c].unload(gs);
                    self.loaded.remove(i);
                    budget -= 1;
                    if budget == 0 {
                        break;
                    }
                    continue;
                }
                i += 1;
            }
        }
        self.cells[ci].load(gs, meta);
        self.loaded.push(ci);
    }

    /// `getBits(x, y)`: 1 (solid) outside the grid.
    pub fn get_bits(&mut self, x: i32, y: i32, gs: &GameState, meta: &NativeMetaGrid) -> u8 {
        let Some(ci) = self.cell_index_world(x, y) else { return 1 };
        self.load_if_needed(ci, gs, meta);
        let cell = &self.cells[ci];
        if !cell.loaded {
            return 0;
        }
        let ch = &cell.chunks[cell.chunk_index(x, y)];
        ch.get_bits(x & 7, y & 7)
    }

    /// `isBlocked(x1, y1, x2, y2, mask)`: whether moving from one square to
    /// an adjacent one is blocked.
    #[allow(clippy::too_many_arguments)]
    pub fn is_blocked(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        mask: u8,
        gs: &GameState,
        meta: &NativeMetaGrid,
    ) -> bool {
        let b1 = self.get_bits(x1, y1, gs, meta);
        let b2 = self.get_bits(x2, y2, gs, meta);
        if b2 & mask != 0 {
            return true;
        }
        let n = y2 < y1 && b1 & BIT_WALLN != 0;
        let s = y1 < y2 && b2 & BIT_WALLN != 0;
        let w = x2 < x1 && b1 & BIT_WALLW != 0;
        let e = x1 < x2 && b2 & BIT_WALLW != 0;
        if e || n || w || s {
            return true;
        }
        if x1 != x2 && y1 != y2 {
            return self.is_blocked(x1, y1, x1, y2, mask, gs, meta)
                || self.is_blocked(x1, y1, x2, y1, mask, gs, meta)
                || self.is_blocked(x2, y2, x1, y2, mask, gs, meta)
                || self.is_blocked(x2, y2, x2, y1, mask, gs, meta);
        }
        false
    }

    /// `isChunkBlocked(wx, wy, mask)`: true outside the grid.
    pub fn is_chunk_blocked(&mut self, wx: i32, wy: i32, mask: u8, gs: &GameState, meta: &NativeMetaGrid) -> bool {
        if self.cells.is_empty() {
            return true;
        }
        let Some(ci) = self.cell_index_chunk(wx, wy) else { return true };
        self.load_if_needed(ci, gs, meta);
        let cell = &self.cells[ci];
        let lx = wx.wrapping_sub(cell.x as i32 * 32) as u32;
        let ly = wy.wrapping_sub(cell.y as i32 * 32) as u32;
        if cell.loaded && lx < 32 && ly < 32 {
            return cell.chunks[(lx | (ly * 32)) as usize].is_blocked(mask);
        }
        false
    }

    /// `findPath2`: queues a wall-follower path for the worker.
    pub fn find_path2(&mut self, sx: i32, sy: i32, ex: i32, ey: i32, task: u32) {
        self.path_tasks.push(PathTask { sx, sy, ex, ey, task });
    }

    /// Applies one queued batch of chunk/square updates (first half of
    /// `MapCollisionData::update`).
    pub fn apply_batch(&mut self, b: McdPassToWorker, gs: &GameState, meta: &NativeMetaGrid) {
        for (wx, wy, sq) in b.chunks.iter() {
            let Some(ci) = self.cell_index_chunk(*wx, *wy) else { continue };
            if self.cells.is_empty() {
                continue;
            }
            self.load_if_needed(ci, gs, meta);
            let cell = &mut self.cells[ci];
            if !cell.loaded {
                continue;
            }
            if cell.x as i32 * 32 <= *wx && cell.y as i32 * 32 <= *wy {
                let idx = ((*wx & 0x1f) | ((*wy & 0x1f) << 5)) as usize;
                if cell.chunks[idx].set_data(sq) {
                    cell.dirty = true;
                }
            }
        }
        for &(x, y, bits) in b.squares.iter() {
            let Some(ci) = self.cell_index_world(x, y) else { continue };
            if self.cells.is_empty() {
                continue;
            }
            self.load_if_needed(ci, gs, meta);
            let cell = &mut self.cells[ci];
            if !cell.loaded {
                continue;
            }
            let lx = x - cell.x as i32 * 256;
            let ly = y - cell.y as i32 * 256;
            if lx < 0 || ly < 0 {
                continue;
            }
            let idx = (((ly * 4) & 0x7fff_ffe0) + (lx >> 3)) as usize;
            if cell.chunks[idx].set_bits(x & 7, y & 7, bits) != bits {
                cell.dirty = true;
            }
        }
    }

    /// The last step of `update`: unloads at most one cell idle for 5 s.
    pub fn unload_idle(&mut self, gs: &GameState) {
        let now = gs.now();
        for i in 0..self.loaded.len() {
            let c = self.loaded[i];
            if self.cells[c].last_access + 5000 < now {
                self.cells[c].unload(gs);
                self.loaded.remove(i);
                return;
            }
        }
    }

    /// `MapCollisionData::save`.
    pub fn save(&mut self, gs: &GameState) {
        for &c in &self.loaded {
            let cell = &mut self.cells[c];
            if cell.dirty {
                cell.dirty = false;
                cell.save(gs);
            }
        }
    }

    /// `MapCollisionData::stop`.
    pub fn stop(&mut self, gs: &GameState) {
        self.pending = McdPassToWorker::default();
        for &c in &self.loaded {
            self.cells[c].dirty = false;
            self.cells[c].unload(gs);
        }
        self.loaded.clear();
        self.cells.clear();
        self.path_tasks.clear();
        self.current = None;
    }

    /// `hasDataForThread` (main thread): queues the current batch.
    pub fn has_data_for_thread(&mut self) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        let b = std::mem::take(&mut self.pending);
        self.queue.push_back(b);
        true
    }

    /// `shouldWait`.
    pub fn should_wait(&self) -> bool {
        self.path_tasks.is_empty() && self.current.is_none() && self.queue.is_empty()
    }

    /// `n_chunkUpdateTask`.
    pub fn chunk_update_task(&mut self, wx: i32, wy: i32, squares: &[u8; 64]) {
        self.pending.chunks.push((wx, wy, *squares));
    }

    /// `n_squareUpdateTask` for one square.
    pub fn square_update_task(&mut self, x: i32, y: i32, bits: u8) {
        self.pending.squares.push((x, y, bits));
    }

    /// The loaded chunk's raw state, for dumps and tests.
    pub fn chunk_at(&self, wx: i32, wy: i32) -> Option<&McdChunk> {
        let ci = self.cell_index_chunk(wx, wy)?;
        let cell = self.cells.get(ci)?;
        if !cell.loaded {
            return None;
        }
        cell.chunks.get(((wx & 31) | ((wy & 31) << 5)) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_set_bits_tracks_counts() {
        let mut c = McdChunk::empty();
        assert_eq!(c.get_type(), 0);
        assert_eq!(c.set_bits(1, 2, BIT_SOLID), 0);
        assert_eq!(c.get_type(), 2);
        assert_eq!(c.get_bits(1, 2), BIT_SOLID);
        assert_eq!(c.get_bits(0, 0), 0);
        assert_eq!(c.set_bits(1, 2, 0), BIT_SOLID);
        assert_eq!(c.get_type(), 0);
        assert!(c.squares.is_empty());
        assert!(!c.is_blocked(BLOCK_MASK));
        let all = [BIT_WATER; 64];
        assert!(c.set_data(&all));
        assert_eq!(c.get_type(), 3);
        assert!(c.is_blocked(BLOCK_MASK));
        assert!(!c.set_data(&all));
    }
}
