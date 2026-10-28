//! The animal population: `AnimalPopulationManager`, `AnimalManagerMain`,
//! `AnimalManagerWorker`, `AnimalCell`, `AnimalChunk`, the static parts of
//! `AnimalZones` and the `apop/apop_<x>_<y>.bin` files.
//!
//! The original's worker is synchronous and on the main thread, so the
//! three managers are one [`AnimalPopulation`] here. Virtual animals live in
//! an arena keyed by [`VirtualAnimal::uid`] (the original compares them by
//! reference); chunks hold their uids. The live world (creating, saving
//! and loading `IsoAnimal`s, turning a virtual herd into characters) is the
//! [`PopulationHost`]'s.

pub mod virtual_animal;
pub mod zone;

use std::collections::{HashMap, HashSet};

use indexmap::IndexSet;
use pz_world::java::fastfloor;
use pz_world::save::io::{Reader, SaveResult, Writer};
use pz_world::save::movers::AnimalTracks;
use pz_world::zones::ZoneId;
use pz_world::{GameRand, IsoDirections, IsoMetaGrid};

use crate::java::{JResult, JavaException};
use pz_core::config::debug_log;
use crate::{AnimalDefinitionTables, AnimalTracksType};
pub use virtual_animal::{VirtualAnimal, VirtualAnimalState};

/// Population callbacks keep the original definition/host JavaException
/// separate from actual junction-field failures. Neither cause is flattened
/// into a success value or a newly constructed exception string.
#[derive(Debug)]
pub enum PopulationError {
    Java(JavaException),
    Junctions(zone::JunctionQueryFailure),
}
pub type PopulationResult<T> = Result<T,PopulationError>;
impl From<JavaException> for PopulationError {
    fn from(original:JavaException)->Self {Self::Java(original)}
}
impl From<zone::JunctionQueryFailure> for PopulationError {
    fn from(original:zone::JunctionQueryFailure)->Self {Self::Junctions(original)}
}
impl std::fmt::Display for PopulationError {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
        match self {Self::Java(original)=>std::fmt::Display::fmt(original,f),Self::Junctions(original)=>std::fmt::Display::fmt(original,f)}
    }
}
impl std::error::Error for PopulationError {
    fn source(&self)->Option<&(dyn std::error::Error+'static)> {
        Some(match self {Self::Java(original)=>original,Self::Junctions(original)=>original})
    }
}


pub const SQUARES_PER_CHUNK: i32 = 8;
pub const CHUNKS_PER_CELL: i32 = 32;
pub const SQUARES_PER_CELL: i32 = 256;
/// The world version `AnimalCell.save` writes.
pub const APOP_WORLD_VERSION: i32 = 249;

/// What the population reads and sets on an `IsoAnimal` of a herd.
pub trait HerdAnimal {
    /// `getAnimalID()`.
    fn animal_id(&self) -> i32;
    /// `attachBackToMother` (the mother's animal id, or 0).
    fn attach_back_to_mother(&self) -> i32;
    /// `setAnimalZone(zone)`.
    fn set_animal_zone(&mut self, zone: Option<ZoneId>);
    /// `setMoveForwardOnZone(b)`.
    fn set_move_forward_on_zone(&mut self, forward: bool);
}

/// The live game the population works against.
pub trait PopulationHost<A> {
    /// `Rand`.
    fn rand(&mut self) -> &mut dyn GameRand;
    /// `GameTime.getHour()`.
    fn hour(&self) -> i32;
    /// `GameTime.getWorldAgeHours()`.
    fn world_age_hours(&self) -> f64;
    /// `GameTime.getCalender().getTimeInMillis()`.
    fn calendar_millis(&self) -> i64;
    /// `GameTime.getMultiplier()`.
    fn multiplier(&self) -> f32;
    /// `GameTime.getThirtyFPSMultiplier()`.
    fn thirty_fps_multiplier(&self) -> f32;
    /// `SandboxOptions.animalTrackChance` (1 = none).
    fn animal_track_chance(&self) -> i32;

    /// `new IsoAnimal(cell, 0, 0, 0, type, breed)`.
    fn new_animal(&mut self, animal_type: &str, breed: &str) -> JResult<A>;
    /// `randomizeAge()`.
    fn randomize_age(&mut self, animal: &mut A) -> JResult<()>;
    /// `getData().canHaveBaby()`.
    fn can_have_baby(&self, animal: &A) -> bool;
    /// `addBaby()` (`None`: the original's null for a breed without babies).
    fn add_baby(&mut self, mother: &mut A) -> JResult<Option<A>>;
    /// `virtualId = id; migrationGroup = group` on a generated animal.
    fn set_virtual_id(&mut self, animal: &mut A, virtual_id: f64, migration_group: &str);
    /// `baby.setMother(mother)`.
    fn set_mother(&mut self, baby: &mut A, mother: &mut A);

    /// `IsoAnimal.save(output, false)` (with the object factory header).
    fn save_animal(&mut self, animal: &A, w: &mut Writer) -> SaveResult<()>;
    /// `IsoObject.factoryFromFileInput` + `IsoAnimal.load(input, version)`,
    /// leaving the animal out of the cell's lists.
    fn load_animal(&mut self, r: &mut Reader<'_>, world_version: i32) -> SaveResult<A>;

    /// The bytes of `apop/apop_<x>_<y>.bin` (`Ok(None)`: no such file).
    fn read_apop(&mut self, x: i32, y: i32) -> std::io::Result<Option<Vec<u8>>>;
    /// Writes `apop/apop_<x>_<y>.bin`.
    fn write_apop(&mut self, x: i32, y: i32, bytes: &[u8]) -> std::io::Result<()>;
    /// `Core.getInstance().isNoSave()`.
    fn no_save(&self) -> bool;

    /// `AnimalManagerMain.fromWorker(animals)`: put each herd's animals
    /// into the world.
    fn from_worker(&mut self, meta: &IsoMetaGrid, animals: Vec<VirtualAnimal<A>>);
}

/// What one population call needs besides the population.
pub struct PopCx<'a, A> {
    pub meta: &'a mut IsoMetaGrid,
    pub defs: &'a AnimalDefinitionTables,
    pub host: &'a mut dyn PopulationHost<A>,
}

/// An `IsoAnimal` in the world being saved or virtualised
/// (`saveRealAnimals`, `n_addAnimal`).
#[derive(Clone, Debug)]
pub struct RealAnimal<A> {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub move_forward_on_zone: bool,
    pub forward_direction: (f32, f32),
    pub virtual_id: f64,
    pub migration_group: Option<String>,
    /// `getLastCellSavedToX/Y()` (`i32::MIN`: never saved).
    pub last_cell_saved_to: (i32, i32),
    pub animal: A,
}

/// `AnimalChunk`.
#[derive(Clone, Debug, Default)]
pub struct AnimalChunk {
    pub x: i32,
    pub y: i32,
    /// `animals` (virtual animal uids).
    pub animals: Vec<u64>,
    pub animal_tracks: Vec<AnimalTracks>,
    /// `tracksUpdateTimer`. The original's pooled chunk keeps its old
    /// timer when reused; a new chunk starts at 0 here.
    pub tracks_update_timer: f32,
}

