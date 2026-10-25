//! `zombie.iso.areas.isoregion.regions`: `IsoChunkRegion`,
//! `IsoWorldRegion` and `IsoRegionManager`.
//!
//! The original links regions by object reference and pools released
//! objects for reuse. Here both kinds live in arenas owned by the manager;
//! a [`ChunkRegionRef`] / [`WorldRegionRef`] is an arena slot, which plays
//! the part of the object identity: a released region goes back to its
//! pool and the slot is handed out again by the next allocation, as the
//! original reuses the pooled object. Ids come from one counter and one
//! free-id stack shared by both kinds, as upstream.
//!
//! The lazily cached "enclosed" values are [`Cell`]s because the original
//! updates them from its getters (`getIsEnclosed`, `isEnclosed`), which
//! callers like the renderer reach through shared references.
//!
//! Not ported: the debug colours (`getColor`, `Colors`), and the
//! `BuildingDef` a region gets from `WorldRegionToMetaGrid` (that class is
//! not ported, so it is always `null` and `clearBuildingDef` does nothing).

use std::cell::Cell;

use super::data::ChunkIdx;

/// An `IsoChunkRegion` object (its arena slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkRegionRef(pub u32);

/// An `IsoWorldRegion` object (its arena slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorldRegionRef(pub u32);

/// `IsoChunkRegion`: the squares of one chunk level that connect without
/// crossing a wall.
#[derive(Debug)]
pub struct IsoChunkRegion {
    in_pool: bool,
    id: i32,
    data_chunk: Option<ChunkIdx>,
    z_layer: i8,
    square_size: i8,
    roof_cnt: i8,
    chunk_border_squares_cnt: i8,
    enclosed: [bool; 4],
    enclosed_cache: Cell<bool>,
    connected_neighbors: Vec<ChunkRegionRef>,
    all_neighbors: Vec<ChunkRegionRef>,
    is_dirty_enclosed: Cell<bool>,
    iso_world_region: Option<WorldRegionRef>,
}

impl IsoChunkRegion {
    fn new() -> Self {
        IsoChunkRegion {
            in_pool: false,
            id: 0,
            data_chunk: None,
            z_layer: 0,
            square_size: 0,
            roof_cnt: 0,
            chunk_border_squares_cnt: 0,
            enclosed: [false; 4],
            enclosed_cache: Cell::new(true),
            connected_neighbors: Vec::new(),
            all_neighbors: Vec::new(),
            is_dirty_enclosed: Cell::new(false),
            iso_world_region: None,
        }
    }
    /// `getID()`.
    pub fn id(&self) -> i32 {
        self.id
    }
    /// `getSquareSize()`.
    pub fn square_size(&self) -> i32 {
        self.square_size as i32
    }
    /// `getzLayer()`.
    pub fn z_layer(&self) -> i32 {
        self.z_layer as i32
    }
    /// `getRoofCnt()`.
    pub fn roof_cnt(&self) -> i32 {
        self.roof_cnt as i32
    }
    /// `getChunkBorderSquaresCnt()`.
    pub fn chunk_border_squares_cnt(&self) -> i32 {
        self.chunk_border_squares_cnt as i32
    }
    /// `getIsoWorldRegion()`.
    pub fn iso_world_region(&self) -> Option<WorldRegionRef> {
        self.iso_world_region
    }
    /// `getDataChunk()`.
    pub fn data_chunk(&self) -> Option<ChunkIdx> {
        self.data_chunk
    }
    /// `getConnectedNeighbors()`.
    pub fn connected_neighbors(&self) -> &[ChunkRegionRef] {
        &self.connected_neighbors
    }
    /// `getAllNeighbors()`.
    pub fn all_neighbors(&self) -> &[ChunkRegionRef] {
        &self.all_neighbors
    }
    /// `getNeighborCount()`.
    pub fn neighbor_count(&self) -> usize {
        self.all_neighbors.len()
    }
    /// `isInPool()`.
    pub fn is_in_pool(&self) -> bool {
        self.in_pool
    }
    /// `setEnclosed(dir, b)`.
    pub(crate) fn set_enclosed(&mut self, dir: i8, b: bool) {
        self.is_dirty_enclosed.set(true);
        self.enclosed[dir as usize] = b;
    }
    fn reset_enclosed(&mut self) {
        self.enclosed = [true; 4];
        self.is_dirty_enclosed.set(false);
        self.enclosed_cache.set(true);
    }
    pub(crate) fn add_square_count(&mut self) {
        self.square_size = self.square_size.wrapping_add(1);
    }
    pub(crate) fn add_chunk_border_squares_cnt(&mut self) {
        self.chunk_border_squares_cnt = self.chunk_border_squares_cnt.wrapping_add(1);
    }
    pub(crate) fn add_connected_neighbor(&mut self, n: ChunkRegionRef) {
        if !self.connected_neighbors.contains(&n) {
            self.connected_neighbors.push(n);
        }
    }
    pub(crate) fn add_neighbor(&mut self, n: ChunkRegionRef) {
        if !self.all_neighbors.contains(&n) {
            self.all_neighbors.push(n);
        }
    }
    pub(crate) fn set_iso_world_region(&mut self, r: Option<WorldRegionRef>) {
        self.iso_world_region = r;
    }
}

