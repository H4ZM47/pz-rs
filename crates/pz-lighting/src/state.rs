//! `LightingState`: one player's lighting world, its queued map changes,
//! and the per-update pipeline (`applyMapChanges`,
//! `LightingState::DoLightingUpdateNew`).

use std::collections::BTreeSet;

use crate::model::{fastfloor, flags, open, Character, NUM_LEVELS};
use crate::results::ResultLevel;
use crate::session::Globals;
use crate::world::{nb, Chunk, ChunkMap, SqId, Square};

/// Size of the line-of-sight cache: 200 x 200 squares x 32 levels.
pub(crate) const LOS_SIZE: usize = 200 * 200 * 32;

/// Parameters of `stateEndFrame`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct EndFrame {
    pub rmod: f32,
    pub gmod: f32,
    pub bmod: f32,
    pub ambient: f32,
    pub night: f32,
    pub view_dist: f32,
    pub view_dist_max: f32,
    pub cache_cleared: bool,
    pub light_source_update: f32,
    /// Number of `stateEndFrame` calls merged into this change.
    pub frames: i32,
    pub sky: u32,
    /// Highest player index that had a state when the change was made.
    pub max_player: i32,
}

/// A queued main-thread change (`MapChange`).
#[derive(Debug)]
pub(crate) enum MapChange {
    Left,
    Right,
    Up,
    Down,
    Chunk { chunk: Box<Chunk>, updated: [bool; NUM_LEVELS] },
    Player(Box<Character>),
    /// Light ids (indices into the global light slab).
    Lights { add: Vec<usize>, remove: Vec<usize> },
    Teleport { x: i32, y: i32 },
    EndFrame(EndFrame),
}

/// Per-square torch accumulation (`TorchLight`, 16 bytes per square).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TorchCell {
    pub torch: Option<usize>,
    pub dist: f32,
    pub rgb: [u8; 3],
}

pub(crate) struct LightingState {
    pub player: i32,
    /// World square coordinates of result index 0.
    pub ox: i32,
    pub oy: i32,
    /// Size in squares.
    pub w: i32,
    pub h: i32,
    /// Level range of the last update, `[min_level, max_level)`.
    pub max_level: i32,
    pub min_level: i32,
    pub chr: Option<Box<Character>>,
    pub chr_square: Option<SqId>,
    pub ambient: [f32; 4],
    pub sky: u32,
    pub map: ChunkMap,
    pub dirty_global: bool,
    /// Point lights handed to this state (global light ids).
    pub lights: Vec<usize>,
    pub room_lights: Vec<usize>,
    pub torches: Vec<usize>,
    pub lights_changed: bool,
    pub time: [u64; 2],
    /// Changes queued by the main thread.
    pub queue: Vec<MapChange>,
    /// Changes handed to the update.
    pub pending: Vec<MapChange>,
    /// The update finished and waits for the main thread.
    pub done: bool,
    /// Work has been handed to the update at least once.
    pub work: bool,
    pub los: Vec<u8>,
    pub cache_cleared: bool,
    pub last_square: [i32; 3],
    pub light_source_update: f32,
    pub temp_lights_changed: bool,
    pub frames: i32,
    pub in_update_light_source: bool,
    pub front: Vec<ResultLevel>,
    pub back: Vec<ResultLevel>,
    pub torch_cells: Vec<TorchCell>,
    pub visible_rooms: BTreeSet<u64>,
    pub visible_rooms_work: BTreeSet<u64>,
    pub update_counter: i32,
    pub squares: Vec<Square>,
    pub free_squares: Vec<SqId>,
    /// Sky-seed flood order for the next full relight (see
    /// `LightingSession::debug_set_sky_seed_order`).
    pub seed_order: Option<Vec<(i32, i32, i32)>>,
}

