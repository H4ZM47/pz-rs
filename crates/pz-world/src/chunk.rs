//! `IsoCell`, `IsoChunk`, `IsoGridSquare` and lot placement
//! (`IsoCell.PlaceLot`, `CellLoader.LoadCellBinaryChunk`).
//!
//! Objects are created by [`crate::cell_loader::do_tile_object_creation`].
//! Object changes after loading (`AddTileObject`, `RemoveTileObject`,
//! `addFloor`, ...) are [`crate::square_mutation`].

use crate::{RetiredWorldOwners, SquareRef, SquareSlot};
use std::collections::HashMap;

use pz_core::debug::{self, JulLevel, StackElement};

use crate::cell_loader::{do_tile_object_creation, ObjectEnv};
use crate::defs::{BuildingIdx, RoomDef};
use crate::erosion_data::{ErosionChunkData, ErosionSquareData};
use crate::fix2x::Fix2xTable;
use crate::iso_lot::IsoLot;
use crate::java::{fastfloor, java_throwable, GameRand};
use crate::map_files::{header_name, lotpack_name};
use crate::meta_grid::IsoMetaGrid;
use crate::objects::{ObjectId, ObjectLoadHooks, ObjectLoadSettings, ObjectTables};
use crate::save::lua_table::LuaTable;
use crate::save::visual::IsoDeadBody;
use crate::sprite::IsoSpriteManager;
use crate::sprite::{PropertyContainer, SurfaceInfo};
use crate::square::IsoDirections;

mod brand_new;
pub use brand_new::{
    get_corner_of_generation, load_brand_new, load_brand_new_cached, BrandNewHook, ChunksCache,
};

pub const CHUNK_WIDTH: i32 = 8;
pub const MIN_LEVEL: i32 = -32;
pub const MAX_LEVEL: i32 = 31;

/// `IsoGridSquare` (map-loading fields).
#[derive(Debug)]
pub struct IsoGridSquare {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub room_id: i64,
    /// `roofHideBuilding`: the building of the `emptyoutside` room here.
    pub roof_hide_building: Option<BuildingIdx>,
    /// `associatedBuilding` (`IsoMetaGrid.getAssociatedBuildingAt`, set
    /// when the square is put in its chunk).
    pub associated_building: Option<BuildingIdx>,
    /// `hasSetIsoWorldRegion`: whether `isoWorldRegion` holds the lookup.
    pub has_set_iso_world_region: std::cell::Cell<bool>,
    /// `isoWorldRegion`: the cached `IsoRegions.getIsoWorldRegion` result
    /// (see [`IsoGridSquare::get_iso_world_region`]).
    pub iso_world_region: std::cell::Cell<Option<crate::isoregion::WorldRegionRef>>,
    pub objects: Vec<crate::ObjectSlot>,
    retired_objects: crate::object_slot::ObjectRetirements,
    /// `specialObjects`, by object identity. Like the original, clearing
    /// `objects` leaves this list alone; identities no longer in `objects`
    /// are skipped by lookups.
    pub special_objects: Vec<ObjectId>,
    pub(crate) next_object_id: ObjectId,
    /// `erosion` (`ErosionData.Square`, created on first use), saved first
    /// in the square record; `None` saves as uninitialised.
    pub erosion: Option<ErosionSquareData>,
    /// The independent stored `worldObjects` ArrayList. Raw membership need
    /// not agree with the main objects list or item/object backlinks.
    pub world_objects: crate::WorldObjectsRef,
    /// `staticMovingObjects`: corpses.
    pub static_moving: Vec<IsoDeadBody>,
    /// `table` (square mod data).
    pub mod_data: Option<LuaTable>,
    pub overlay_done: bool,
    pub have_sheet_rope: bool,
    /// `isSeen(playerIndex)` for the four local players.
    pub seen: [bool; 4],
    /// `getTrapPositionX/Y/Z()` when X is positive. Loading reads and drops
    /// the saved position.
    pub trap_position: Option<(i32, i32, i32)>,
    /// `properties`: the merged sprite properties and derived flags
    /// (`RecalcProperties`, see [`crate::square`]).
    pub properties: PropertyContainer,
    /// `hasTypes`: bit `IsoObjectType.index()` per object type present
    /// (`1L << index`, so indices wrap at 64 like Java's shift).
    pub has_types: u64,
    /// `hasTree`.
    pub has_tree: bool,
    /// `haveRoof`: a rain-blocking square is somewhere above.
    pub have_roof: bool,
    /// `burntOut`.
    pub burnt_out: bool,
    /// `solidFloorCached` / `solidFloor`: `TreatAsSolidFloor()`'s cached
    /// answer. Only `RecalcAllWithNeighbours` and `ReCalculateAll` drop it,
    /// so it can be stale after property changes made without them
    /// (`RemoveTileObjectErosionNoRecalc`), as upstream.
    pub solid_floor_cache: std::cell::Cell<Option<bool>>,
    /// `properties.initSurface()` values of the merged properties.
    pub surface: SurfaceInfo,
    /// `properties.getSlopedSurfaceDirection()`.
    pub sloped_surface_direction: Option<IsoDirections>,
    /// `collideMatrix`, `pathMatrix`, `visionMatrix`: bit
    /// `(dx+1) + 3 (dy+1) + 9 (dz+1)` set when blocked.
    pub collide_matrix: i32,
    pub path_matrix: i32,
    pub vision_matrix: i32,
}

