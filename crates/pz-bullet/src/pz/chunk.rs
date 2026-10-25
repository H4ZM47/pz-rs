//! `ChunkMap`, `Chunk` and `ChunkLevel`: the static collision the game
//! streams in per chunk (8×8 squares) and level.
//!
//! The Java side sends up to four shape codes per square through
//! `ToBullet` (`IsoChunk.updatePhysicsForLevel`): the `PhysicsShapes`
//! ordinal plus one, so 1 solid, 2–5 walls N/W/S/E, 6 tree, 7 floor,
//! 8/9 stairs (middle N/W), 10 solid stairs, 11+ physics mesh `n - 11`.
//! Bodies are created when a vehicle needs the level (`ChunkMap::setNeed`)
//! and removed when no vehicle does.

use crate::bt::collision_object::ObjId;

/// Index of a chunk in the world's chunk arena.
pub type ChunkId = usize;

/// `ChunkLevel` (one level of one chunk).
#[derive(Clone, Debug)]
pub struct ChunkLevel {
    pub level: i32,
    /// `+0x0c`: shapes changed since the bodies were built.
    pub dirty: bool,
    /// `+0x0d`: bodies are in the world.
    pub has_bodies: bool,
    /// `+0x0e`: a vehicle needs this level this step.
    pub need: bool,
    /// `+0x10`: shape codes per square, `[x][y][slot]`.
    pub shapes: [[[u32; 4]; 8]; 8],
    /// `+0x410`: per-slot bodies (solid, tree, solid stairs, mesh).
    pub bodies: [[[Option<ObjId>; 4]; 8]; 8],
    /// `+0xc10`: merged floor bodies (one per merged rectangle, at its
    /// first square).
    pub floor: [[Option<ObjId>; 8]; 8],
    /// `+0xe10`..`+0x1410`: merged wall bodies N, W, S, E.
    pub walls: [[[Option<ObjId>; 8]; 8]; 4],
    /// `+0x1610`: stairs collision objects.
    pub stairs: [[Option<ObjId>; 8]; 8],
}

impl ChunkLevel {
    /// `ChunkLevel::alloc()` + `init(chunk, level)`: everything cleared.
    pub fn new(level: i32) -> ChunkLevel {
        ChunkLevel {
            level,
            dirty: false,
            has_bodies: false,
            need: false,
            shapes: [[[0; 4]; 8]; 8],
            bodies: [[[None; 4]; 8]; 8],
            floor: [[None; 8]; 8],
            walls: [[[None; 8]; 8]; 4],
            stairs: [[None; 8]; 8],
        }
    }

    /// `ChunkLevel::setShapes(x, y, count, shapes)`: the first `count`
    /// (up to 4) slots get the codes, the rest are cleared.
    pub fn set_shapes(&mut self, x: usize, y: usize, shapes: &[u32]) {
        let s = &mut self.shapes[x][y];
        for (i, slot) in s.iter_mut().enumerate() {
            *slot = if i < shapes.len() { shapes[i] } else { 0 };
        }
        self.dirty = true;
    }
}

/// `Chunk` (a chunk column with its levels).
#[derive(Clone, Debug)]
pub struct Chunk {
    pub wx: i32,
    pub wy: i32,
    pub min_level: i32,
    pub max_level: i32,
    /// Levels `min_level..=max_level`.
    pub levels: Vec<ChunkLevel>,
    /// `+0x18`: referenced by chunk map 0..3.
    pub referenced: [bool; 4],
}

impl Chunk {
    /// `Chunk(wx, wy)`: one empty level 0.
    pub fn new(wx: i32, wy: i32) -> Chunk {
        Chunk {
            wx,
            wy,
            min_level: 0,
            max_level: 0,
            levels: vec![ChunkLevel::new(0)],
            referenced: [false; 4],
        }
    }
    pub fn is_referenced(&self) -> bool {
        self.referenced.iter().any(|&r| r)
    }
    /// `Chunk::getLevelData(level)`.
    pub fn level(&self, level: i32) -> Option<&ChunkLevel> {
        if self.min_level <= level && level <= self.max_level {
            self.levels.get((level - self.min_level) as usize)
        } else {
            None
        }
    }
    pub fn level_mut(&mut self, level: i32) -> Option<&mut ChunkLevel> {
        if self.min_level <= level && level <= self.max_level {
            self.levels.get_mut((level - self.min_level) as usize)
        } else {
            None
        }
    }
    /// `Chunk::setNeed(level)`.
    pub fn set_need(&mut self, level: i32) {
        if let Some(l) = self.level_mut(level) {
            l.need = true;
        }
    }
}

/// `ChunkMap`: one player's window of `width × width` chunks.
#[derive(Clone, Debug)]
pub struct ChunkMap {
    pub player_index: i32,
    pub wx: i32,
    pub wy: i32,
    pub width: i32,
    /// `grid[x * width + y]`.
    pub grid: Vec<Option<ChunkId>>,
}

impl ChunkMap {
    /// `ChunkMap(playerIndex, wx, wy, width)`.
    pub fn new(player_index: i32, wx: i32, wy: i32, width: i32) -> ChunkMap {
        let n = (width.max(0) * width.max(0)) as usize;
        ChunkMap {
            player_index,
            wx,
            wy,
            width,
            grid: vec![None; n],
        }
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<ChunkId> {
        self.grid[(x * self.width + y) as usize]
    }
    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: Option<ChunkId>) {
        let w = self.width;
        self.grid[(x * w + y) as usize] = c;
    }
    /// Chunk at world chunk coordinates, if inside the window.
    pub fn chunk_at(&self, wx: i32, wy: i32) -> Option<ChunkId> {
        if self.wx <= wx && wx < self.wx + self.width && self.wy <= wy && wy < self.wy + self.width
        {
            self.get(wx - self.wx, wy - self.wy)
        } else {
            None
        }
    }
    pub fn contains(&self, wx: i32, wy: i32) -> bool {
        self.wx <= wx && wx < self.wx + self.width && self.wy <= wy && wy < self.wy + self.width
    }
}
