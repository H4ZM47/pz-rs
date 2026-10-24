//! The lighting world: squares, chunks and the scrolling chunk map
//! (`LightingSquare`, `LightingChunk`/`ChunkLevel`, `LightingChunkMap`).
//!
//! Squares live in a per-state slab and are referred to by [`SqId`]. Freed
//! squares keep their last contents (like the native object pool), so a
//! stale neighbour link reads old data instead of failing.

use crate::model::{LightingObject, Transmission};

/// Index of a square in its state's slab.
pub(crate) type SqId = u32;

/// Neighbour slots, in the order of the native neighbour pointers.
pub(crate) mod nb {
    pub const W: usize = 0;
    pub const NW: usize = 1;
    pub const N: usize = 2;
    pub const NE: usize = 3;
    pub const E: usize = 4;
    pub const SE: usize = 5;
    pub const S: usize = 6;
    pub const SW: usize = 7;
    pub const UP: usize = 8;
    pub const DOWN: usize = 9;
}

/// Edge slots of [`Square::trans`].
pub(crate) mod edge {
    pub const W: usize = 0;
    pub const N: usize = 1;
    pub const E: usize = 2;
    pub const S: usize = 3;
}

#[derive(Clone, Debug)]
pub(crate) struct Square {
    /// Propagated sun/sky light, bytes 0..2 of the packed light level.
    pub sun: [u8; 3],
    /// Own colour maxed with the square below (`CalcMaxLightInfo`).
    pub max_down: [u8; 3],
    /// Own colour maxed with the square above.
    pub max_up: [u8; 3],
    pub x: i32,
    pub y: i32,
    /// Native level, stored as a signed byte.
    pub z: i8,
    /// Byte 3 of the packed light level (bytes 0..2 live in `sun`).
    pub level_hi: u8,
    pub flags: u16,
    pub open: u32,
    pub nb: [Option<SqId>; 10],
    pub vision_matrix: u32,
    /// Index into the per-level result arrays.
    pub idx: u16,
    pub building_id: i64,
    pub room_id: i64,
    pub objects: Vec<LightingObject>,
    pub trans: [Transmission; 4],
    // Previous outputs, for dirty tracking.
    pub prev_seen: u8,
    pub prev_can_see: u8,
    pub prev_could_see: u8,
    pub prev_target: i32,
    pub prev_rgb: [u8; 3],
    pub prev_level: u32,
    pub prev_dark: i32,
    pub prev_vert: [u32; 8],
    pub prev_light_count: i32,
}

impl Default for Square {
    fn default() -> Self {
        Square {
            sun: [0; 3],
            max_down: [0; 3],
            max_up: [0; 3],
            x: 0,
            y: 0,
            z: 0,
            level_hi: 0,
            flags: 0,
            open: 0,
            nb: [None; 10],
            vision_matrix: 0,
            idx: 0,
            building_id: 0,
            room_id: 0,
            objects: Vec::new(),
            trans: [Transmission::NONE; 4],
            prev_seen: 0,
            prev_can_see: 0,
            prev_could_see: 0,
            prev_target: 0,
            prev_rgb: [0; 3],
            prev_level: 0,
            prev_dark: 0,
            prev_vert: [0; 8],
            prev_light_count: 0,
        }
    }
}

impl Square {
    /// The packed 32-bit light level (`+0x18`).
    #[inline]
    pub fn level(&self) -> u32 {
        u32::from(self.sun[0])
            | (u32::from(self.sun[1]) << 8)
            | (u32::from(self.sun[2]) << 16)
            | (u32::from(self.level_hi) << 24)
    }

    #[inline]
    pub fn set_level(&mut self, v: u32) {
        self.sun = [v as u8, (v >> 8) as u8, (v >> 16) as u8];
        self.level_hi = (v >> 24) as u8;
    }

    /// Initial values `squareBeginUpdate` gives a fresh square.
    pub fn begin(&mut self, x: i32, y: i32, z: i32) {
        *self = Square::default();
        self.idx = 0xffff;
        self.x = x;
        self.y = y;
        self.z = z as i8;
        self.prev_rgb = [0xff; 3];
        self.prev_level = 0xffff_fc18;
        self.prev_dark = -1;
        self.prev_seen = 1;
        self.prev_can_see = 1;
        self.prev_could_see = 1;
        self.prev_target = -1000;
        self.prev_vert = [(-100i32) as u32; 8];
        self.prev_light_count = 0;
    }
}