impl LightingState {
    /// `LightingState::LightingState(player, chunkX, chunkY, w, h)`.
    pub fn new(player: i32, cx: i32, cy: i32, w: i32, h: i32) -> LightingState {
        let map = ChunkMap::new(cx, cy, w, h);
        let ox = map.x << 3;
        let oy = map.y << 3;
        let sw = w << 3;
        let sh = h << 3;
        LightingState {
            player,
            ox,
            oy,
            w: sw,
            h: sh,
            max_level: 0,
            min_level: 0,
            chr: None,
            chr_square: None,
            ambient: [0.0; 4],
            sky: 0,
            map,
            dirty_global: true,
            lights: Vec::new(),
            room_lights: Vec::new(),
            torches: Vec::new(),
            lights_changed: true,
            time: [0; 2],
            queue: Vec::new(),
            pending: Vec::new(),
            done: false,
            work: false,
            los: vec![0; LOS_SIZE],
            cache_cleared: false,
            last_square: [-1; 3],
            light_source_update: 100.0,
            temp_lights_changed: false,
            frames: 0,
            in_update_light_source: false,
            // The native constructor initialises the front buffers with the
            // x origin for both axes.
            front: (0..NUM_LEVELS).map(|_| ResultLevel::new(ox, ox)).collect(),
            back: (0..NUM_LEVELS).map(|_| ResultLevel::new(ox, oy)).collect(),
            torch_cells: vec![TorchCell::default(); (sw.max(0) * sh.max(0)) as usize],
            visible_rooms: BTreeSet::new(),
            visible_rooms_work: BTreeSet::new(),
            update_counter: 0,
            squares: Vec::new(),
            free_squares: Vec::new(),
            seed_order: None,
        }
    }

    // ---------------------------------------------------------------
    // Square slab

    pub fn alloc_square(&mut self) -> SqId {
        if let Some(id) = self.free_squares.pop() {
            id
        } else {
            self.squares.push(Square::default());
            (self.squares.len() - 1) as SqId
        }
    }

    pub fn free_square(&mut self, id: SqId) {
        self.squares[id as usize].objects.clear();
        self.free_squares.push(id);
    }

    pub fn free_chunk(&mut self, chunk: &Chunk) {
        for id in chunk.all_squares() {
            self.free_square(id);
        }
    }

    #[inline]
    pub fn sq(&self, id: SqId) -> &Square {
        &self.squares[id as usize]
    }

    #[inline]
    pub fn sq_mut(&mut self, id: SqId) -> &mut Square {
        &mut self.squares[id as usize]
    }

    /// `LightingState::getSquare`, world square coordinates.
    #[inline]
    pub fn get_square(&self, x: i32, y: i32, z: i32) -> Option<SqId> {
        let lx = x.wrapping_sub(self.ox);
        let ly = y.wrapping_sub(self.oy);
        if lx < 0 || lx >= self.w || ly < 0 || ly >= self.h {
            return None;
        }
        self.map.at(lx >> 3, ly >> 3)?.square(lx & 7, ly & 7, z)
    }

    // ---------------------------------------------------------------
    // Chunk map edits

    fn drop_chunk(&mut self, slot: Option<Box<Chunk>>) {
        if let Some(c) = slot {
            self.free_chunk(&c);
        }
    }

    pub fn scroll_left(&mut self) {
        let (w, h) = (self.map.w, self.map.h);
        if h > 0 && w > 0 {
            for cy in 0..h {
                let old = self.map.slot(w - 1, cy).take();
                self.drop_chunk(old);
                for cx in (1..w).rev() {
                    let c = self.map.slot(cx - 1, cy).take();
                    *self.map.slot(cx, cy) = c;
                }
            }
        }
        self.map.x -= 1;
        self.map.full_dirty = true;
    }

    pub fn scroll_right(&mut self) {
        let (w, h) = (self.map.w, self.map.h);
        if h > 0 && w > 0 {
            for cy in 0..h {
                let old = self.map.slot(0, cy).take();
                self.drop_chunk(old);
                for cx in 0..w - 1 {
                    let c = self.map.slot(cx + 1, cy).take();
                    *self.map.slot(cx, cy) = c;
                }
            }
        }
        self.map.x += 1;
        self.map.full_dirty = true;
    }