impl IsoGridSquare {
    /// An explicit internal value copy, distinct from retaining this square.
    /// Repeated main-list references share one copied object. Leaf item/bag
    /// copies retain their existing per-object value-tree policy; cross-object
    /// child sharing is not preserved. Borrowed, cyclic, deep or large item
    /// graphs are rejected without changing the source. This operation runs
    /// synchronously without callbacks; it is not a native save/reload clone.
    pub fn try_archive_copy(&self) -> Result<Self, crate::SquareArchiveError> {
        self.try_archive_copy_with_policy(crate::archive::ForeignArchivePolicy::RejectForeign)
    }

    /// Explicitly retaining foreign handles can share opaque VM state with the
    /// source. It does not rewrite references hidden inside foreign values.
    pub fn try_archive_copy_with_policy(
        &self,
        policy: crate::archive::ForeignArchivePolicy,
    ) -> Result<Self, crate::SquareArchiveError> {
        let copied = crate::archive::copy_objects(self, policy)?;
        Ok(Self {
            x: self.x,
            y: self.y,
            z: self.z,
            room_id: self.room_id,
            roof_hide_building: self.roof_hide_building,
            associated_building: self.associated_building,
            has_set_iso_world_region: self.has_set_iso_world_region.clone(),
            iso_world_region: self.iso_world_region.clone(),
            objects: copied.objects,
            retired_objects: Default::default(),
            special_objects: self.special_objects.clone(),
            next_object_id: self.next_object_id,
            erosion: self.erosion.clone(),
            world_objects: copied.world_objects,
            static_moving: self.static_moving.clone(),
            mod_data: self.mod_data.clone(),
            overlay_done: self.overlay_done,
            have_sheet_rope: self.have_sheet_rope,
            seen: self.seen,
            trap_position: self.trap_position,
            properties: self.properties.clone(),
            has_types: self.has_types,
            has_tree: self.has_tree,
            have_roof: self.have_roof,
            burnt_out: self.burnt_out,
            solid_floor_cache: self.solid_floor_cache.clone(),
            surface: self.surface.clone(),
            sloped_surface_direction: self.sloped_surface_direction,
            collide_matrix: self.collide_matrix,
            path_matrix: self.path_matrix,
            vision_matrix: self.vision_matrix,
        })
    }

    pub fn new(x: i32, y: i32, z: i32) -> Self {
        IsoGridSquare {
            x,
            y,
            z,
            room_id: -1,
            roof_hide_building: None,
            associated_building: None,
            has_set_iso_world_region: std::cell::Cell::new(false),
            iso_world_region: std::cell::Cell::new(None),
            objects: Vec::new(),
            retired_objects: Default::default(),
            special_objects: Vec::new(),
            next_object_id: 0,
            erosion: None,
            world_objects: crate::WorldObjects::new_ref(),
            static_moving: Vec::new(),
            mod_data: None,
            overlay_done: false,
            have_sheet_rope: false,
            seen: [false; 4],
            trap_position: None,
            properties: PropertyContainer::default(),
            has_types: 0,
            has_tree: false,
            have_roof: false,
            burnt_out: false,
            solid_floor_cache: std::cell::Cell::new(None),
            surface: SurfaceInfo::default(),
            sloped_surface_direction: None,
            collide_matrix: 0x7FF_FFFF,
            path_matrix: 0x7FF_FFFF,
            vision_matrix: 0,
        }
    }

    /// `ResetIsoWorldRegion()`: drop the cached world region, so the next
    /// [`IsoGridSquare::get_iso_world_region`] looks it up again.
    pub fn reset_iso_world_region(&self) {
        self.iso_world_region.set(None);
        self.has_set_iso_world_region.set(false);
    }

    /// `setIsoWorldRegion(mr)`.
    pub fn set_iso_world_region(&self, mr: Option<crate::isoregion::WorldRegionRef>) {
        self.has_set_iso_world_region.set(mr.is_some());
        self.iso_world_region.set(mr);
    }

    /// `getIsoWorldRegion()` (single player: the lookup is cached until
    /// reset; levels below 0 have none).
    pub fn get_iso_world_region(
        &self,
        regions: &crate::isoregion::IsoRegions,
    ) -> Option<crate::isoregion::WorldRegionRef> {
        if self.z < 0 {
            return None;
        }
        if !self.has_set_iso_world_region.get() {
            self.iso_world_region
                .set(regions.get_iso_world_region(self.x, self.y, self.z));
            self.has_set_iso_world_region.set(true);
        }
        self.iso_world_region.get()
    }

    /// Keep a detached receiver alive until its host releases world/sprite guards.
    pub fn retire_object(&mut self, object: crate::ObjectSlot) {
        self.retired_objects.0.push(object);
    }
    /// Remove the actual slot, pinning an alias for the host retirement boundary.
    pub fn remove_object_slot(&mut self, index: usize) -> crate::ObjectSlot {
        let object = self.objects.remove(index);
        self.retire_object(crate::ObjectSlot::retain(&object.ref_handle()));
        object
    }
    pub fn replace_object_slot(&mut self, index: usize, object: crate::ObjectSlot) {
        let old = std::mem::replace(&mut self.objects[index], object);
        self.retire_object(old);
    }
    pub fn clear_objects(&mut self) {
        self.retired_objects.0.append(&mut self.objects);
    }
    pub fn retain_objects(&mut self, mut keep: impl FnMut(&crate::ObjectSlot) -> bool) {
        let mut i = 0;
        while i < self.objects.len() {
            if keep(&self.objects[i]) {
                i += 1;
            } else {
                let old = self.objects.remove(i);
                self.retire_object(old);
            }
        }
    }
    pub fn take_retired_objects(&mut self) -> Vec<crate::ObjectSlot> {
        std::mem::take(&mut self.retired_objects.0)
    }

    /// `isInARoom()`: a meta-grid room, or a player room region.
    pub fn is_in_a_room(&self, meta: &IsoMetaGrid, regions: &crate::isoregion::IsoRegions) -> bool {
        self.room_def(meta).is_some()
            || self
                .get_iso_world_region(regions)
                .is_some_and(|r| regions.is_player_room(r))
    }