fn remove_first<T: PartialEq>(v: &mut Vec<T>, x: &T) -> bool {
    if let Some(i) = v.iter().position(|e| e == x) {
        v.remove(i);
        true
    } else {
        false
    }
}

/// `IsoWorldRegion`: chunk regions joined across chunk edges.
#[derive(Debug)]
pub struct IsoWorldRegion {
    in_pool: bool,
    id: i32,
    enclosed: Cell<bool>,
    iso_chunk_regions: Vec<ChunkRegionRef>,
    square_size: i32,
    roof_cnt: i32,
    is_dirty_enclosed: Cell<bool>,
    is_dirty_roofed: bool,
    neighbors: Vec<WorldRegionRef>,
}

impl IsoWorldRegion {
    fn new() -> Self {
        IsoWorldRegion {
            in_pool: false,
            id: 0,
            enclosed: Cell::new(true),
            iso_chunk_regions: Vec::new(),
            square_size: 0,
            roof_cnt: 0,
            is_dirty_enclosed: Cell::new(false),
            is_dirty_roofed: false,
            neighbors: Vec::new(),
        }
    }
    /// `getID()`.
    pub fn id(&self) -> i32 {
        self.id
    }
    /// `size()`: the number of chunk regions.
    pub fn size(&self) -> usize {
        self.iso_chunk_regions.len()
    }
    /// `getSquareSize()`.
    pub fn square_size(&self) -> i32 {
        self.square_size
    }
    /// `getRoofCnt()`.
    pub fn roof_cnt(&self) -> i32 {
        self.roof_cnt
    }
    /// `isFullyRoofed()`.
    pub fn is_fully_roofed(&self) -> bool {
        self.roof_cnt == self.square_size
    }
    /// `getRoofedPercentage()`.
    pub fn roofed_percentage(&self) -> f32 {
        if self.square_size == 0 {
            return 0.0;
        }
        self.roof_cnt as f32 / self.square_size as f32
    }
    /// `getNeighbors()`.
    pub fn neighbors(&self) -> &[WorldRegionRef] {
        &self.neighbors
    }
    /// `getChunkRegions()`.
    pub fn chunk_regions(&self) -> &[ChunkRegionRef] {
        &self.iso_chunk_regions
    }
    /// `isInPool()`.
    pub fn is_in_pool(&self) -> bool {
        self.in_pool
    }
    /// `isDirtyRoofed` (never set by the original).
    pub fn is_dirty_roofed(&self) -> bool {
        self.is_dirty_roofed
    }
    fn add_neighbor(&mut self, n: WorldRegionRef) {
        if !self.neighbors.contains(&n) {
            self.neighbors.push(n);
        }
    }
    fn add_roof(&mut self, warn: &mut Vec<String>) {
        self.roof_cnt += 1;
        if self.roof_cnt > self.square_size {
            warn.push("WorldRegion.addRoof roofCount exceed squareSize.".into());
            self.roof_cnt = self.square_size;
        }
    }
    fn remove_roofs(&mut self, roofs: i32, warn: &mut Vec<String>) {
        if roofs <= 0 {
            return;
        }
        self.roof_cnt -= roofs;
        if self.roof_cnt < 0 {
            warn.push("MasterRegion.removeRoofs Roofcount managed to get below zero.".into());
            self.roof_cnt = 0;
        }
    }
}