    pub fn scroll_up(&mut self) {
        let (w, h) = (self.map.w, self.map.h);
        if w > 0 && h > 0 {
            for cx in 0..w {
                let old = self.map.slot(cx, h - 1).take();
                self.drop_chunk(old);
                for cy in (1..h).rev() {
                    let c = self.map.slot(cx, cy - 1).take();
                    *self.map.slot(cx, cy) = c;
                }
            }
        }
        self.map.y -= 1;
        self.map.full_dirty = true;
    }

    pub fn scroll_down(&mut self) {
        let (w, h) = (self.map.w, self.map.h);
        if w > 0 && h > 0 {
            for cx in 0..w {
                let old = self.map.slot(cx, 0).take();
                self.drop_chunk(old);
                for cy in 0..h - 1 {
                    let c = self.map.slot(cx, cy + 1).take();
                    *self.map.slot(cx, cy) = c;
                }
            }
        }
        self.map.y += 1;
        self.map.full_dirty = true;
    }

    pub fn teleport(&mut self, x: i32, y: i32) {
        for i in 0..self.map.chunks.len() {
            let c = self.map.chunks[i].take();
            self.drop_chunk(c);
        }
        self.map.x = x;
        self.map.y = y;
        self.map.full_dirty = true;
    }

    /// `LightingChunkMap::setChunk`.
    fn set_chunk(&mut self, mut chunk: Box<Chunk>, updated: &[bool; NUM_LEVELS]) {
        let cx = chunk.wx.wrapping_sub(self.map.x);
        let cy = chunk.wy.wrapping_sub(self.map.y);
        if cx < 0 || cx >= self.map.w || cy < 0 || cy >= self.map.h {
            self.free_chunk(&chunk);
            return;
        }
        if let Some(mut old) = self.map.slot(cx, cy).take() {
            chunk.light_touched = old.light_touched;
            chunk.done = old.done;
            for z in chunk.min..=chunk.max {
                let Some(old_level) = old.level_mut(z) else { continue };
                if !updated[z as usize & 63] {
                    let moved = std::mem::replace(old_level, [None; 64]);
                    let new_level = chunk.level_mut(z).expect("level in range");
                    let replaced = std::mem::replace(new_level, moved);
                    for id in replaced.iter().flatten() {
                        self.free_square(*id);
                    }
                } else {
                    let old_level = *old_level;
                    let new_level = *chunk.level(z).expect("level in range");
                    for i in 0..64 {
                        if let (Some(o), Some(n)) = (old_level[i], new_level[i]) {
                            let (lv, fl) = {
                                let s = self.sq(o);
                                (s.level(), s.flags)
                            };
                            let s = self.sq_mut(n);
                            s.set_level(lv);
                            s.flags = (s.flags & 0xcfff) | (fl & 0x3000);
                        }
                    }
                }
            }
            self.free_chunk(&old);
        }
        *self.map.slot(cx, cy) = Some(chunk);
    }

    /// `LightingSquare::fromSquare2`: links the neighbours and builds the
    /// open-edge mask.
    pub fn link_square(&mut self, id: SqId) {
        let (x, y, z) = {
            let s = self.sq(id);
            (s.x, s.y, i32::from(s.z))
        };
        let mut n = [None; 10];
        n[nb::W] = self.get_square(x - 1, y, z);
        n[nb::E] = self.get_square(x + 1, y, z);
        n[nb::N] = self.get_square(x, y - 1, z);
        n[nb::S] = self.get_square(x, y + 1, z);
        n[nb::NW] = self.get_square(x - 1, y - 1, z);
        n[nb::SW] = self.get_square(x - 1, y + 1, z);
        n[nb::NE] = self.get_square(x + 1, y - 1, z);
        n[nb::SE] = self.get_square(x + 1, y + 1, z);
        n[nb::UP] = self.get_square(x, y, z + 1);
        n[nb::DOWN] = self.get_square(x, y, z - 1);
        let f = self.sq(id).flags;
        let fl = |o: Option<SqId>| o.map(|o| self.sq(o).flags);
        let mut mask = 0u32;
        if let Some(of) = fl(n[nb::W]) {
            if f & flags::W != 0 || of & flags::E != 0 {
                mask |= open::W;
            }
        }
        if let Some(of) = fl(n[nb::E]) {
            if f & flags::E != 0 || of & flags::W != 0 {
                mask |= open::E;
            }
        }
        if let Some(of) = fl(n[nb::N]) {
            if f & flags::N != 0 || of & flags::S != 0 {
                mask |= open::N;
            }
        }
        if let Some(of) = fl(n[nb::S]) {
            if f & flags::S != 0 || of & flags::N != 0 {
                mask |= open::S;
            }
        }
        let s = self.sq_mut(id);
        s.nb = n;
        s.open = mask;
    }