/// The squares of one level of a chunk, indexed `x * 8 + y`.
pub(crate) type ChunkLevel = [Option<SqId>; 64];

#[derive(Clone, Debug)]
pub(crate) struct Chunk {
    pub wx: i32,
    pub wy: i32,
    pub min: i32,
    pub max: i32,
    /// `levels[z - min]`.
    pub levels: Vec<ChunkLevel>,
    /// Neighbour links not yet rebuilt.
    pub fresh: bool,
    /// Touched by an inactive room light; its lit flags are cleared on the
    /// next light-source rebuild.
    pub light_touched: bool,
    /// Lighting has been computed at least once.
    pub done: bool,
}

impl Chunk {
    pub fn new(wx: i32, wy: i32) -> Chunk {
        Chunk {
            wx,
            wy,
            min: 32,
            max: 32,
            levels: vec![[None; 64]],
            fresh: true,
            light_touched: false,
            done: false,
        }
    }

    #[inline]
    pub fn level(&self, z: i32) -> Option<&ChunkLevel> {
        if self.min <= z && z <= self.max {
            Some(&self.levels[(z - self.min) as usize])
        } else {
            None
        }
    }

    #[inline]
    pub fn level_mut(&mut self, z: i32) -> Option<&mut ChunkLevel> {
        if self.min <= z && z <= self.max {
            Some(&mut self.levels[(z - self.min) as usize])
        } else {
            None
        }
    }

    /// `LightingChunk::getSquare` (`x`, `y` are chunk-local).
    #[inline]
    pub fn square(&self, x: i32, y: i32, z: i32) -> Option<SqId> {
        if self.min <= z && ((x | y) as u32) < 8 && z <= self.max {
            self.levels[(z - self.min) as usize][(x * 8 + y) as usize]
        } else {
            None
        }
    }

    /// All squares of the chunk, in level order.
    pub fn all_squares(&self) -> impl Iterator<Item = SqId> + '_ {
        self.levels.iter().flat_map(|l| l.iter().flatten().copied())
    }
}

/// The grid of chunks a state covers (`LightingChunkMap`).
#[derive(Clone, Debug)]
pub(crate) struct ChunkMap {
    /// World chunk coordinates of the top-left chunk.
    pub x: i32,
    pub y: i32,
    /// Size in chunks.
    pub w: i32,
    pub h: i32,
    /// `chunks[cx * h + cy]`.
    pub chunks: Vec<Option<Box<Chunk>>>,
    /// Per chunk `done` flags as of the last hand-off, `[cy * w + cx]`.
    pub done: Vec<u8>,
    /// Set by scrolling or teleporting: everything is relinked and dirty.
    pub full_dirty: bool,
}

impl ChunkMap {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> ChunkMap {
        let n = (w.max(0) * h.max(0)) as usize;
        ChunkMap {
            x,
            y,
            w,
            h,
            chunks: (0..n).map(|_| None).collect(),
            done: vec![0; n],
            full_dirty: false,
        }
    }

    #[inline]
    pub fn at(&self, cx: i32, cy: i32) -> Option<&Chunk> {
        self.chunks[(cx * self.h + cy) as usize].as_deref()
    }

    #[inline]
    pub fn slot(&mut self, cx: i32, cy: i32) -> &mut Option<Box<Chunk>> {
        &mut self.chunks[(cx * self.h + cy) as usize]
    }

    /// `LightingChunkMap::getChunk` (world chunk coordinates).
    pub fn get_chunk_mut(&mut self, wx: i32, wy: i32) -> Option<&mut Chunk> {
        let cx = wx.wrapping_sub(self.x);
        let cy = wy.wrapping_sub(self.y);
        if cx >= 0 && cx < self.w && cy >= 0 && cy < self.h {
            self.slot(cx, cy).as_deref_mut()
        } else {
            None
        }
    }
}