    /// `getRoomSize()` for a player room region (`None` for meta-grid rooms,
    /// whose squares come from `IsoRoom`, and where the original returns
    /// -1).
    pub fn player_room_size(&self, regions: &crate::isoregion::IsoRegions) -> Option<i32> {
        let r = self.get_iso_world_region(regions)?;
        regions
            .is_player_room(r)
            .then(|| regions.world_region(r).square_size())
    }

    /// `getRoom().def`: the room this square's `roomID` names.
    pub fn room_def<'m>(&self, meta: &'m IsoMetaGrid) -> Option<&'m RoomDef> {
        if self.room_id == -1 {
            return None;
        }
        meta.get_room_def_by_id(self.room_id)
    }
}

/// `zombie.iso.enums.ChunkGenerationStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChunkGenerationStatus {
    Core,
    Modded,
    Worldgen,
}

/// `EnumSet<ChunkGenerationStatus>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChunkGenerationStatusSet(u8);

impl ChunkGenerationStatusSet {
    fn bit(s: ChunkGenerationStatus) -> u8 {
        1 << s as u8
    }
    pub fn contains(&self, s: ChunkGenerationStatus) -> bool {
        self.0 & Self::bit(s) != 0
    }
    pub fn add(&mut self, s: ChunkGenerationStatus) {
        self.0 |= Self::bit(s);
    }
    pub fn remove(&mut self, s: ChunkGenerationStatus) {
        self.0 &= !Self::bit(s);
    }
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
    /// Members in ordinal order, as `EnumSet` iterates.
    pub fn iter(&self) -> impl Iterator<Item = ChunkGenerationStatus> + '_ {
        [
            ChunkGenerationStatus::Core,
            ChunkGenerationStatus::Modded,
            ChunkGenerationStatus::Worldgen,
        ]
        .into_iter()
        .filter(|s| self.contains(*s))
    }
}

/// `IsoChunk` (map-loading fields).
pub struct IsoChunk {
    pub wx: i32,
    pub wy: i32,
    pub min_level: i32,
    pub max_level: i32,
    /// `squares[z - minLevel][x + y*8]`.
    squares: Vec<Vec<Option<SquareSlot>>>,
    retired_objects: Vec<crate::ObjectSlot>,
    retired_squares: Vec<SquareSlot>,
    /// `lotheader`: header file name, set when the lot came from the
    /// top-level header.
    pub lotheader: Option<String>,
    /// True when any lot placed into the chunk came from a modded map
    /// folder (kept alongside `generation_status` for existing callers).
    pub modded: bool,
    /// `chunkGenerationStatus` (`isModded()` / `addModded` / `rmModded`):
    /// `CORE` or `MODDED` from each placed lot's map folder, `WORLDGEN`
    /// when world generation ran on the chunk.
    pub generation_status: ChunkGenerationStatusSet,
    /// The chunk-level fields of the chunk file; see
    /// [`crate::save::chunk_file`].
    pub saved: ChunkSaveData,
    /// `erosion` (`ErosionData.Chunk`, created on first use); `None` saves
    /// as uninitialised.
    pub erosion: Option<ErosionChunkData>,
    /// `addZombies`: set by `LoadBrandNew`; `doLoadGridsquare` then adds
    /// vehicles, zone stories and zombies.
    pub add_zombies: bool,
    /// `vehicles`: filled from the chunk file's vehicle list (saving writes
    /// an empty list; the original keeps vehicles in `vehicles.db`) and by
    /// [`IsoChunk::vehicle_loaded`].
    pub vehicles: crate::chunk_vehicles::ChunkVehicles,
    /// `IsoChunkLevel.physicsCheck`, per level (`[z - minLevel]`).
    physics_check: Vec<bool>,
    /// `collision` (`CollideWithObstaclesPoly.ChunkData`): the collision
    /// geometry the pathfinding side builds per level. Every
    /// `RecalcProperties` of one of the chunk's squares clears it.
    pub collision: ChunkCollision,
}

/// `CollideWithObstaclesPoly.ChunkData`: a lazily built entry per level
/// 0..8 (`ChunkDataZ`: obstacles and nodes from the squares, built by the
/// collision code, which is not part of this crate). `clear()` only flags
/// the data; the next [`ChunkCollision::init`] drops every level first.
#[derive(Default)]
pub struct ChunkCollision {
    data: [Option<Box<dyn std::any::Any + Send>>; 8],
    clear: bool,
}

impl std::fmt::Debug for ChunkCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let levels: Vec<usize> = (0..8).filter(|&z| self.data[z].is_some()).collect();
        f.debug_struct("ChunkCollision")
            .field("levels", &levels)
            .field("clear", &self.clear)
            .finish()
    }
}

impl ChunkCollision {
    /// `clear()`: the data is dropped at the next `init`.
    pub fn clear(&mut self) {
        self.clear = true;
    }

    /// Whether a `clear()` is waiting for the next `init`.
    pub fn is_clear_pending(&self) -> bool {
        self.clear
    }

    /// `init(chunk, z, instance)`: level `z`'s data, built with `build`
    /// when missing (`ChunkDataZ.pool.alloc().init(...)`); a pending clear
    /// drops every level first (`clearInner`). `None` for a level outside
    /// 0..8 (an index error upstream). Data of another type than `T` is
    /// replaced.
    pub fn init<T: std::any::Any + Send>(
        &mut self,
        z: i32,
        build: impl FnOnce() -> T,
    ) -> Option<&mut T> {
        if self.clear {
            self.clear = false;
            self.data = Default::default();
        }
        let slot = self.data.get_mut(usize::try_from(z).ok()?)?;
        if !slot.as_ref().is_some_and(|d| d.is::<T>()) {
            *slot = Some(Box::new(build()));
        }
        slot.as_mut()?.downcast_mut()
    }