    /// `LightingChunk::fromChunk2`.
    fn link_chunk(&mut self, cx: i32, cy: i32) {
        let ids: Vec<SqId> = match self.map.at(cx, cy) {
            // Level by level, x outer and y inner within a level.
            Some(c) => c.levels.iter().flat_map(|l| l.iter().flatten().copied()).collect(),
            None => return,
        };
        for id in ids {
            self.link_square(id);
        }
        if let Some(c) = self.map.slot(cx, cy).as_deref_mut() {
            c.fresh = false;
        }
    }

    // ---------------------------------------------------------------
    // applyMapChanges

    /// `LightingState::applyMapChanges`: applies the handed-over queue.
    pub fn apply_map_changes(&mut self, g: &mut Globals) {
        self.map.full_dirty = false;
        self.frames = 0;
        let mut any_chunk = false;
        let mut lights_change: Option<(Vec<usize>, Vec<usize>)> = None;
        let pending = std::mem::take(&mut self.pending);
        for change in pending {
            match change {
                MapChange::Left => self.scroll_left(),
                MapChange::Right => self.scroll_right(),
                MapChange::Up => self.scroll_up(),
                MapChange::Down => self.scroll_down(),
                MapChange::Teleport { x, y } => self.teleport(x, y),
                MapChange::Lights { add, remove } => lights_change = Some((add, remove)),
                MapChange::EndFrame(ef) => {
                    self.sky = ef.sky;
                    self.ambient = [ef.ambient, ef.night, ef.view_dist, ef.view_dist_max];
                    g.rgb_mod = [ef.rmod, ef.gmod, ef.bmod];
                    if !self.cache_cleared {
                        self.cache_cleared = ef.cache_cleared;
                    }
                    if ef.light_source_update > 30.0 {
                        self.light_source_update = ef.light_source_update;
                    }
                    if ef.max_player == self.player {
                        self.frames = ef.frames;
                    }
                }
                MapChange::Player(c) => {
                    self.chr = Some(c);
                }
                MapChange::Chunk { chunk, updated } => {
                    self.set_chunk(chunk, &updated);
                    any_chunk = true;
                }
            }
        }
        self.ox = self.map.x << 3;
        self.oy = self.map.y << 3;
        for rl in &mut self.back {
            rl.ox = self.ox;
            rl.oy = self.oy;
        }
        if self.map.full_dirty {
            self.cache_cleared = true;
        }
        if let Some((add, remove)) = lights_change {
            for l in remove {
                self.lights.retain(|&x| x != l);
                g.lights[l].added[self.player as usize] = false;
                self.light_source_update = 100.0;
            }
            for l in add {
                self.lights.push(l);
                self.light_source_update = 100.0;
            }
        }
        let any_temp = self.lights.iter().any(|&l| g.lights[l].life_max > 0 && g.lights[l].life > 0);
        self.lights_changed =
            self.temp_lights_changed || any_temp || !(self.light_source_update <= 30.0) || any_chunk;
        let (w, h) = (self.map.w, self.map.h);
        if h > 0 && w > 0 {
            for cy in 0..h {
                for cx in 0..w {
                    let Some(chunk) = self.map.at(cx, cy) else { continue };
                    if (!chunk.done || (chunk.fresh && !self.lights_changed)) && chunk.min <= chunk.max {
                        let levels: Vec<(i32, [Option<SqId>; 64])> =
                            (chunk.min..=chunk.max).map(|z| (z, *chunk.level(z).unwrap())).collect();
                        for (z, level) in levels {
                            for ix in 0..8 {
                                for iy in 0..8 {
                                    let Some(id) = level[(ix * 8 + iy) as usize] else { continue };
                                    let idx = (cx * 8 + ix) + self.w * (cy * 8 + iy);
                                    self.sq_mut(id).idx = idx as u16;
                                    let (rw, rh) = (self.w, self.h);
                                    let d = self.back[z as usize & 63].data_mut(rw, rh);
                                    let i = idx as usize;
                                    d.level[i] = 0;
                                    d.statics[i].count = 0;
                                    d.lit[i] &= 0xfe;
                                }
                            }
                        }
                    }
                    if any_chunk || self.map.full_dirty {
                        self.link_chunk(cx, cy);
                    }
                    if let Some(c) = self.map.slot(cx, cy).as_deref_mut() {
                        c.fresh = false;
                    }
                }
            }
        }
        self.chr_square = None;
        if let Some(c) = &self.chr {
            let lx = (c.x as i32).wrapping_sub(self.ox);
            let ly = (c.y as i32).wrapping_sub(self.oy);
            if lx >= 0 && lx < self.w && ly >= 0 && ly < self.h {
                self.chr_square = self
                    .map
                    .at(lx >> 3, ly >> 3)
                    .and_then(|ch| ch.square(lx & 7, ly & 7, c.z as i32));
            }
        }
    }