impl AnimalChunk {
    fn is_empty(&self) -> bool {
        self.animals.is_empty() && self.animal_tracks.is_empty()
    }
}

/// `AnimalCell`.
#[derive(Clone, Debug, Default)]
pub struct AnimalCell {
    pub x: i32,
    pub y: i32,
    /// `chunks` (`null` until loaded).
    pub chunks: Option<Vec<Option<AnimalChunk>>>,
    pub loaded: bool,
    pub file_loaded: bool,
    pub data_changed: bool,
    /// `loadedChunks` (a 32x32 `BooleanGrid`).
    pub loaded_chunks: Option<Vec<bool>>,
    /// `saveRealAnimalHack` (virtual animal uids, saved once and dropped).
    pub save_real_animal_hack: Option<Vec<u64>>,
    pub added_junctions: bool,
}

impl AnimalCell {
    fn new(x: i32, y: i32) -> Self {
        AnimalCell { x, y, ..Default::default() }
    }

    fn chunk_index_from_square(&self, x: i32, y: i32) -> Option<usize> {
        if !self.loaded {
            return None;
        }
        let (x, y) = (x - self.x * SQUARES_PER_CELL, y - self.y * SQUARES_PER_CELL);
        if !(0..SQUARES_PER_CELL).contains(&x) || !(0..SQUARES_PER_CELL).contains(&y) {
            return None;
        }
        Some((x / 8 + y / 8 * 32) as usize)
    }

    fn chunk_index_from_chunk(&self, x: i32, y: i32) -> Option<usize> {
        if !self.loaded {
            return None;
        }
        let (x, y) = (x - self.x * 32, y - self.y * 32);
        if !(0..32).contains(&x) || !(0..32).contains(&y) {
            return None;
        }
        Some((x + y * 32) as usize)
    }

    fn chunk_at(&self, index: Option<usize>) -> Option<usize> {
        let i = index?;
        self.chunks.as_ref()?[i].as_ref().map(|_| i)
    }

    /// `setChunkLoaded(x, y, loaded)`.
    fn set_chunk_loaded(&mut self, x: i32, y: i32, loaded: bool) {
        if !loaded && self.loaded_chunks.is_none() {
            return;
        }
        let grid = self.loaded_chunks.get_or_insert_with(|| vec![false; 1024]);
        let (x, y) = (x - self.x * 32, y - self.y * 32);
        // BooleanGrid.setValue ignores out-of-range positions.
        if (0..32).contains(&x) && (0..32).contains(&y) {
            grid[(x + y * 32) as usize] = loaded;
        }
    }

    /// `isChunkLoadedWorldPos(x, y)`.
    fn is_chunk_loaded_world_pos(&self, x: i32, y: i32) -> bool {
        let Some(grid) = &self.loaded_chunks else {
            return false;
        };
        let (x, y) = ((x - self.x * 256) / 8, (y - self.y * 256) / 8);
        (0..32).contains(&x) && (0..32).contains(&y) && grid[(x + y * 32) as usize]
    }
}

/// (cell index, chunk index in the cell).
pub type ChunkKey = (usize, usize);

/// `AnimalPopulationManager` + `AnimalManagerMain` + `AnimalManagerWorker`
/// + `AnimalZones`' statics.
#[derive(Debug)]
pub struct AnimalPopulation<A> {
    pub min_x: i32,
    pub min_y: i32,
    pub width: i32,
    pub height: i32,
    pub stopped: bool,
    pub client: bool,
    pub server: bool,
    /// `newChunks` (`wy << 16 | wx`).
    pub new_chunks: HashSet<i32>,
    /// `AnimalManagerWorker.cells` (`null` before `init` / after `stop`).
    pub cells: Option<Vec<Option<AnimalCell>>>,
    /// `loadedCells` (indices into `cells`).
    pub loaded_cells: Vec<usize>,
    /// `AnimalZones.chunksWithTracks` (chunk positions).
    pub chunks_with_tracks: IndexSet<(i32, i32)>,
    /// The virtual animals, by uid.
    pub animals: HashMap<u64, VirtualAnimal<A>>,
    next_uid: u64,
    /// Set by `moveAnimal` when the herd being updated reached a loaded
    /// chunk (`passToMain`).
    pass_to_main: bool,
}

impl<A> Default for AnimalPopulation<A> {
    fn default() -> Self {
        AnimalPopulation {
            min_x: 0,
            min_y: 0,
            width: 0,
            height: 0,
            stopped: false,
            client: false,
            server: false,
            new_chunks: HashSet::new(),
            cells: None,
            loaded_cells: Vec::new(),
            chunks_with_tracks: IndexSet::new(),
            animals: HashMap::new(),
            next_uid: 1,
            pass_to_main: false,
        }
    }
}

/// `AnimalTracks.addAnimalTrack` / `addAnimalTrackAtPos`'s track.
fn new_track<A>(va: &VirtualAnimal<A>, tt: &AnimalTracksType, x: i32, y: i32, added_time: i64) -> AnimalTracks {
    AnimalTracks {
        animal_type: va.migration_group.clone().unwrap_or_default(),
        track_type: tt.type_.clone().unwrap_or_default(),
        x,
        y,
        dir: tt.need_dir.then(|| {
            // IsoDirections.fromAngle(dx, dy).
            let (fx, fy) = va.forward_direction;
            let angle = (fy as f64).atan2(fx as f64) as f32;
            IsoDirections::ALL[((6 - (8.5f32 + 1.273_239_5 * angle) as i32) & 7) as usize] as i32
        }),
        added_time,
        added_to_world: false,
    }
}

impl<A: HerdAnimal> AnimalPopulation<A> {
    /// A new virtual animal (`new VirtualAnimal()`, drawing `Rand`).
    pub fn new_virtual_animal(&mut self, rand: &mut dyn GameRand) -> VirtualAnimal<A> {
        let uid = self.next_uid;
        self.next_uid += 1;
        VirtualAnimal::new(uid, rand)
    }

    fn cell(&self, i: usize) -> &AnimalCell {
        self.cells.as_ref().unwrap()[i].as_ref().unwrap()
    }

    fn cell_mut(&mut self, i: usize) -> &mut AnimalCell {
        self.cells.as_mut().unwrap()[i].as_mut().unwrap()
    }

    pub fn chunk(&self, (c, k): ChunkKey) -> &AnimalChunk {
        self.cell(c).chunks.as_ref().unwrap()[k].as_ref().unwrap()
    }

    fn chunk_mut(&mut self, (c, k): ChunkKey) -> &mut AnimalChunk {
        self.cell_mut(c).chunks.as_mut().unwrap()[k].as_mut().unwrap()
    }

    // ---- AnimalPopulationManager ----