    /// `data[z]` as it is (no pending clear applied).
    pub fn get<T: std::any::Any>(&self, z: i32) -> Option<&T> {
        self.data
            .get(usize::try_from(z).ok()?)?
            .as_ref()?
            .downcast_ref()
    }

    /// `data[z] = null` with the old entry handed back
    /// (`CollideWithObstaclesPoly.vehicleMoved`).
    pub fn take(&mut self, z: i32) -> Option<Box<dyn std::any::Any + Send>> {
        self.data.get_mut(usize::try_from(z).ok()?)?.take()
    }
}

/// `IsoFloorBloodSplat` (positions quantised to 1/255 of a chunk).
#[derive(Clone, Debug, PartialEq)]
pub struct FloorBloodSplat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub kind: i8,
    pub world_age: f32,
    pub index: i8,
}

/// Fields of `IsoChunk` that chunk files carry besides the squares.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkSaveData {
    pub blending_done_full: bool,
    pub blending_modified: [bool; 4],
    pub blending_done_partial: bool,
    /// `blendingDepth` (N, S, W, E). A new chunk starts from
    /// `BlendDirection.defaultDepth`, which is only saved once blending is
    /// partially done; this defaults to zeros.
    pub blending_depth: [i8; 4],
    pub attachments_done_full: bool,
    pub attachments_state: [bool; 5],
    pub attachments_partial: Option<Vec<(i32, i32, i32)>>,
    /// `floorBloodSplats` (a queue bounded to 1000).
    pub floor_blood_splats: Vec<FloorBloodSplat>,
    pub next_splat_index: i32,
    /// `generatorsTouchingThisChunk`.
    pub generators: Option<Vec<(i32, i32, i8)>>,
    pub loot_respawn_hour: i32,
    pub spawned_rooms: Vec<i64>,
}

impl Default for ChunkSaveData {
    /// `new IsoChunk(cell)`.
    fn default() -> Self {
        ChunkSaveData {
            blending_done_full: false,
            blending_modified: [false; 4],
            blending_done_partial: false,
            blending_depth: [0; 4],
            attachments_done_full: true,
            attachments_state: [true; 5],
            attachments_partial: None,
            floor_blood_splats: Vec::new(),
            next_splat_index: 0,
            generators: None,
            loot_respawn_hour: -1,
            spawned_rooms: Vec::new(),
        }
    }
}

impl IsoChunk {
    /// `addModded(status)`.
    pub fn add_modded(&mut self, status: ChunkGenerationStatus) {
        self.generation_status.add(status);
        if status == ChunkGenerationStatus::Modded {
            self.modded = true;
        }
    }

    /// `rmModded(status)`.
    pub fn rm_modded(&mut self, status: ChunkGenerationStatus) {
        self.generation_status.remove(status);
        if status == ChunkGenerationStatus::Modded {
            self.modded = false;
        }
    }

    pub fn new(wx: i32, wy: i32) -> Self {
        IsoChunk {
            wx,
            wy,
            min_level: 0,
            max_level: 0,
            squares: vec![(0..64).map(|_| None).collect()],
            retired_objects: Vec::new(),
            retired_squares: Vec::new(),
            lotheader: None,
            modded: false,
            generation_status: ChunkGenerationStatusSet::default(),
            saved: ChunkSaveData::default(),
            erosion: None,
            add_zombies: false,
            vehicles: Default::default(),
            physics_check: vec![false],
            collision: ChunkCollision::default(),
        }
    }

    /// `checkPhysicsLater(level)`: marks the level's physics shapes for
    /// rebuilding (`IsoChunkLevel.physicsCheck = true`); nothing for a level
    /// the chunk does not have. `RecalcProperties` on one of the chunk's
    /// squares calls it (see [`IsoCell::recalc_chunk_squares`] and
    /// [`IsoCell::recalc_all_with_neighbours`]).
    pub fn check_physics_later(&mut self, level: i32) {
        if self.is_valid_level(level) {
            self.physics_check[(level - self.min_level) as usize] = true;
        }
    }

    /// `getLevelData(level).physicsCheck` (`false` for a level the chunk
    /// does not have).
    pub fn physics_check(&self, level: i32) -> bool {
        self.is_valid_level(level) && self.physics_check[(level - self.min_level) as usize]
    }

    /// The test-and-clear of `Bullet.updatePhysicsForLevelIfNeeded`: returns
    /// whether the level's flag was set and clears it (the caller then runs
    /// `updatePhysicsForLevel(level)`).
    pub fn take_physics_check(&mut self, level: i32) -> bool {
        if !self.is_valid_level(level) {
            return false;
        }
        std::mem::take(&mut self.physics_check[(level - self.min_level) as usize])
    }

    /// `VehiclesDB2.WorldStreamerThread.vehicleLoaded`: a vehicle of a
    /// `vehicles.db` row of this chunk, given the square at its position
    /// (`(int)(x - wx*8)`, `(int)(y - wy*8)`, level 0), joins the list.
    /// Returns its handle.
    pub fn vehicle_loaded(
        &mut self,
        row: &crate::save::vehicle::VehicleRow,
        vehicle: crate::save::vehicle::BaseVehicle,
    ) -> crate::chunk_vehicles::VehicleHandle {
        let lx = (row.x - (self.wx * CHUNK_WIDTH) as f32) as i32;
        let ly = (row.y - (self.wy * CHUNK_WIDTH) as f32) as i32;
        let square = self
            .get_grid_square(lx, ly, 0)
            .map(|s| (s.borrow().x, s.borrow().y, s.borrow().z));
        let h: crate::chunk_vehicles::VehicleHandle =
            std::rc::Rc::new(crate::chunk_vehicles::LoadedVehicle {
                origin: crate::chunk_vehicles::VehicleOrigin::Database { sql_id: row.id },
                square,
                vehicle,
            });
        self.vehicles.add(h.clone());
        h
    }