    // ---------------------------------------------------------------
    // The update

    /// `LightingState::DoLightingUpdateNew`. Returns whether there was a
    /// character to compute for.
    pub fn do_lighting_update(&mut self, g: &mut Globals, nanos: u64) -> bool {
        let Some(chr) = self.chr.as_deref().copied() else { return false };
        let (max_excl, min);
        if self.map.w < 1 || self.map.h < 1 {
            max_excl = 0x21;
            min = 0x20;
        } else {
            let mut mx = 0x20;
            let mut mn = 0x20;
            for c in self.map.chunks.iter().flatten() {
                if mx <= c.max {
                    mx = c.max;
                }
                if c.min < mn {
                    mn = c.min;
                }
            }
            max_excl = mx + 1;
            min = mn;
        }
        let pos = [fastfloor(chr.x), fastfloor(chr.y), fastfloor(chr.z)];
        if self.cache_cleared || self.last_square != pos {
            self.last_square = pos;
            self.los.fill(0);
            self.cache_cleared = false;
        }
        let (w, h) = (self.w, self.h);
        self.time = [nanos, self.time[0]];
        self.visible_rooms_work.clear();

        for z in min..max_excl {
            if w <= 0 {
                continue;
            }
            let rows = (0..h / 2).chain((h / 2..h).rev());
            for y in rows {
                for x in 0..w {
                    self.visibility_square(&chr, x, y, z);
                }
            }
        }
        self.max_level = max_excl;
        self.min_level = min;

        if self.lights_changed {
            self.update_light_source(g);
            self.light_source_update = 0.0;
        }
        for &l in &self.lights {
            let light = &mut g.lights[l];
            if light.life_max > 0 && light.life > 0 {
                let v = light.life.wrapping_sub(self.frames);
                light.life = v.max(0);
                self.temp_lights_changed = true;
            }
        }
        if self.dirty_global || self.temp_lights_changed || self.lights_changed {
            self.calculate_full_lighting();
        }
        self.add_torch_lighting(g);

        self.for_each_square(|s, id| s.calc_light_info(id, &chr));
        self.for_each_square(|s, id| s.calc_max_light_info(id));
        self.for_each_square(|s, id| s.calculate_max_vert_light1(id));
        self.for_each_square(|s, id| s.calculate_max_vert_light2(id));
        self.for_each_square(|s, id| s.calculate_vert_colours_for_tile(id));
        for c in self.map.chunks.iter_mut().flatten() {
            c.done = true;
        }
        true
    }