/// `IsoRegionManager`: allocation, pools and ids of both region kinds.
///
/// It also holds `DataRoot.dirtyIsoWorldRegions`, which
/// `releaseIsoWorldRegion` edits.
#[derive(Debug, Default)]
pub struct IsoRegionManager {
    chunk_regions: Vec<IsoChunkRegion>,
    world_regions: Vec<IsoWorldRegion>,
    pool_iso_world_region: Vec<WorldRegionRef>,
    pool_iso_chunk_region: Vec<ChunkRegionRef>,
    region_id_stack: Vec<i32>,
    next_id: i32,
    world_region_count: i32,
    chunk_region_count: i32,
    /// `DataRoot.dirtyIsoWorldRegions`.
    pub(crate) dirty_iso_world_regions: Vec<WorldRegionRef>,
    /// `IsoRegions.warn` messages, oldest first.
    pub warnings: Vec<String>,
}

impl IsoRegionManager {
    /// The chunk region object `r`.
    pub fn chunk_region(&self, r: ChunkRegionRef) -> &IsoChunkRegion {
        &self.chunk_regions[r.0 as usize]
    }
    pub(crate) fn cr(&mut self, r: ChunkRegionRef) -> &mut IsoChunkRegion {
        &mut self.chunk_regions[r.0 as usize]
    }
    /// The world region object `r`.
    pub fn world_region(&self, r: WorldRegionRef) -> &IsoWorldRegion {
        &self.world_regions[r.0 as usize]
    }
    fn wr(&mut self, r: WorldRegionRef) -> &mut IsoWorldRegion {
        &mut self.world_regions[r.0 as usize]
    }
    /// `getWorldRegionCount()`.
    pub fn world_region_count(&self) -> i32 {
        self.world_region_count
    }
    /// `getChunkRegionCount()`.
    pub fn chunk_region_count(&self) -> i32 {
        self.chunk_region_count
    }
    /// `nextId`: the next id when the free-id stack is empty.
    pub fn next_id(&self) -> i32 {
        self.next_id
    }
    /// `regionIdStack`, top first (the order the ids are taken back).
    pub fn region_id_stack(&self) -> impl Iterator<Item = i32> + '_ {
        self.region_id_stack.iter().rev().copied()
    }

    fn next_region_id(&mut self) -> i32 {
        match self.region_id_stack.pop() {
            Some(id) => id,
            None => {
                let id = self.next_id;
                self.next_id += 1;
                id
            }
        }
    }

    /// `allocIsoWorldRegion()`.
    pub(crate) fn alloc_iso_world_region(&mut self) -> WorldRegionRef {
        let r = match self.pool_iso_world_region.pop() {
            Some(r) => r,
            None => {
                self.world_regions.push(IsoWorldRegion::new());
                WorldRegionRef(self.world_regions.len() as u32 - 1)
            }
        };
        let id = self.next_region_id();
        // `init(id)`.
        let w = self.wr(r);
        w.in_pool = false;
        w.id = id;
        w.square_size = 0;
        w.roof_cnt = 0;
        w.enclosed.set(true);
        w.is_dirty_enclosed.set(false);
        w.is_dirty_roofed = false;
        self.world_region_count += 1;
        r
    }

    /// `releaseIsoWorldRegion(worldRegion)`.
    pub(crate) fn release_iso_world_region(&mut self, r: WorldRegionRef) {
        remove_first(&mut self.dirty_iso_world_regions, &r);
        if !self.world_region(r).in_pool {
            let id = self.world_region(r).id;
            self.region_id_stack.push(id);
            self.reset_world_region(r);
            self.pool_iso_world_region.push(r);
            self.world_region_count -= 1;
        } else {
            self.warnings
                .push("IsoRegionManager -> Trying to release a MasterRegion twice.".into());
        }
    }

    /// `IsoWorldRegion.reset()` (non-debug build: a region that still has
    /// chunk regions only warns).
    fn reset_world_region(&mut self, r: WorldRegionRef) {
        {
            let w = self.wr(r);
            w.in_pool = true;
            w.id = -1;
            w.square_size = 0;
            w.roof_cnt = 0;
            w.enclosed.set(true);
            w.is_dirty_roofed = false;
            w.is_dirty_enclosed.set(false);
        }
        self.unlink_world_neighbors(r);
        if !self.world_region(r).iso_chunk_regions.is_empty() {
            self.warnings.push(
                "MasterRegion.reset Resetting master region which still has chunk regions".into(),
            );
            let list = std::mem::take(&mut self.wr(r).iso_chunk_regions);
            for c in list {
                self.cr(c).iso_world_region = None;
            }
        }
    }

    /// `allocIsoChunkRegion(dataChunk, zLayer)`.
    pub(crate) fn alloc_iso_chunk_region(&mut self, chunk: ChunkIdx, z: i32) -> ChunkRegionRef {
        let r = match self.pool_iso_chunk_region.pop() {
            Some(r) => r,
            None => {
                self.chunk_regions.push(IsoChunkRegion::new());
                ChunkRegionRef(self.chunk_regions.len() as u32 - 1)
            }
        };
        let id = self.next_region_id();
        // `init(id, dataChunk, zLayer)`.
        let c = self.cr(r);
        c.in_pool = false;
        c.id = id;
        c.data_chunk = Some(chunk);
        c.z_layer = z as i8;
        c.chunk_border_squares_cnt = 0;
        c.square_size = 0;
        c.roof_cnt = 0;
        c.reset_enclosed();
        self.chunk_region_count += 1;
        r
    }

    /// `releaseIsoChunkRegion(chunkRegion)`.
    pub(crate) fn release_iso_chunk_region(&mut self, r: ChunkRegionRef) {
        if !self.chunk_region(r).in_pool {
            let id = self.chunk_region(r).id;
            self.region_id_stack.push(id);
            self.reset_chunk_region(r);
            self.pool_iso_chunk_region.push(r);
            self.chunk_region_count -= 1;
        } else {
            self.warnings
                .push("IsoRegionManager -> Trying to release a ChunkRegion twice.".into());
        }
    }

    /// `IsoChunkRegion.reset()` (non-debug build).
    fn reset_chunk_region(&mut self, r: ChunkRegionRef) {
        self.cr(r).in_pool = true;
        self.unlink_chunk_neighbors(r);
        if let Some(mr) = self.unlink_from_iso_world_region(r) {
            if self.world_region(mr).size() == 0 {
                self.release_iso_world_region(mr);
                self.warnings.push(
                    "ChunkRegion.reset IsoChunkRegion has IsoWorldRegion with 0 members.".into(),
                );
            }
        }
        let c = self.cr(r);
        c.chunk_border_squares_cnt = 0;
        c.id = -1;
        c.square_size = 0;
        c.roof_cnt = 0;
        c.reset_enclosed();
    }

    /// `IsoChunkRegion.unlinkNeighbors()`.
    fn unlink_chunk_neighbors(&mut self, r: ChunkRegionRef) {
        let mut i = 0;
        while i < self.chunk_region(r).connected_neighbors.len() {
            let n = self.chunk_region(r).connected_neighbors[i];
            remove_first(&mut self.cr(n).connected_neighbors, &r);
            i += 1;
        }
        self.cr(r).connected_neighbors.clear();
        let mut i = 0;
        while i < self.chunk_region(r).all_neighbors.len() {
            let n = self.chunk_region(r).all_neighbors[i];
            remove_first(&mut self.cr(n).all_neighbors, &r);
            i += 1;
        }
        self.cr(r).all_neighbors.clear();
    }

    /// `IsoChunkRegion.unlinkFromIsoWorldRegion()`.
    pub(crate) fn unlink_from_iso_world_region(
        &mut self,
        r: ChunkRegionRef,
    ) -> Option<WorldRegionRef> {
        let mr = self.chunk_region(r).iso_world_region?;
        self.remove_iso_chunk_region(mr, r);
        self.cr(r).iso_world_region = None;
        Some(mr)
    }

    /// `IsoChunkRegion.getIsEnclosed()`: recomputed when dirty, which also
    /// marks its world region dirty.
    pub fn chunk_region_is_enclosed(&self, r: ChunkRegionRef) -> bool {
        let c = self.chunk_region(r);
        if !c.is_dirty_enclosed.get() {
            return c.enclosed_cache.get();
        }
        c.is_dirty_enclosed.set(false);
        c.enclosed_cache.set(c.enclosed.iter().all(|&e| e));
        if let Some(w) = c.iso_world_region {
            self.world_region(w).is_dirty_enclosed.set(true);
        }
        c.enclosed_cache.get()
    }

    /// `IsoChunkRegion.setDirtyEnclosed()`.
    pub fn chunk_region_set_dirty_enclosed(&self, r: ChunkRegionRef) {
        let c = self.chunk_region(r);
        c.is_dirty_enclosed.set(true);
        if let Some(w) = c.iso_world_region {
            self.world_region(w).is_dirty_enclosed.set(true);
        }
    }

    /// `IsoChunkRegion.addRoof()`.
    pub(crate) fn chunk_region_add_roof(&mut self, r: ChunkRegionRef) {
        let c = &mut self.chunk_regions[r.0 as usize];
        c.roof_cnt = c.roof_cnt.wrapping_add(1);
        if c.roof_cnt > c.square_size {
            c.roof_cnt = c.square_size;
            self.warnings
                .push("ChunkRegion.addRoof roofCount exceed squareSize.".into());
            return;
        }
        if let Some(w) = c.iso_world_region {
            let w = &mut self.world_regions[w.0 as usize];
            w.add_roof(&mut self.warnings);
        }
    }

    /// `IsoChunkRegion.resetRoofCnt()`.
    pub(crate) fn chunk_region_reset_roof_cnt(&mut self, r: ChunkRegionRef) {
        let c = &mut self.chunk_regions[r.0 as usize];
        if let Some(w) = c.iso_world_region {
            let roofs = c.roof_cnt as i32;
            self.world_regions[w.0 as usize].remove_roofs(roofs, &mut self.warnings);
        }
        self.chunk_regions[r.0 as usize].roof_cnt = 0;
    }

    /// `IsoChunkRegion.getConnectedNeighborWithLargestIsoWorldRegion()`.
    pub fn connected_neighbor_with_largest_iso_world_region(
        &self,
        r: ChunkRegionRef,
    ) -> Option<ChunkRegionRef> {
        let mut largest: Option<WorldRegionRef> = None;
        let mut target = None;
        for &n in &self.chunk_region(r).connected_neighbors {
            let Some(mr) = self.chunk_region(n).iso_world_region else {
                continue;
            };
            if let Some(l) = largest {
                if self.world_region(mr).square_size <= self.world_region(l).square_size {
                    continue;
                }
            }
            largest = Some(mr);
            target = Some(n);
        }
        target
    }

    /// `IsoChunkRegion.getFirstNeighborWithIsoWorldRegion()`.
    pub fn first_neighbor_with_iso_world_region(
        &self,
        r: ChunkRegionRef,
    ) -> Option<ChunkRegionRef> {
        self.chunk_region(r)
            .connected_neighbors
            .iter()
            .copied()
            .find(|&n| self.chunk_region(n).iso_world_region.is_some())
    }

    /// `IsoWorldRegion.isEnclosed()`.
    pub fn world_region_is_enclosed(&self, r: WorldRegionRef) -> bool {
        let w = self.world_region(r);
        if w.is_dirty_enclosed.get() {
            // `recalcEnclosed()`.
            w.is_dirty_enclosed.set(false);
            w.enclosed.set(true);
            for &c in &w.iso_chunk_regions {
                if !self.chunk_region_is_enclosed(c) {
                    w.enclosed.set(false);
                }
            }
        }
        w.enclosed.get()
    }

    /// `IsoWorldRegion.isFogMask()`.
    pub fn world_region_is_fog_mask(&self, r: WorldRegionRef) -> bool {
        self.world_region_is_enclosed(r) && self.world_region(r).is_fully_roofed()
    }

    /// `IsoWorldRegion.isPlayerRoom()`.
    pub fn world_region_is_player_room(&self, r: WorldRegionRef) -> bool {
        self.world_region_is_fog_mask(r)
    }

    /// `IsoWorldRegion.setDirtyEnclosed()`.
    pub fn world_region_set_dirty_enclosed(&self, r: WorldRegionRef) {
        self.world_region(r).is_dirty_enclosed.set(true);
    }

    /// `IsoWorldRegion.unlinkNeighbors()`.
    pub(crate) fn unlink_world_neighbors(&mut self, r: WorldRegionRef) {
        let mut i = 0;
        while i < self.world_region(r).neighbors.len() {
            let n = self.world_region(r).neighbors[i];
            remove_first(&mut self.wr(n).neighbors, &r);
            i += 1;
        }
        self.wr(r).neighbors.clear();
    }

    /// `IsoWorldRegion.linkNeighbors()`.
    pub(crate) fn link_world_neighbors(&mut self, r: WorldRegionRef) {
        let crs = self.world_region(r).iso_chunk_regions.clone();
        for c in crs {
            let ns = self.chunk_region(c).all_neighbors.clone();
            for n in ns {
                let Some(nw) = self.chunk_region(n).iso_world_region else {
                    continue;
                };
                if nw == r {
                    continue;
                }
                self.wr(r).add_neighbor(nw);
                self.wr(nw).add_neighbor(r);
            }
        }
    }

    /// `IsoWorldRegion.addIsoChunkRegion(region)`.
    pub(crate) fn add_iso_chunk_region(&mut self, w: WorldRegionRef, c: ChunkRegionRef) {
        if self.world_region(w).iso_chunk_regions.contains(&c) {
            return;
        }
        let (sz, rc) = {
            let cr = self.chunk_region(c);
            (cr.square_size as i32, cr.roof_cnt as i32)
        };
        let wr = self.wr(w);
        wr.square_size += sz;
        wr.roof_cnt += rc;
        wr.is_dirty_enclosed.set(true);
        wr.iso_chunk_regions.push(c);
        self.cr(c).iso_world_region = Some(w);
    }

    /// `IsoWorldRegion.removeIsoChunkRegion(region)`.
    pub(crate) fn remove_iso_chunk_region(&mut self, w: WorldRegionRef, c: ChunkRegionRef) {
        if remove_first(&mut self.wr(w).iso_chunk_regions, &c) {
            let (sz, rc) = {
                let cr = self.chunk_region(c);
                (cr.square_size as i32, cr.roof_cnt as i32)
            };
            let wr = self.wr(w);
            wr.square_size -= sz;
            wr.roof_cnt -= rc;
            wr.is_dirty_enclosed.set(true);
            self.cr(c).iso_world_region = None;
        }
    }

    /// `IsoWorldRegion.swapIsoChunkRegions`: takes the region's list,
    /// leaving it empty (the original swaps in a cleared list).
    pub(crate) fn take_iso_chunk_regions(&mut self, w: WorldRegionRef) -> Vec<ChunkRegionRef> {
        std::mem::take(&mut self.wr(w).iso_chunk_regions)
    }

    /// `IsoWorldRegion.merge(other)` into `w`.
    pub(crate) fn merge(&mut self, w: WorldRegionRef, other: WorldRegionRef) {
        if !self.world_region(other).iso_chunk_regions.is_empty() {
            let mut i = self.world_region(other).iso_chunk_regions.len();
            while i > 0 {
                i -= 1;
                let c = self.world_region(other).iso_chunk_regions[i];
                self.remove_iso_chunk_region(other, c);
                self.add_iso_chunk_region(w, c);
            }
            self.world_region(w).is_dirty_enclosed.set(true);
            self.wr(other).iso_chunk_regions.clear();
        }
        if !self.world_region(other).neighbors.is_empty() {
            // Index loop over the live list, as upstream (a region can be
            // its own neighbour after a merge).
            let mut i = 0;
            while i < self.world_region(other).neighbors.len() {
                let n = self.world_region(other).neighbors[i];
                remove_first(&mut self.wr(n).neighbors, &other);
                self.wr(w).add_neighbor(n);
                i += 1;
            }
            self.wr(other).neighbors.clear();
        }
        self.release_iso_world_region(other);
    }
}