    pub fn is_valid_level(&self, z: i32) -> bool {
        z >= self.min_level && z <= self.max_level
    }

    pub fn set_min_max_level(&mut self, min_level: i32, max_level: i32) {
        if min_level == self.min_level && max_level == self.max_level {
            return;
        }
        // Kept levels keep their `IsoChunkLevel` (and flags); dropped levels
        // leave live storage as whole retained allocations. No native pool reset
        // or discard is implied by retiring an owner here.
        let mut levels = Vec::with_capacity((max_level - min_level + 1) as usize);
        let mut physics = Vec::with_capacity(levels.capacity());
        for z in min_level..=max_level {
            if self.is_valid_level(z) {
                let i = (z - self.min_level) as usize;
                levels.push(std::mem::take(&mut self.squares[i]));
                physics.push(self.physics_check[i]);
            } else {
                levels.push((0..64).map(|_| None).collect());
                physics.push(false);
            }
        }
        for level in &mut self.squares {
            for old in level.iter_mut().filter_map(Option::take) {
                self.retired_squares.push(old);
            }
        }
        self.min_level = min_level;
        self.max_level = max_level;
        self.squares = levels;
        self.physics_check = physics;
    }

    pub fn set_square(&mut self, x: i32, y: i32, z: i32, square: Option<SquareSlot>) {
        // Pin the incoming allocation before any indexing work can unwind.
        if let Some(sq) = &square {
            self.retired_squares
                .push(SquareSlot::retain(&sq.ref_handle()));
        }
        self.set_min_max_level(self.min_level.min(z), self.max_level.max(z));
        let zz = (z - self.min_level) as usize;
        if let Some(old) = std::mem::replace(&mut self.squares[zz][(x + y * 8) as usize], square) {
            self.retired_squares.push(old);
        }
    }

    pub fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<SquareRef> {
        if !(0..8).contains(&x) || !(0..8).contains(&y) || z > self.max_level || z < self.min_level
        {
            return None;
        }
        self.squares[(z - self.min_level) as usize][(y * 8 + x) as usize]
            .as_ref()
            .map(SquareSlot::ref_handle)
    }
    pub fn get_grid_square_mut(&mut self, x: i32, y: i32, z: i32) -> Option<SquareRef> {
        self.get_grid_square(x, y, z)
    }
    /// Move whole square slots from a terminally retired chunk to its host's
    /// retry queue. Only call after its last possible save. The square bodies
    /// are never borrowed, cleared or reset by this ownership transfer.
    pub fn take_square_owners_for_retirement(&mut self) -> Vec<SquareSlot> {
        let mut squares = std::mem::take(&mut self.retired_squares);
        for slot in self.squares.iter_mut().flatten() {
            if let Some(square) = slot.take() {
                squares.push(square);
            }
        }
        squares
    }
    pub fn retire_object(&mut self, object: crate::ObjectSlot) {
        self.retired_objects.push(object);
    }
    pub fn take_retired_owners(&mut self) -> RetiredWorldOwners {
        let mut retired = RetiredWorldOwners {
            objects: std::mem::take(&mut self.retired_objects),
            squares: Vec::new(),
            components: Default::default(),
        };
        for slot in self.squares.iter().flatten().flatten() {
            if let Ok(mut square) = slot.square_borrow_for_retirement() {
                retired.objects.extend(square.take_retired_objects());
            }
        }
        crate::square_slot::drain_squares(&mut self.retired_squares, &mut retired);
        retired
    }
    pub fn take_retired_objects(&mut self) -> Vec<crate::ObjectSlot> {
        let mut retired = std::mem::take(&mut self.retired_objects);
        for slot in self
            .squares
            .iter()
            .flatten()
            .flatten()
            .chain(self.retired_squares.iter())
        {
            if let Ok(mut square) = slot.square_borrow_for_retirement() {
                retired.extend(square.take_retired_objects());
            }
        }
        retired
    }
    pub fn squares(&self) -> impl Iterator<Item = SquareRef> + '_ {
        self.squares
            .iter()
            .flatten()
            .flatten()
            .map(SquareSlot::ref_handle)
    }
}

/// Runtime inputs chunk loading takes from other systems.
pub struct ChunkLoadEnv<'a> {
    /// `IsoSpriteManager.instance` after `loadedTileDefinitions`. Object
    /// creation changes some sprites (windows), as upstream.
    pub sprites: &'a mut IsoSpriteManager,
    /// `IsoChunk.Fix2xMap`, read from the installed game.
    pub fix2x: &'a Fix2xTable,
    /// `Rand`.
    pub rand: &'a mut dyn GameRand,
    /// Game-code tables, read from the installed game.
    pub tables: &'a ObjectTables,
    /// Installed `BloodBodyPartType.MAX.index()` for visual constructors.
    pub blood_body_parts: usize,
    pub settings: ObjectLoadSettings,
    pub hooks: &'a mut dyn ObjectLoadHooks,
}

impl ChunkLoadEnv<'_> {
    fn objects(&mut self) -> ObjectEnv<'_> {
        ObjectEnv {
            sprites: self.sprites,
            rand: self.rand,
            tables: self.tables,
            blood_body_parts: self.blood_body_parts,
            settings: self.settings,
            hooks: self.hooks,
        }
    }
}