    /// `init(metaGrid)`.
    pub fn init(&mut self, client: bool, server: bool, cx: &mut PopCx<'_, A>) {
        self.client = client;
        self.min_x = cx.meta.min_x;
        self.min_y = cx.meta.min_y;
        self.width = cx.meta.width;
        self.height = cx.meta.height;
        self.stopped = false;
        self.worker_init(client, server, cx);
    }

    /// `addChunkToWorld(chunk)`.
    pub fn add_chunk_to_world(&mut self, wx: i32, wy: i32, new_chunk: bool, cx: &mut PopCx<'_, A>) -> JResult<()> {
        if self.client {
            return Ok(());
        }
        if new_chunk {
            self.new_chunks.insert(wy << 16 | wx);
        }
        self.load_chunk(wx, wy, cx)
    }

    /// The start of `removeChunkFromWorld(chunk)`: false when the original
    /// returns at once. The caller then hands each `IsoAnimal` on the
    /// chunk's squares to [`Self::add_real_animal`] (after its
    /// `unloaded()`), and finishes with [`Self::remove_chunk_from_world_end`].
    pub fn remove_chunk_from_world_begin(&mut self, wx: i32, wy: i32) -> bool {
        if self.client || self.stopped {
            return false;
        }
        self.unload_chunk(wx, wy);
        true
    }

    /// The end of `removeChunkFromWorld(chunk)`.
    pub fn remove_chunk_from_world_end(&mut self, wx: i32, wy: i32) {
        self.new_chunks.remove(&(wy << 16 | wx));
    }

    /// `n_addAnimal(animal)` (also `virtualizeAnimal`'s).
    pub fn add_real_animal(&mut self, real: RealAnimal<A>, cx: &mut PopCx<'_, A>) -> JResult<()> {
        let mut va = self.new_virtual_animal(cx.host.rand());
        va.x = real.x;
        va.y = real.y;
        va.set_z(real.z);
        va.move_forward_on_zone = real.move_forward_on_zone;
        va.forward_direction = real.forward_direction;
        va.animals.push(real.animal);
        va.id = real.virtual_id;
        va.migration_group = real.migration_group;
        va.init_value_from_def(cx.defs)?;
        self.add_animal(va, cx);
        Ok(())
    }

    /// `save()`: the world's animals (`saveRealAnimals`) then the changed
    /// cells.
    pub fn save(&mut self, real: Vec<RealAnimal<A>>, cx: &mut PopCx<'_, A>) -> JResult<Vec<(i32, i32)>> {
        if self.client {
            return Ok(Vec::new());
        }
        let saved_to = self.save_real_animals(real, cx)?;
        self.worker_save(cx);
        Ok(saved_to)
    }

    /// `saveToBufferMap(bufferMap)`: the `apop` files that changed, as
    /// (cell x, cell y, bytes).
    pub fn save_to_buffer_map(
        &mut self,
        real: Vec<RealAnimal<A>>,
        cx: &mut PopCx<'_, A>,
    ) -> JResult<(Vec<(i32, i32)>, Vec<(i32, i32, Vec<u8>)>)> {
        if self.client {
            return Ok((Vec::new(), Vec::new()));
        }
        let saved_to = self.save_real_animals(real, cx)?;
        let mut out = Vec::new();
        for i in self.loaded_cells.clone() {
            if !self.cell(i).data_changed {
                continue;
            }
            self.cell_mut(i).data_changed = false;
            if !self.cell(i).loaded || cx.host.no_save() {
                continue;
            }
            match self.cell_bytes(i, cx) {
                Ok(b) => out.push((self.cell(i).x, self.cell(i).y, b)),
                Err(e) => debug_log(&format!("AnimalCell.saveToBufferMap: {e}")),
            }
        }
        Ok((saved_to, out))
    }

    /// `stop()`. The original clears each changed cell's flag before
    /// unloading it, so nothing is saved here.
    pub fn stop(&mut self) {
        if self.client {
            return;
        }
        self.stopped = true;
        self.loaded_cells.clear();
        self.cells = None;
        self.new_chunks.clear();
        self.animals.clear();
        self.chunks_with_tracks.clear();
    }

    // ---- AnimalManagerWorker ----

    fn worker_init(&mut self, client: bool, server: bool, cx: &mut PopCx<'_, A>) {
        self.server = server;
        self.client = client;
        let n = (self.width * self.height).max(0) as usize;
        self.cells = Some(vec![None; n]);
        if client {
            return;
        }
        for i in 0..n {
            self.alloc_cell(i, cx);
        }
    }

    fn alloc_cell(&mut self, index: usize, cx: &mut PopCx<'_, A>) {
        let (x, y) = (self.min_x + index as i32 % self.width, self.min_y + index as i32 / self.width);
        self.cells.as_mut().unwrap()[index] = Some(AnimalCell::new(x, y));
        self.create_junctions(index, cx.meta);
    }

    /// `allocCell(cellX, cellY)` (`ZoneGenerator.genAnimalsPath` after it
    /// made the cell's paths).
    pub fn alloc_cell_at(&mut self, cell_x: i32, cell_y: i32, cx: &mut PopCx<'_, A>) {
        if let Some(i) = self.cell_index_from_cell_pos(cell_x, cell_y) {
            self.cell_mut(i).added_junctions = false;
            self.create_junctions(i, cx.meta);
            return;
        }
        let index = cell_x - self.min_x + (cell_y - self.min_y) * self.width;
        let in_range = self.cells.as_ref().is_some_and(|c| index >= 0 && (index as usize) < c.len());
        if in_range {
            self.alloc_cell(index as usize, cx);
        }
    }

    /// `saveRealAnimals(animals)`: the new `lastCellSavedTo` of each animal
    /// (unchanged when it is outside the cells).
    fn save_real_animals(&mut self, real: Vec<RealAnimal<A>>, cx: &mut PopCx<'_, A>) -> JResult<Vec<(i32, i32)>> {
        let mut saved_to = Vec::with_capacity(real.len());
        for r in real {
            self.check_last_cell_saved_to(&r, cx);
            let Some(cell) = self.cell_index_from_square_pos(fastfloor(r.x), fastfloor(r.y)) else {
                saved_to.push(r.last_cell_saved_to);
                continue;
            };
            self.load_if_needed(cell, cx);
            let mut va = self.new_virtual_animal(cx.host.rand());
            va.x = r.x;
            va.y = r.y;
            va.set_z(r.z);
            va.move_forward_on_zone = r.move_forward_on_zone;
            va.forward_direction = r.forward_direction;
            va.id = r.virtual_id;
            va.migration_group = r.migration_group;
            va.animals.push(r.animal);
            let uid = va.uid;
            self.animals.insert(uid, va);
            let c = self.cell_mut(cell);
            c.save_real_animal_hack.get_or_insert_with(Vec::new).push(uid);
            c.data_changed = true;
            saved_to.push((c.x, c.y));
        }
        Ok(saved_to)
    }