    /// One square of the visibility pass.
    fn visibility_square(&mut self, chr: &Character, x: i32, y: i32, z: i32) {
        let (w, h) = (self.w, self.h);
        let full = (y * w + x) as usize;
        let chunk = self.map.at(x >> 3, y >> 3);
        let found = chunk.and_then(|c| c.square(x & 7, y & 7, z).map(|id| (id, c.done)));
        let Some((id, done)) = found else {
            let d = self.back[z as usize & 63].data_mut(w, h);
            d.vis[full] &= 0xfe;
            return;
        };
        let i = full & 0xffff;
        self.sq_mut(id).idx = i as u16;
        let (sx, sy, sflags) = {
            let s = self.sq(id);
            (s.x, s.y, s.flags)
        };
        let zi = z as usize & 63;
        if !done {
            let d = self.back[zi].data_mut(w, h);
            d.vis[i] = (d.vis[i] & 0xfe) | ((sflags >> 12) as u8 & 1);
        } else {
            let (fox, foy) = (self.front[zi].ox, self.front[zi].oy);
            let src = (sx.wrapping_sub(fox)).wrapping_add(sy.wrapping_sub(foy).wrapping_mul(w));
            if src >= 0 && src < h * w {
                let src = src as usize;
                let f = self.front[zi].data_mut(w, h);
                let (fv, fd, fs, ft, fdirty) = (f.vis[src], f.dark[src], f.statics[src], f.torches[src], f.dirty[src]);
                let d = self.back[zi].data_mut(w, h);
                d.vis[i] = (d.vis[i] & 0xfe) | (fv & 1);
                d.dark[i] = fd;
                d.statics[i] = fs;
                d.torches[i] = ft;
                d.dirty[i] = fdirty;
            }
        }
        let lights_changed = self.lights_changed;
        {
            let d = self.back[zi].data_mut(w, h);
            if lights_changed {
                d.statics[i].count = 0;
            }
            d.torches[i].count = 0;
        }
        self.calc_visibility(id, chr);
        let full_dirty = self.map.full_dirty;
        let d = self.back[zi].data_mut(w, h);
        if !done {
            let t = d.target[i];
            let s = &mut self.squares[id as usize];
            if s.prev_dark != (t * 1000.0) as i32 {
                d.set_dirty(w, i);
            }
            d.dark[i] = t;
            s.prev_dark = (t * 1000.0) as i32;
        }
        if full_dirty {
            d.set_dirty(w, i);
        }
    }

    /// Runs `f` for every square, chunk row by chunk row, chunk column,
    /// level, then square index (x fastest), the order of the colour
    /// passes.
    fn for_each_square(&mut self, mut f: impl FnMut(&mut LightingState, SqId)) {
        let (w, h) = (self.map.w, self.map.h);
        for cy in 0..h {
            for cx in 0..w {
                let Some(c) = self.map.at(cx, cy) else { continue };
                if c.min > c.max {
                    continue;
                }
                let ids: Vec<SqId> = (c.min..=c.max)
                    .flat_map(|z| {
                        let l = c.level(z).unwrap();
                        (0..64u32).filter_map(move |u| l[((u & 7) * 8 + (u >> 3)) as usize])
                    })
                    .collect();
                for id in ids {
                    f(self, id);
                }
            }
        }
    }

    /// `ResultLevel::swap` for the computed levels and the chunk `done`
    /// bytes, done when the main thread picks up a finished update.
    pub fn swap_results(&mut self) {
        let lo = self.min_level;
        let hi = self.max_level;
        let mut z = lo;
        while z < hi {
            let zi = z as usize & 63;
            std::mem::swap(&mut self.front[zi], &mut self.back[zi]);
            z += 1;
        }
        let (w, h) = (self.map.w, self.map.h);
        if h > 0 && w > 0 {
            for cy in 0..h {
                for cx in 0..w {
                    let done = self.map.at(cx, cy).map(|c| c.done).unwrap_or(false);
                    self.map.done[(cy * w + cx) as usize] = u8::from(done);
                }
            }
        }
        self.visible_rooms = self.visible_rooms_work.clone();
        self.update_counter = if self.update_counter >= -1 { self.update_counter.wrapping_add(1) } else { 0 };
        self.done = false;
    }
}