/// The exception `PlaceLot` caught.
#[derive(Debug)]
struct PlaceError(String);

fn npe_place() -> PlaceError {
    PlaceError("NullPointerException".into())
}

impl From<String> for PlaceError {
    fn from(s: String) -> Self {
        PlaceError(s)
    }
}

fn place_error() -> PlaceError {
    PlaceError("ArrayIndexOutOfBoundsException".into())
}

/// `IsoGridSquare.getNew` + `IsoChunk.setSquare`, which sets
/// `associatedBuilding`.
fn new_square(meta: &IsoMetaGrid, x: i32, y: i32, z: i32) -> Result<IsoGridSquare, PlaceError> {
    let mut sq = IsoGridSquare::new(x, y, z);
    sq.associated_building = meta.get_associated_building_at(x, y)?;
    Ok(sq)
}

/// `IsoCell.PlaceLot(lot, sx, sy, sz, ch, wx, wy, bDoneSquares)`.
#[allow(clippy::too_many_arguments)]
pub fn place_lot(
    meta: &IsoMetaGrid,
    lot: &IsoLot,
    sx: i32,
    sy: i32,
    sz: i32,
    ch: &mut IsoChunk,
    wx: i32,
    wy: i32,
    done: &mut [bool; 64],
    env: &mut ChunkLoadEnv<'_>,
) -> i32 {
    let wx = wx * 8;
    let wy = wy * 8;
    let min_level = sz.max(MIN_LEVEL);
    let max_level = (sz + lot.max_level - lot.min_level - 1).min(MAX_LEVEL);
    let mut count = 0;
    let result = place_lot_inner(
        meta, lot, sx, sy, ch, wx, wy, min_level, max_level, done, &mut count, env,
    );
    if let Err(PlaceError(e)) = result {
        // `DebugLog.log(...)` then `ExceptionLogger.logException(ex)`.
        debug::log("Failed to load chunk, blocking out area");
        debug::log_exception(
            &java_throwable(&e),
            Some(&StackElement::new("zombie.iso.IsoCell", "PlaceLot")),
        );
        for x in wx + sx..wx + sx + 8 {
            for y in wy + sy..wy + sy + 8 {
                for z in min_level..=max_level {
                    ch.set_square(x - wx - sx, y - wy - sy, z - sz, None);
                }
            }
        }
    }
    count
}

#[allow(clippy::too_many_arguments)]
fn place_lot_inner(
    meta: &IsoMetaGrid,
    lot: &IsoLot,
    sx: i32,
    sy: i32,
    ch: &mut IsoChunk,
    wx: i32,
    wy: i32,
    min_level: i32,
    max_level: i32,
    done: &mut [bool; 64],
    count: &mut i32,
    env: &mut ChunkLoadEnv<'_>,
) -> Result<(), PlaceError> {
    let info = lot.info(&meta.map_files).ok_or_else(npe_place)?;
    for x in wx + sx..wx + sx + 8 {
        for y in wy + sy..wy + sy + 8 {
            if x < wx || y < wy || x >= wx + 8 || y >= wy + 8 {
                continue;
            }
            let di = (x - wx + (y - wy) * 8) as usize;
            if done[di] {
                *count += 1;
                continue;
            }
            for z in min_level..=max_level {
                let lx = x - wx - sx;
                let ly = y - wy - sy;
                let lz = z - info.min_level;
                let square_xyz = lx + ly * 8 + lz * 64;
                if square_xyz < 0 || square_xyz as usize >= lot.offset_in_data.len() {
                    return Err(place_error());
                }
                let off = lot.offset_in_data[square_xyz as usize];
                if off == -1 {
                    continue;
                }
                let num_ints = *lot.data.get(off as usize).ok_or_else(place_error)?;
                if num_ints <= 0 {
                    continue;
                }
                if !done[di] {
                    done[di] = true;
                    *count += 1;
                }
                if ch.get_grid_square(x - wx, y - wy, z).is_none() {
                    ch.set_square(x - wx, y - wy, z, Some((new_square(meta, x, y, z)?).into()));
                }
                for xx in -1..=1 {
                    for yy in -1..=1 {
                        if xx == 0 && yy == 0
                            || xx + x - wx < 0
                            || xx + x - wx >= 8
                            || yy + y - wy < 0
                            || yy + y - wy >= 8
                        {
                            continue;
                        }
                        if ch.get_grid_square(x + xx - wx, y + yy - wy, z).is_some() {
                            continue;
                        }
                        ch.set_square(
                            x + xx - wx,
                            y + yy - wy,
                            z,
                            Some((new_square(meta, x + xx, y + yy, z)?).into()),
                        );
                    }
                }
                let room_id = meta.get_room_at(x, y, z).map_or(-1, |r| r.id);
                let square_owner = ch.get_grid_square_mut(x - wx, y - wy, z).unwrap();
                let mut square = square_owner.borrow_mut();
                // `setRoomID` looks the room up (`IsoChunk.getRoom`).
                square.room_id = room_id;
                if room_id != -1 {
                    meta.get_room_by_id(room_id, env.rand);
                }
                square.reset_iso_world_region();
                if let Some(eo) = meta.get_empty_outside_at(x, y, z) {
                    square.roof_hide_building = meta
                        .get_room_by_id(eo.id, env.rand)
                        .and_then(|r| r.building);
                }
                drop(square);
                for n in 0..num_ints {
                    let tile_idx = *lot
                        .data
                        .get(off as usize + 1 + n as usize)
                        .ok_or_else(place_error)?;
                    let tile = info
                        .tiles_used
                        .get(tile_idx as usize)
                        .ok_or_else(place_error)?;
                    let tile = if info.fixed2x {
                        tile.clone()
                    } else {
                        env.fix2x.fix2x(tile, env.rand)
                    };
                    let Some(&spr) = env.sprites.named_map.get(&tile) else {
                        // `Logger.getLogger(GameWindow.class.getName()).log(Level.SEVERE, ...)`.
                        debug::game_window_logger(
                            JulLevel::Severe,
                            Some(&StackElement::new("zombie.iso.IsoCell", "PlaceLot")),
                            Some(&format!("Missing tile definition: {tile}")),
                            None,
                        );
                        continue;
                    };
                    // `bClearObjects` is always true upstream (its solidfloor /
                    // hidewalls test only re-sets it), so the first tile with a
                    // definition always clears the square.
                    if n == 0 {
                        square_owner.borrow_mut().clear_objects();
                    }
                    let ty = env.sprites.sprites[spr].tile_type;
                    do_tile_object_creation(
                        &mut env.objects(),
                        meta,
                        &square_owner,
                        spr,
                        ty,
                        &tile,
                    )?;
                }
                square_owner.borrow_mut().fix_stackable_objects(env.sprites);
            }
        }
    }
    Ok(())
}