    fn check_last_cell_saved_to(&mut self, r: &RealAnimal<A>, cx: &mut PopCx<'_, A>) {
        let (cx_, cy_) = r.last_cell_saved_to;
        if cx_ == i32::MIN || cy_ == i32::MIN {
            return;
        }
        let (cur_x, cur_y) = (fastfloor(r.x / 256.0), fastfloor(r.y / 256.0));
        if cx_ == cur_x && cy_ == cur_y {
            return;
        }
        let Some(cell) = self.cell_index_from_cell_pos(cx_, cy_) else {
            return;
        };
        self.load_if_needed(cell, cx);
        self.cell_mut(cell).data_changed = true;
    }

    fn worker_save(&mut self, cx: &mut PopCx<'_, A>) {
        for i in self.loaded_cells.clone() {
            if !self.cell(i).data_changed {
                continue;
            }
            self.cell_mut(i).data_changed = false;
            self.save_cell(i, cx);
        }
    }

    fn cell_index_from_cell_pos(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x - self.min_x, y - self.min_y);
        if x < 0 || x >= self.width || y < 0 || y >= self.height {
            return None;
        }
        let i = (x + y * self.width) as usize;
        self.cells.as_ref()?.get(i)?.as_ref().map(|_| i)
    }

    fn cell_index_from_square_pos(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x - self.min_x * 256, y - self.min_y * 256);
        if x < 0 || y < 0 {
            return None;
        }
        let (cx, cy) = (x / 256, y / 256);
        if cx >= self.width || cy >= self.height {
            return None;
        }
        let i = (cx + cy * self.width) as usize;
        self.cells.as_ref()?.get(i)?.as_ref().map(|_| i)
    }

    fn cell_index_from_chunk_pos(&self, x: i32, y: i32) -> Option<usize> {
        self.cell_index_from_square_pos(x * 8, y * 8)
    }

    fn load_if_needed(&mut self, cell: usize, cx: &mut PopCx<'_, A>) {
        if self.cell(cell).loaded {
            return;
        }
        self.load_cell(cell, cx);
        self.loaded_cells.push(cell);
    }

    /// `loadChunk(x, y)`: the chunk's herds go into the world.
    fn load_chunk(&mut self, x: i32, y: i32, cx: &mut PopCx<'_, A>) -> JResult<()> {
        let Some(cell) = self.cell_index_from_chunk_pos(x, y) else {
            return Ok(());
        };
        self.cell_mut(cell).set_chunk_loaded(x, y, true);
        self.load_if_needed(cell, cx);
        let c = self.cell(cell);
        let Some(k) = c.chunk_at(c.chunk_index_from_chunk(x, y)) else {
            return Ok(());
        };
        let uids = std::mem::take(&mut self.chunk_mut((cell, k)).animals);
        let herds: Vec<_> = uids.iter().filter_map(|u| self.animals.remove(u)).collect();
        cx.host.from_worker(&*cx.meta, herds);
        self.cell_mut(cell).data_changed = true;
        Ok(())
    }

    fn unload_chunk(&mut self, x: i32, y: i32) {
        if let Some(cell) = self.cell_index_from_chunk_pos(x, y) {
            self.cell_mut(cell).set_chunk_loaded(x, y, false);
        }
    }

    /// `addAnimal(animal)`.
    pub fn add_animal(&mut self, mut va: VirtualAnimal<A>, cx: &mut PopCx<'_, A>) {
        let (sx, sy) = (fastfloor(va.x), fastfloor(va.y));
        let Some(cell) = self.cell_index_from_square_pos(sx, sy) else {
            return;
        };
        self.load_if_needed(cell, cx);
        let Some(key) = self.get_or_create_chunk_from_square_pos(cell, sx, sy) else {
            return;
        };
        if self.chunk(key).animals.contains(&va.uid) {
            self.animals.insert(va.uid, va);
            return;
        }
        let existing = (va.id != 0.0)
            .then(|| self.chunk(key).animals.iter().copied().find(|u| self.animals.get(u).is_some_and(|o| o.id != 0.0 && o.id == va.id)))
            .flatten();
        if let Some(other) = existing {
            let mut moved = Vec::new();
            for a in va.animals.drain(..) {
                let o = &self.animals[&other];
                if o.animals.iter().any(|b| b.animal_id() == a.animal_id()) {
                    debug_log(&format!("Trying to add an animal with an existing IsoAnimal.animalID."));
                    continue;
                }
                moved.push(a);
            }
            self.animals.get_mut(&other).unwrap().animals.extend(moved);
            self.cell_mut(cell).data_changed = true;
            return;
        }
        let uid = va.uid;
        self.animals.insert(uid, va);
        let chunk = self.chunk_mut(key);
        chunk.animals.push(uid);
        let size = chunk.animals.len() + chunk.animal_tracks.len();
        let pos = (chunk.x, chunk.y);
        self.cell_mut(cell).data_changed = true;
        if size == 1 {
            self.chunks_with_tracks.insert(pos);
        }
    }

    /// `removeFromWorld(animal)` with its `lastCellSavedTo`.
    pub fn remove_from_world(&mut self, last_cell_saved_to: (i32, i32), cx: &mut PopCx<'_, A>) {
        let (x, y) = last_cell_saved_to;
        if x == i32::MIN || y == i32::MIN {
            return;
        }
        if let Some(cell) = self.cell_index_from_cell_pos(x, y) {
            self.load_if_needed(cell, cx);
            self.cell_mut(cell).data_changed = true;
        }
    }

    /// `getAnimalChunk(x, y)`.
    pub fn get_animal_chunk(&mut self, x: f32, y: f32, cx: &mut PopCx<'_, A>) -> Option<ChunkKey> {
        let (sx, sy) = (fastfloor(x), fastfloor(y));
        let cell = self.cell_index_from_square_pos(sx, sy)?;
        self.load_if_needed(cell, cx);
        let c = self.cell(cell);
        let k = c.chunk_at(c.chunk_index_from_square(sx, sy))?;
        Some((cell, k))
    }

    /// `getOrCreateAnimalChunk(x, y)`.
    pub fn get_or_create_animal_chunk(&mut self, x: f32, y: f32, cx: &mut PopCx<'_, A>) -> Option<ChunkKey> {
        let (sx, sy) = (fastfloor(x), fastfloor(y));
        let cell = self.cell_index_from_square_pos(sx, sy)?;
        self.load_if_needed(cell, cx);
        self.get_or_create_chunk_from_square_pos(cell, sx, sy)
    }

    /// `moveAnimal(animal, x, y)` for the herd being updated.
    fn move_animal(&mut self, va: &mut VirtualAnimal<A>, x: f32, y: f32, cx: &mut PopCx<'_, A>) {
        self.remove_from_chunk(va, cx);
        va.x = x;
        va.y = y;
        let (sx, sy) = (fastfloor(x), fastfloor(y));
        if self.is_chunk_loaded_world_pos(sx, sy) {
            va.removed = true;
            self.pass_to_main = true;
            return;
        }
        // The herd is re-added to the arena after its update (when a chunk
        // took it).
        let placeholder = VirtualAnimal {
            animals: std::mem::take(&mut va.animals),
            ..va.clone_without_animals()
        };
        self.add_animal(placeholder, cx);
        if let Some(back) = self.animals.remove(&va.uid) {
            va.animals = back.animals;
        }
    }

    fn remove_from_chunk(&mut self, va: &VirtualAnimal<A>, cx: &mut PopCx<'_, A>) {
        let (sx, sy) = (fastfloor(va.x), fastfloor(va.y));
        let Some(cell) = self.cell_index_from_square_pos(sx, sy) else {
            return;
        };
        self.load_if_needed(cell, cx);
        let c = self.cell(cell);
        let Some(k) = c.chunk_at(c.chunk_index_from_square(sx, sy)) else {
            return;
        };
        let chunk = self.chunk_mut((cell, k));
        let size = chunk.animals.len() + chunk.animal_tracks.len();
        let pos = (chunk.x, chunk.y);
        chunk.animals.retain(|&u| u != va.uid);
        if size == 1 {
            self.chunks_with_tracks.shift_remove(&pos);
        }
        self.release_chunk_if_empty((cell, k));
    }

    fn is_chunk_loaded_world_pos(&self, x: i32, y: i32) -> bool {
        self.cell_index_from_square_pos(x, y)
            .is_some_and(|c| self.cell(c).is_chunk_loaded_world_pos(x, y))
    }

    // ---- AnimalCell ----

    fn get_or_create_chunk(&mut self, cell: usize, index: usize) -> ChunkKey {
        let c = self.cell_mut(cell);
        let (x, y) = (c.x * 32 + index as i32 % 32, c.y * 32 + index as i32 / 32);
        let slot = &mut c.chunks.as_mut().unwrap()[index];
        if slot.is_none() {
            *slot = Some(AnimalChunk { x, y, ..Default::default() });
        }
        (cell, index)
    }

    fn get_or_create_chunk_from_square_pos(&mut self, cell: usize, x: i32, y: i32) -> Option<ChunkKey> {
        let i = self.cell(cell).chunk_index_from_square(x, y)?;
        Some(self.get_or_create_chunk(cell, i))
    }

    fn release_chunk_if_empty(&mut self, (cell, k): ChunkKey) {
        let c = self.cell_mut(cell);
        let Some(chunks) = c.chunks.as_mut() else {
            return;
        };
        let Some(chunk) = chunks[k].as_ref() else {
            return;
        };
        if !chunk.is_empty() {
            return;
        }
        let pos = (chunk.x, chunk.y);
        chunks[k] = None;
        self.chunks_with_tracks.shift_remove(&pos);
    }

    /// `AnimalChunk.release()`.
    fn release_chunk(&mut self, chunk: AnimalChunk) {
        if chunk.animals.len() + chunk.animal_tracks.len() > 0 {
            self.chunks_with_tracks.shift_remove(&(chunk.x, chunk.y));
        }
        for u in chunk.animals {
            self.animals.remove(&u);
        }
    }

    /// `AnimalCell.load()`.
    fn load_cell(&mut self, cell: usize, cx: &mut PopCx<'_, A>) {
        let c = self.cell_mut(cell);
        c.loaded = true;
        c.chunks = Some(vec![None; 1024]);
        let (x, y) = (c.x, c.y);
        // checkAnimalZonesGenerated: ZoneGenerator.genAnimalsPath runs when
        // the cell first loads (TODO(worldgen): genAnimalsPath).
        let _ = cx.meta.check_animal_zones_generated(x, y);
        let loaded = self.load_cell_file(cell, cx);
        self.cell_mut(cell).file_loaded = loaded;
        if !loaded {
            if let Err(e) = self.spawn_animals_in_cell(cell, cx) {
                debug_log(&format!("AnimalZones.spawnAnimalsInCell: {}", e.0));
            }
        }
    }

    /// `AnimalCell.load(fileName)`.
    fn load_cell_file(&mut self, cell: usize, cx: &mut PopCx<'_, A>) -> bool {
        let (x, y) = (self.cell(cell).x, self.cell(cell).y);
        let bytes = match cx.host.read_apop(x, y) {
            Ok(Some(b)) => b,
            Ok(None) => return false,
            Err(e) => {
                debug_log(&format!("AnimalCell.load apop_{x}_{y}.bin: {e}"));
                return false;
            }
        };
        let mut reattach = Vec::new();
        if let Err(e) = self.load_cell_bytes(cell, &bytes, &mut reattach, cx) {
            debug_log(&format!("AnimalCell.load apop_{x}_{y}.bin: {e}"));
            return false;
        }
        self.reattach_mothers(cell, reattach, cx);
        true
    }

    /// `AnimalCell.load(ByteBuffer)` and the chunks' `load`.
    fn load_cell_bytes(
        &mut self,
        cell: usize,
        bytes: &[u8],
        reattach: &mut Vec<(u64, usize)>,
        cx: &mut PopCx<'_, A>,
    ) -> SaveResult<()> {
        let mut r = Reader::new(bytes);
        let world_version = r.i32()?;
        for i in 0..1024 {
            let key = self.get_or_create_chunk(cell, i);
            let count = r.i16()?;
            for _ in 0..count {
                let mut va = self.new_virtual_animal(cx.host.rand());
                let host = &mut *cx.host;
                va.load(&*cx.meta, &mut r, world_version, &mut |r, v| host.load_animal(r, v))?;
                va.init_value_from_def(cx.defs).map_err(|e| pz_world::save::io::SaveError(e.0))?;
                for (j, a) in va.animals.iter().enumerate() {
                    if a.attach_back_to_mother() > 0 {
                        reattach.push((va.uid, j));
                    }
                }
                let uid = va.uid;
                self.animals.insert(uid, va);
                self.chunk_mut(key).animals.push(uid);
            }
            let count = r.i16()?;
            for _ in 0..count {
                let t = AnimalTracks::load(&mut r)?;
                self.chunk_mut(key).animal_tracks.push(t);
            }
            let chunk = self.chunk(key);
            if !chunk.is_empty() {
                let pos = (chunk.x, chunk.y);
                self.chunks_with_tracks.insert(pos);
                continue;
            }
            let chunk = self.cell_mut(cell).chunks.as_mut().unwrap()[i].take().unwrap();
            self.release_chunk(chunk);
        }
        Ok(())
    }

    /// The `animalListToReattach` pass of `AnimalCell.load(fileName)`.
    fn reattach_mothers(&mut self, cell: usize, reattach: Vec<(u64, usize)>, cx: &mut PopCx<'_, A>) {
        if reattach.is_empty() {
            return;
        }
        // animals: animalId -> (herd, index), later herds overwriting.
        let mut by_id: HashMap<i32, (u64, usize)> = HashMap::new();
        for chunk in self.cell(cell).chunks.iter().flatten().flatten() {
            for u in &chunk.animals {
                if let Some(va) = self.animals.get(u) {
                    for (k, a) in va.animals.iter().enumerate() {
                        by_id.insert(a.animal_id(), (*u, k));
                    }
                }
            }
        }
        for (bu, bj) in reattach {
            let Some(baby) = self.animals.get(&bu).map(|v| v.animals[bj].attach_back_to_mother()) else {
                continue;
            };
            let Some(&(mu, mj)) = by_id.get(&baby) else {
                continue;
            };
            if (mu, mj) == (bu, bj) {
                continue;
            }
            if mu == bu {
                let va = self.animals.get_mut(&bu).unwrap();
                let (lo, hi) = (bj.min(mj), bj.max(mj));
                let (a, b) = va.animals.split_at_mut(hi);
                let (first, second) = (&mut a[lo], &mut b[0]);
                if bj < mj {
                    cx.host.set_mother(first, second);
                } else {
                    cx.host.set_mother(second, first);
                }
            } else {
                let mut mother_herd = self.animals.remove(&mu).unwrap();
                let baby_herd = self.animals.get_mut(&bu).unwrap();
                cx.host.set_mother(&mut baby_herd.animals[bj], &mut mother_herd.animals[mj]);
                self.animals.insert(mu, mother_herd);
            }
        }
    }

    /// `AnimalCell.save()`.
    fn save_cell(&mut self, cell: usize, cx: &mut PopCx<'_, A>) {
        if !self.cell(cell).loaded || cx.host.no_save() {
            return;
        }
        let (x, y) = (self.cell(cell).x, self.cell(cell).y);
        match self.cell_bytes(cell, cx) {
            Ok(b) => {
                if let Err(e) = cx.host.write_apop(x, y, &b) {
                    debug_log(&format!("AnimalCell.save apop_{x}_{y}.bin: {e}"));
                }
            }
            Err(e) => debug_log(&format!("AnimalCell.save apop_{x}_{y}.bin: {e}")),
        }
    }

    /// `AnimalCell.save(ByteBuffer)`: the file's bytes. The real animals of
    /// `saveRealAnimalHack` are written once and dropped.
    fn cell_bytes(&mut self, cell: usize, cx: &mut PopCx<'_, A>) -> SaveResult<Vec<u8>> {
        let hack = self.cell_mut(cell).save_real_animal_hack.take().unwrap_or_default();
        for u in &hack {
            let (x, y) = (fastfloor(self.animals[u].x), fastfloor(self.animals[u].y));
            let _ = self.get_or_create_chunk_from_square_pos(cell, x, y);
        }
        let result = self.write_cell(cell, &hack, cx);
        for u in hack {
            self.animals.remove(&u);
        }
        result
    }

    fn write_cell(&self, cell: usize, hack: &[u64], cx: &mut PopCx<'_, A>) -> SaveResult<Vec<u8>> {
        let mut w = Writer::new();
        w.i32(APOP_WORLD_VERSION);
        let c = self.cell(cell);
        let host = &mut *cx.host;
        let meta = &*cx.meta;
        let mut save = |a: &A, w: &mut Writer| host.save_animal(a, w);
        for (i, chunk) in c.chunks.as_ref().unwrap().iter().enumerate() {
            let Some(chunk) = chunk else {
                w.i16(0);
                w.i16(0);
                continue;
            };
            let real: Vec<u64> = hack
                .iter()
                .copied()
                .filter(|u| {
                    let va = &self.animals[u];
                    c.chunk_index_from_square(fastfloor(va.x), fastfloor(va.y)) == Some(i)
                })
                .collect();
            w.i16((chunk.animals.len() + real.len()) as i16);
            for u in chunk.animals.iter().chain(&real) {
                self.animals[u].save(meta, &mut w, &mut save)?;
            }
            w.i16(chunk.animal_tracks.len() as i16);
            for t in &chunk.animal_tracks {
                t.save(&mut w);
            }
        }
        Ok(w.buf)
    }

    // ---- AnimalChunk ----

    /// `AnimalChunk.updateTracks()`.
    fn update_tracks(&mut self, key: ChunkKey, cx: &mut PopCx<'_, A>) {
        let now = cx.host.calendar_millis();
        let m = cx.host.multiplier();
        let chunk = self.chunk_mut(key);
        if chunk.animal_tracks.is_empty() {
            return;
        }
        if chunk.tracks_update_timer <= 0.0 {
            chunk.tracks_update_timer = 5000.0;
            // getTrackAgeDays() >= 4 goes.
            chunk
                .animal_tracks
                .retain(|t| fastfloor(((now - t.added_time) / 60_000) as f32) / 1440 < 4);
        }
        chunk.tracks_update_timer -= m;
    }

    /// `AnimalChunk.addTracks(animal, trackType)`.
    fn chunk_add_tracks(&mut self, key: ChunkKey, va: &VirtualAnimal<A>, tt: &AnimalTracksType, cx: &mut PopCx<'_, A>) {
        let dist = self.closest_zone_dist(&*cx.meta, va.x, va.y);
        if dist == -1.0 || dist > 20.0 {
            return;
        }
        let track = self.add_animal_track(va, tt, cx);
        let chunk = self.chunk_mut(key);
        chunk.animal_tracks.push(track);
        if chunk.animals.len() + chunk.animal_tracks.len() == 1 {
            let pos = (chunk.x, chunk.y);
            self.chunks_with_tracks.insert(pos);
        }
    }

    /// `AnimalTracks.addAnimalTrack(animal, trackType)`.
    fn add_animal_track(&self, va: &VirtualAnimal<A>, tt: &AnimalTracksType, cx: &mut PopCx<'_, A>) -> AnimalTracks {
        let rand = cx.host.rand();
        let x = rand.next_range(va.x as i32 - 2, va.x as i32 + 2);
        let y = rand.next_range(va.y as i32 - 2, va.y as i32 + 2);
        new_track(va, tt, x, y, cx.host.calendar_millis())
    }

    // ---- AnimalZones ----

    /// `AnimalZones.createJunctions(cell)`.
    pub fn create_junctions(&mut self, cell: usize, meta: &mut IsoMetaGrid) {
        if self.cell(cell).added_junctions {
            return;
        }
        self.cell_mut(cell).added_junctions = true;
        let (x, y) = (self.cell(cell).x, self.cell(cell).y);
        let Some(zones) = meta.get_cell_data(x, y).map(|m| m.animal_zones.clone()) else {
            return;
        };
        for (i, &a) in zones.iter().enumerate() {
            for &b in &zones[i + 1..] {
                if meta.zone(b).is_polyline() {
                    zone::add_junctions_with_other_zone(meta, a, b);
                }
            }
        }
        for dy in -1..=1 {
            for dx in -1..=1 {
                let Some(c2) = self.cell_index_from_cell_pos(x + dx, y + dy) else {
                    continue;
                };
                if c2 == cell {
                    continue;
                }
                let (x2, y2) = (self.cell(c2).x, self.cell(c2).y);
                let Some(zones2) = meta.get_cell_data(x2, y2).map(|m| m.animal_zones.clone()) else {
                    continue;
                };
                for &a in &zones {
                    for &b in &zones2 {
                        zone::add_junctions_with_other_zone(meta, a, b);
                    }
                }
            }
        }
    }

    /// `spawnAnimalsInCell(cell)`.
    fn spawn_animals_in_cell(&mut self, cell: usize, cx: &mut PopCx<'_, A>) -> JResult<()> {
        let (x, y) = (self.cell(cell).x, self.cell(cell).y);
        let Some(zones) = cx.meta.get_cell_data(x, y).map(|m| m.animal_zones.clone()) else {
            return Ok(());
        };
        for z in zones {
            self.spawn_animals_on_zone(z, cx)?;
        }
        Ok(())
    }

    /// `spawnAnimalsOnZone(zone)`.
    fn spawn_animals_on_zone(&mut self, zone_id: ZoneId, cx: &mut PopCx<'_, A>) -> JResult<()> {
        let animal_type = {
            let pz_world::zones::ZoneKind::Animal(a) = &mut cx.meta.zone_mut(zone_id).kind else {
                return Ok(());
            };
            if a.spawned_animals || !a.spawn_animal {
                return Ok(());
            }
            a.spawned_animals = true;
            if a.action.as_deref() != Some("Follow") {
                return Ok(());
            }
            a.animal_type.clone()
        };
        let t1 = cx.host.rand().next_float_range(0.0, 1.0);
        let (ok, pos) = zone::point_on_polyline(cx.meta.zone(zone_id), t1);
        if !ok {
            return Ok(());
        }
        let mut va = self.new_virtual_animal(cx.host.rand());
        va.x = pos.0;
        va.y = pos.1;
        va.zone = Some(zone_id);
        self.generate_possible_animals(&mut va, animal_type.as_deref(), cx)?;
        va.init_value_from_def(cx.defs)?;
        let chance = cx.host.animal_track_chance();
        if chance > 1 {
            let rand = cx.host.rand();
            let mut random_tracks = rand.next_range(6, 12);
            match chance {
                2 => random_tracks = rand.next_range(0, 2),
                3 => random_tracks = rand.next_range(2, 4),
                4 => random_tracks = rand.next_range(2, 7),
                5 => random_tracks = rand.next_range(4, 10),
                _ => {}
            }
            for _ in 0..random_tracks {
                let rand = cx.host.rand();
                let type_ = match rand.next(3) {
                    0 => "poop",
                    1 => "brokentwigs",
                    _ => "footstep",
                };
                let t2 = rand.next_float_range(0.0, 1.0);
                let (_, pos) = zone::point_on_polyline(cx.meta.zone(zone_id), t2);
                let hours = cx.host.rand().next_range(1, 5) as i64;
                self.add_zone_track(zone_id, t2, &mut va, Some(pos), type_, 3_600_000 * hours, cx)?;
            }
            self.add_zone_track(zone_id, t1, &mut va, None, "footstep", 0, cx)?;
        }
        self.add_animal(va, cx);
        Ok(())
    }

    /// `AnimalZones.addTrack(...)` (both overloads: `pos` given for
    /// `addAnimalTrackAtPos`).
    #[allow(clippy::too_many_arguments)]
    fn add_zone_track(
        &mut self,
        zone_id: ZoneId,
        t: f32,
        va: &mut VirtualAnimal<A>,
        pos: Option<(f32, f32)>,
        type_: &str,
        time_minus: i64,
        cx: &mut PopCx<'_, A>,
    ) -> JResult<()> {
        if let Some(d) = zone::direction_on_polyline(cx.meta.zone(zone_id), t) {
            va.forward_direction = d;
        }
        if !va.move_forward_on_zone {
            va.forward_direction = (va.forward_direction.0 * -1.0, va.forward_direction.1 * -1.0);
        }
        let Some(def) = va.migration_group.as_deref().and_then(|g| cx.defs.tracks.get(g)) else {
            return Ok(());
        };
        let tt = def
            .tracks
            .as_ref()
            .and_then(|t| t.get(type_))
            .ok_or_else(|| JavaException("NullPointerException: trackType".into()))?;
        let track = match pos {
            Some((px, py)) => new_track(va, tt, fastfloor(px), fastfloor(py), cx.host.calendar_millis() - time_minus),
            None => self.add_animal_track(va, tt, cx),
        };
        if let Some(key) = self.get_or_create_animal_chunk(track.x as f32, track.y as f32, cx) {
            let chunk = self.chunk_mut(key);
            chunk.animal_tracks.push(track);
            if chunk.animals.len() + chunk.animal_tracks.len() == 1 {
                let pos = (chunk.x, chunk.y);
                self.chunks_with_tracks.insert(pos);
            }
        }
        Ok(())
    }

    /// `MigrationGroupDefinitions.generatePossibleAnimals(vAnimal, type)`.
    fn generate_possible_animals(
        &mut self,
        va: &mut VirtualAnimal<A>,
        group_type: Option<&str>,
        cx: &mut PopCx<'_, A>,
    ) -> JResult<()> {
        let Some(def) = group_type.and_then(|t| cx.defs.migration.get(t)) else {
            return Ok(());
        };
        if def.group.as_ref().is_some_and(|g| !g.is_empty()) {
            // generatePossibleAnimalsFromGroup.
            let total: i32 = def.group.as_ref().unwrap().iter().map(|g| g.chance).sum();
            let r = cx.host.rand().next(total);
            let mut index = 0;
            let mut chosen = None;
            for g in def.group.as_ref().unwrap() {
                chosen = g.animal.as_deref().and_then(|a| cx.defs.migration.get(a));
                if g.chance + index >= r {
                    break;
                }
                index += g.chance;
                chosen = None;
            }
            let Some(chosen) = chosen else {
                return Ok(());
            };
            let t = chosen.type_.clone();
            return self.generate_possible_animals(va, t.as_deref(), cx);
        }
        let group_type = group_type.unwrap();
        let virtual_id = (cx.host.rand().next(1_000_000) + 1_000_000) as f64;
        va.migration_group = Some(group_type.to_owned());
        let mut result = Vec::new();
        let male = def.male.clone().unwrap_or_default();
        for _ in 0..def.max_male {
            let breed = def.rand_breed(cx.host.rand())?;
            let mut a = cx.host.new_animal(&male, &breed)?;
            cx.host.set_virtual_id(&mut a, virtual_id, group_type);
            result.push(a);
        }
        let count = cx.host.rand().next_range(def.min_animal, def.max_animal + 1);
        let female = def.female.clone().unwrap_or_default();
        for _ in 0..count {
            let breed = def.rand_breed(cx.host.rand())?;
            let mut a = cx.host.new_animal(&female, &breed)?;
            cx.host.randomize_age(&mut a)?;
            cx.host.set_virtual_id(&mut a, virtual_id, group_type);
            let baby = if cx.host.can_have_baby(&a) && cx.host.rand().next(100) < def.baby_chance {
                cx.host.add_baby(&mut a)?
            } else {
                None
            };
            result.push(a);
            result.extend(baby);
        }
        va.animals.extend(result);
        Ok(())
    }

    /// `getClosestZoneDist(x, y)` (-1 outside the cells).
    pub fn closest_zone_dist(&self, meta: &IsoMetaGrid, x: f32, y: f32) -> f32 {
        let Some(cell) = self.cell_index_from_square_pos(x as i32, y as i32) else {
            return -1.0;
        };
        let c = self.cell(cell);
        let Some(m) = meta.get_cell_data(c.x, c.y) else {
            return -1.0;
        };
        let mut closest = f32::MAX;
        for &z in &m.animal_zones {
            let mut pos = (0.0, 0.0);
            let t = zone::closest_point_on_polyline(meta.zone(z), x, y, &mut pos);
            if t < 0.0 {
                continue;
            }
            let (dx, dy) = (x - pos.0, y - pos.1);
            let dist = ((dx * dx + dy * dy) as f64).sqrt() as f32;
            if dist < closest {
                closest = dist;
            }
        }
        closest
    }

    /// `getClosestZone(x, y, action)` (`action` null: any).
    pub fn closest_zone(&self, meta: &IsoMetaGrid, x: f32, y: f32, action: Option<&str>) -> Option<ZoneId> {
        let cell = self.cell_index_from_square_pos(x as i32, y as i32)?;
        let c = self.cell(cell);
        let m = meta.get_cell_data(c.x, c.y)?;
        let mut closest = None;
        let mut closest_dist = f32::MAX;
        for &z in &m.animal_zones {
            if action.is_some() && action != zone::action(meta.zone(z)) {
                continue;
            }
            let mut pos = (0.0, 0.0);
            let t = zone::closest_point_on_polyline(meta.zone(z), x, y, &mut pos);
            if t < 0.0 {
                continue;
            }
            let (dx, dy) = (x - pos.0, y - pos.1);
            let dist = ((dx * dx + dy * dy) as f64).sqrt() as f32;
            if dist < closest_dist {
                closest_dist = dist;
                closest = Some(z);
            }
        }
        closest
    }

    /// `getClosestJunction(x, y)`.
    pub fn closest_junction(&self, meta: &IsoMetaGrid, x: i32, y: i32) -> Option<zone::Junction> {
        let z = self.closest_zone(meta, x as f32, y as f32, None)?;
        let js = zone::junctions(meta, z);
        let mut result = *js.first()?;
        let mut previous = 100.0f64;
        for j in &js {
            let (jx, jy) = j.xy(meta);
            let d = (((jy - y) * (jy - y) + (jx - x) * (jx - x)) as f64).sqrt();
            if d < previous && d < 30.0 {
                previous = d;
                result = *j;
            }
        }
        Some(result)
    }

    /// `AnimalZones.updateVirtualAnimals()`.
    pub fn update_virtual_animals(&mut self, cx: &mut PopCx<'_, A>) -> PopulationResult<()> {
        let snapshot: Vec<(i32, i32)> = self.chunks_with_tracks.iter().copied().collect();
        let mut done = HashSet::new();
        for (wx, wy) in snapshot {
            let Some(cell) = self.cell_index_from_chunk_pos(wx, wy) else {
                continue;
            };
            let c = self.cell(cell);
            let Some(k) = c.chunk_at(c.chunk_index_from_chunk(wx, wy)) else {
                continue;
            };
            let key = (cell, k);
            let temp = self.chunk(key).animals.clone();
            self.update_tracks(key, cx);
            for uid in temp {
                let Some(mut va) = self.animals.remove(&uid) else {
                    continue;
                };
                if va.removed {
                    if let Some(chunk) = self.try_chunk_mut(key) {
                        chunk.animals.retain(|&u| u != uid);
                    }
                    continue;
                }
                if !done.insert(uid) {
                    self.animals.insert(uid, va);
                    continue;
                }
                self.pass_to_main = false;
                let r = self.update_virtual_animal(&mut va, cx);
                if std::mem::take(&mut self.pass_to_main) {
                    cx.host.from_worker(&*cx.meta, vec![va]);
                } else if self.is_placed(uid, key, va.x, va.y) {
                    self.animals.insert(uid, va);
                }
                r?;
            }
            if self.try_chunk_mut(key).is_some_and(|c| c.is_empty()) {
                self.release_chunk_if_empty(key);
            }
        }
        Ok(())
    }

    fn try_chunk_mut(&mut self, (c, k): ChunkKey) -> Option<&mut AnimalChunk> {
        self.cells.as_mut()?.get_mut(c)?.as_mut()?.chunks.as_mut()?.get_mut(k)?.as_mut()
    }

    /// Whether the chunk it was in, or the chunk at its position, holds
    /// the virtual animal `uid`.
    fn is_placed(&self, uid: u64, key: ChunkKey, x: f32, y: f32) -> bool {
        let holds = |k: ChunkKey| {
            self.cells.as_ref().and_then(|c| c.get(k.0)?.as_ref()?.chunks.as_ref()?.get(k.1)?.as_ref())
                .is_some_and(|ch| ch.animals.contains(&uid))
        };
        if holds(key) {
            return true;
        }
        let (sx, sy) = (fastfloor(x), fastfloor(y));
        self.cell_index_from_square_pos(sx, sy).is_some_and(|c| {
            self.cell(c).chunk_index_from_square(sx, sy).is_some_and(|k| holds((c, k)))
        })
    }

    /// `AnimalZones.Reset()`.
    pub fn reset_zones(&mut self) {
        self.chunks_with_tracks.clear();
    }
}