/// Lots opened for one chunk load, one per map directory, reused like the
/// original's `IsoLot` pool keeps its file open.
#[derive(Default)]
pub struct LotCache {
    lots: HashMap<usize, IsoLot>,
}

impl LotCache {
    fn get(
        &mut self,
        meta: &IsoMetaGrid,
        mf: usize,
        cell_x: i32,
        cell_y: i32,
        wx: i32,
        wy: i32,
    ) -> &IsoLot {
        let lot = self.lots.entry(mf).or_default();
        lot.load(&meta.map_files, mf, cell_x, cell_y, wx, wy);
        lot
    }
}

/// `CellLoader.LoadCellBinaryChunk(cell, wx, wy, chunk)`.
pub fn load_cell_binary_chunk(
    meta: &IsoMetaGrid,
    wx: i32,
    wy: i32,
    chunk: &mut IsoChunk,
    env: &mut ChunkLoadEnv<'_>,
    cache: &mut LotCache,
) -> bool {
    let cell_x = fastfloor(wx as f32 / 32.0);
    let cell_y = fastfloor(wy as f32 / 32.0);
    let pack = lotpack_name(cell_x, cell_y);
    let Some(path) = meta.info_file_names.get(&pack) else {
        return false;
    };
    if !path.exists() {
        return false;
    }
    let header = header_name(cell_x, cell_y);
    // NPE upstream when the lotpack has no header.
    let Some(&mf) = meta.info_headers.get(&header) else {
        return false;
    };
    let lot = cache.get(meta, mf, cell_x, cell_y, wx, wy);
    if lot.info.is_none() {
        return true;
    }
    if meta.info_headers.get(&header) == Some(&lot.map_files) {
        chunk.lotheader = Some(header.clone());
    }
    let mut done = [false; 64];
    let count = place_lot(
        meta,
        lot,
        0,
        0,
        lot.min_level,
        chunk,
        wx,
        wy,
        &mut done,
        env,
    );
    if count > 0 {
        let modded = meta.map_files[mf]
            .info_file_modded
            .get(&pack)
            .copied()
            .unwrap_or(false);
        chunk.add_modded(if modded {
            ChunkGenerationStatus::Modded
        } else {
            ChunkGenerationStatus::Core
        });
    }
    if count == 64 {
        return true;
    }
    let start = meta.map_files[mf].priority + 1;
    for i in start..meta.map_files.len() {
        if !meta.map_files[i].has_cell(cell_x, cell_y) {
            continue;
        }
        let lot2 = cache.get(meta, i, cell_x, cell_y, wx, wy);
        let count = place_lot(
            meta,
            lot2,
            0,
            0,
            lot2.min_level,
            chunk,
            wx,
            wy,
            &mut done,
            env,
        );
        if count > 0 {
            let modded = meta.map_files[i]
                .info_file_modded
                .get(&pack)
                .copied()
                .unwrap_or(false);
            chunk.add_modded(if modded {
                ChunkGenerationStatus::Modded
            } else {
                ChunkGenerationStatus::Core
            });
        }
        if count == 64 {
            break;
        }
    }
    true
}

/// `IsoCell`: loaded chunks keyed by chunk coordinates.
#[derive(Default)]
pub struct IsoCell {
    pub chunks: HashMap<(i32, i32), IsoChunk>,
    pub drag: crate::cell_drag::CellDrag,
    pub process_items: crate::process_items::ProcessItemQueues,
    cache: LotCache,
    retired_objects: Vec<crate::ObjectSlot>,
    retired_squares: Vec<SquareSlot>,
    retired_components: pz_entity::RetiredComponents,
}

impl IsoCell {
    /// Keep detached actual components and old owners alive until the same
    /// outer world/sprite boundary as retired whole objects and squares.
    pub fn retire_components(&mut self, retired: pz_entity::RetiredComponents) {
        self.retired_components.append(retired);
    }
    /// Retain a whole detached square until its pending host roots can be
    /// extracted and every outer world/sprite guard has been released.
    pub fn retire_square(&mut self, square: SquareSlot) {
        self.retired_squares.push(square);
    }

    pub fn retire_object(&mut self, object: crate::ObjectSlot) {
        self.retired_objects.push(object);
    }
    /// Move detached owners to a host boundary outside every world/sprite guard.
    pub fn take_retired_owners(&mut self) -> RetiredWorldOwners {
        let mut retired = RetiredWorldOwners {
            objects: std::mem::take(&mut self.retired_objects),
            squares: Vec::new(),
            components: std::mem::take(&mut self.retired_components),
        };
        crate::square_slot::drain_squares(&mut self.retired_squares, &mut retired);
        for chunk in self.chunks.values_mut() {
            retired.append(&mut chunk.take_retired_owners());
        }
        retired
    }
    pub fn take_retired_objects(&mut self) -> Vec<crate::ObjectSlot> {
        let mut retired = std::mem::take(&mut self.retired_objects);
        for slot in &self.retired_squares {
            if let Ok(mut square) = slot.square_borrow_for_retirement() {
                retired.extend(square.take_retired_objects());
            }
        }
        for chunk in self.chunks.values_mut() {
            retired.extend(chunk.take_retired_objects());
        }
        retired
    }

    /// The `vehicles` of loaded chunk `(wx, wy)`.
    pub fn chunk_vehicles(
        &self,
        wx: i32,
        wy: i32,
    ) -> Option<&crate::chunk_vehicles::ChunkVehicles> {
        self.chunks.get(&(wx, wy)).map(|c| &c.vehicles)
    }

    /// `BaseVehicle.update`'s chunk change: when the vehicle's current
    /// square is in loaded chunk `to` and that is not its chunk `from`,
    /// it leaves `from.vehicles` and joins `to.vehicles`. Returns whether
    /// it moved (the caller then sets `chunk` and calls
    /// `addFromCheckedVehicles`).
    pub fn update_vehicle_chunk(
        &mut self,
        v: &crate::chunk_vehicles::VehicleHandle,
        from: (i32, i32),
        to: (i32, i32),
    ) -> bool {
        if from == to || !self.chunks.contains_key(&to) {
            return false;
        }
        if let Some(c) = self.chunks.get_mut(&from) {
            c.vehicles.remove(v);
        }
        if let Some(c) = self.chunks.get_mut(&to) {
            c.vehicles.add(v.clone());
        }
        true
    }

    /// `this.chunk.vehicles.remove(this)` (`BaseVehicle.permanentlyRemove`
    /// and the meta hand-off of `IsoChunk.removeFromWorld`).
    pub fn remove_vehicle_from_chunk(
        &mut self,
        v: &crate::chunk_vehicles::VehicleHandle,
        chunk: (i32, i32),
    ) -> bool {
        self.chunks
            .get_mut(&chunk)
            .is_some_and(|c| c.vehicles.remove(v))
    }

    /// Remove chunk `(wx, wy)` from the cell (`IsoChunk.removeFromWorld`
    /// for the parts ported so far) and tell `hook` it is gone.
    pub fn unload_chunk(
        &mut self,
        wx: i32,
        wy: i32,
        hook: Option<&mut dyn crate::save::chunk_file::ChunkLoadHook>,
    ) -> Option<IsoChunk> {
        let mut chunk = self.chunks.remove(&(wx, wy));
        if let Some(c) = &mut chunk {
            self.retired_squares
                .extend(c.squares().map(|sq| SquareSlot::retain(&sq)));
            let mut retired = c.take_retired_owners();
            self.retired_objects.append(&mut retired.objects);
            self.retired_squares.append(&mut retired.squares);
            self.retired_components.append(std::mem::take(&mut retired.components));
            if let Some(h) = hook {
                h.chunk_unloaded(wx, wy);
            }
            // `IsoChunk.removeFromWorld`: after the pathfinding removal.
            c.collision.clear();
        }
        chunk
    }

    /// `IsoGridSquare.RecalcProperties()` of the square at `(x, y, z)` with
    /// its chunk's part: `chunk.checkPhysicsLater(z)`, then
    /// `chunk.collision.clear()`.
    pub fn recalc_square_properties(&mut self, x: i32, y: i32, z: i32, m: &IsoSpriteManager) {
        let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
        let Some(ch) = self.chunks.get_mut(&(wx, wy)) else {
            return;
        };
        if let Some(sq_owner) = ch.get_grid_square_mut(x - wx * 8, y - wy * 8, z) {
            let mut sq = sq_owner.borrow_mut();
            sq.recalc_properties(m);
            ch.check_physics_later(z);
            ch.collision.clear();
        }
    }

    /// Load chunk `(wx, wy)` from map files if it is not loaded yet.
    pub fn load_chunk(
        &mut self,
        meta: &IsoMetaGrid,
        wx: i32,
        wy: i32,
        env: &mut ChunkLoadEnv<'_>,
    ) -> &IsoChunk {
        if !self.chunks.contains_key(&(wx, wy)) {
            let mut ch = IsoChunk::new(wx, wy);
            load_cell_binary_chunk(meta, wx, wy, &mut ch, env, &mut self.cache);
            self.chunks.insert((wx, wy), ch);
        }
        &self.chunks[&(wx, wy)]
    }

    /// `getGridSquare(x, y, z)` over loaded chunks.
    pub fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<SquareRef> {
        let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
        self.chunks
            .get(&(wx, wy))?
            .get_grid_square(x - wx * 8, y - wy * 8, z)
    }
}

#[cfg(test)]
mod generation_status_tests {
    use super::*;

    #[test]
    fn add_and_remove() {
        let mut ch = IsoChunk::new(0, 0);
        assert!(ch.generation_status.is_empty());
        ch.add_modded(ChunkGenerationStatus::Worldgen);
        ch.add_modded(ChunkGenerationStatus::Core);
        let v: Vec<_> = ch.generation_status.iter().collect();
        assert_eq!(
            v,
            vec![ChunkGenerationStatus::Core, ChunkGenerationStatus::Worldgen]
        );
        ch.add_modded(ChunkGenerationStatus::Modded);
        assert!(ch.modded);
        ch.rm_modded(ChunkGenerationStatus::Modded);
        assert!(!ch.modded && !ch.generation_status.contains(ChunkGenerationStatus::Modded));
    }
}