impl<A> VirtualAnimal<A> {
    fn clone_without_animals(&self) -> VirtualAnimal<A> {
        VirtualAnimal {
            uid: self.uid,
            x: self.x,
            y: self.y,
            z: self.z,
            forward_direction: self.forward_direction,
            animals: Vec::new(),
            state: self.state,
            move_forward_on_zone: self.move_forward_on_zone,
            current_zone_action: self.current_zone_action.clone(),
            next_rest_time: self.next_rest_time,
            next_eat_time: self.next_eat_time,
            id: self.id,
            migration_group: self.migration_group.clone(),
            speed: self.speed,
            time_to_eat: self.time_to_eat,
            time_to_sleep: self.time_to_sleep,
            track_chance: self.track_chance,
            poop_chance: self.poop_chance,
            broken_twigs_chance: self.broken_twigs_chance,
            herb_graze_chance: self.herb_graze_chance,
            fur_chance: self.fur_chance,
            flat_herb_chance: self.flat_herb_chance,
            wake_time: self.wake_time,
            eat_start_time: self.eat_start_time,
            sleep_period_start: self.sleep_period_start.clone(),
            sleep_period_end: self.sleep_period_end.clone(),
            eat_period_start: self.eat_period_start.clone(),
            eat_period_end: self.eat_period_end.clone(),
            debug_force_sleep: self.debug_force_sleep,
            debug_force_eat: self.debug_force_eat,
            zone: self.zone,
            removed: self.removed,
        }
    }
}

#[cfg(test)]
mod tests;
