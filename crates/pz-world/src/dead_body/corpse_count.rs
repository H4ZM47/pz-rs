//! `CorpseCount`: corpses per chunk level, outside and per building, for
//! the corpse sickness (`BodyDamage`) and the flies (`FliesSound`).
//!
//! The counts live on the chunk ([`IsoChunk::corpse_count`], the
//! original's `IsoChunk.corpseCount` and `IsoChunkLevel.corpseCount`):
//! `None` until [`chunk_loaded`], and a level only counts while the chunk
//! has it (`getOrCreateLevelData` returns null for a level the chunk does
//! not have, and a dropped level loses its counts).

use std::collections::HashMap;

use pz_core::debug::DebugType;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};

use crate::chunk::{IsoCell, IsoChunk};
use crate::defs::BuildingIdx;
use crate::java::fastfloor;
use crate::meta_grid::IsoMetaGrid;

/// Initial value of mutable `CorpseCount.maxCorpseCount`; never its live authority.
pub const INITIAL_MAX_CORPSE_COUNT: i32 = 25;

/// `CorpseCount.ChunkLevelData`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorpseLevelCount {
    /// `corpseCount`: corpses outside buildings.
    pub corpse_count: i32,
    /// `buildingCorpseCount`: corpses per building (`None` until the
    /// first corpse in a building).
    pub building_corpse_count: Option<HashMap<BuildingIdx, i32>>,
}

impl CorpseLevelCount {
    /// `corpseAdded(building)`.
    pub fn corpse_added(&mut self, building: Option<BuildingIdx>) {
        match building {
            None => self.corpse_count = self.corpse_count.wrapping_add(1),
            Some(b) => {
                let count = self
                    .building_corpse_count
                    .get_or_insert_with(HashMap::new)
                    .entry(b)
                    .or_insert(0);
                *count = count.wrapping_add(1);
            }
        }
    }

    /// `corpseRemoved(building)`: a building's count leaves the map at
    /// zero; the outside count may go negative.
    pub fn corpse_removed(&mut self, building: Option<BuildingIdx>) {
        match building {
            None => self.corpse_count = self.corpse_count.wrapping_sub(1),
            Some(b) => {
                if let Some(m) = self.building_corpse_count.as_mut() {
                    if let Some(c) = m.get_mut(&b) {
                        if *c > 1 {
                            *c -= 1;
                        } else {
                            m.remove(&b);
                        }
                    }
                }
            }
        }
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.corpse_count = 0;
        if let Some(m) = self.building_corpse_count.as_mut() {
            m.clear();
        }
    }
}

/// `CorpseCount.ChunkData`: the chunk's level counts by level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorpseChunkCount {
    levels: HashMap<i32, CorpseLevelCount>,
}

impl CorpseChunkCount {
    /// `getLevelData(z)` (`None` before the level's first corpse).
    pub fn level(&self, z: i32) -> Option<&CorpseLevelCount> {
        self.levels.get(&z)
    }

    /// `IsoChunkLevel` release: a level the chunk no longer has loses its
    /// counts (`corpseCount = null`).
    pub(crate) fn retain_levels(&mut self, min_level: i32, max_level: i32) {
        self.levels
            .retain(|z, _| (min_level..=max_level).contains(z));
    }

    /// `reset()` / `removeFromWorld()`: every level of the chunk reset.
    pub fn reset(&mut self) {
        for l in self.levels.values_mut() {
            l.reset();
        }
    }
}

/// `CorpseCount.chunkLoaded(chunk)`: the chunk's counts created, or reset.
pub fn chunk_loaded(chunk: &mut IsoChunk) {
    chunk
        .corpse_count
        .get_or_insert_with(Default::default)
        .reset();
}

/// `IsoChunk.removeFromWorld`'s `corpseCount.removeFromWorld()`.
pub fn chunk_removed_from_world(chunk: &mut IsoChunk) {
    if let Some(c) = chunk.corpse_count.as_mut() {
        c.reset();
    }
}

/// `IsoGridSquare.getBuilding()` of the square at `(x, y, z)`: its room's
/// building (`None` without a square or room).
pub fn square_building(
    cell: &IsoCell,
    meta: &IsoMetaGrid,
    x: i32,
    y: i32,
    z: i32,
) -> HostResult<Option<BuildingIdx>> {
    let Some(square) = cell.get_grid_square(x, y, z) else {
        return Ok(None);
    };
    let building = square
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("CorpseCount building square"))?
        .room_def(meta)
        .and_then(|room| room.building);
    Ok(building)
}

fn check_z(x: i32, y: i32, z: i32) -> bool {
    if !(-32..=31).contains(&z) {
        DebugType::General.error(None, &format!("invalid z-coordinate {x},{y},{z}"));
        return false;
    }
    true
}

fn change(
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    x: i32,
    y: i32,
    z: i32,
    added: bool,
) -> HostResult<()> {
    if !check_z(x, y, z) {
        return Ok(());
    }
    let chunk_key = (x.div_euclid(8), y.div_euclid(8));
    // Native outer CorpseCount admits getChunkData before the inner
    // ChunkData method reads the square/building. Genuine absence skips
    // that read, including a held unrelated square body.
    if !cell
        .chunks
        .get(&chunk_key)
        .is_some_and(|chunk| chunk.corpse_count.is_some())
    {
        return Ok(());
    }
    let building = square_building(cell, meta, x, y, z)?;
    // This building query is callback-free over the same exclusive cell;
    // the admitted receiver cannot be replaced during it. Level admission
    // follows the building read, at getOrCreateLevelData's native phase.
    let ch = cell
        .chunks
        .get_mut(&chunk_key)
        .expect("admitted callback-free corpse-count chunk");
    let valid = ch.is_valid_level(z);
    let data = ch
        .corpse_count
        .as_mut()
        .expect("admitted callback-free corpse-count receiver");
    // `getOrCreateLevelData(z)`.
    if !valid {
        return Ok(());
    }
    let l = data.levels.entry(z).or_default();
    if added {
        l.corpse_added(building);
    } else {
        l.corpse_removed(building);
    }
    Ok(())
}

/// `CorpseCount.corpseAdded(x, y, z)`.
pub fn corpse_added(
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    x: i32,
    y: i32,
    z: i32,
) -> HostResult<()> {
    change(cell, meta, x, y, z, true)
}

/// `CorpseCount.corpseRemoved(x, y, z)`.
pub fn corpse_removed(
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    x: i32,
    y: i32,
    z: i32,
) -> HostResult<()> {
    change(cell, meta, x, y, z, false)
}

/// `CorpseCount.getCorpseCount(wx, wy, z, building)`: the corpses of the
/// nine chunks around chunk `(wx, wy)` on level `z` outside (no building)
/// or in `building`, rereading current `max_corpse_count` at each native
/// threshold check; with the sandbox's
/// `ZombieHealthImpact` (`zombie_count` is then `getZombieCount()` of a
/// square), also the zombies on the squares within 12 of the chunk's
/// corner that are in the same building.
pub fn get_corpse_count(
    cell: &IsoCell,
    meta: &IsoMetaGrid,
    wx: i32,
    wy: i32,
    z: i32,
    building: Option<BuildingIdx>,
    max_corpse_count: &dyn Fn() -> HostResult<i32>,
    zombie_count: Option<&dyn Fn(i32, i32, i32) -> i32>,
) -> HostResult<i32> {
    let mut count = 0_i32;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let Some(data) = cell
                .chunks
                .get(&(wx + dx, wy + dy))
                .and_then(|c| c.corpse_count.as_ref())
            else {
                continue;
            };
            let Some(l) = data.level(z) else {
                continue;
            };
            match building {
                None => count = count.wrapping_add(l.corpse_count),
                Some(b) => {
                    if let Some(c) = l.building_corpse_count.as_ref().and_then(|m| m.get(&b)) {
                        count = count.wrapping_add(*c);
                    }
                }
            }
            if count >= max_corpse_count()? {
                return Ok(count);
            }
        }
    }
    if let Some(zombies) = zombie_count {
        let (x, y) = (wx * 8, wy * 8);
        for dy in -12..=12 {
            for dx in -12..=12 {
                let Some(sq) = cell.get_grid_square(x + dx, y + dy, z) else {
                    continue;
                };
                let same_building = {
                    sq.try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("CorpseCount zombie square"))?
                        .room_def(meta)
                        .and_then(|r| r.building)
                        == building
                };
                if same_building {
                    count = count.wrapping_add(zombies(x + dx, y + dy, z));
                }
                if count >= max_corpse_count()? {
                    return Ok(count);
                }
            }
        }
    }
    Ok(count)
}

/// `CorpseCount.getCorpseCount(chr)` for a character at `(x, y, z)`
/// (`getXi()`, `getYi()`, `getZi()`) whose current square is `square`
/// (0 without one) and whose building is `building`.
#[allow(clippy::too_many_arguments)]
pub fn get_corpse_count_at(
    cell: &IsoCell,
    meta: &IsoMetaGrid,
    has_square: bool,
    xi: i32,
    yi: i32,
    zi: i32,
    building: Option<BuildingIdx>,
    max_corpse_count: &dyn Fn() -> HostResult<i32>,
    zombie_count: Option<&dyn Fn(i32, i32, i32) -> i32>,
) -> HostResult<i32> {
    if !has_square {
        return Ok(0);
    }
    let div = |v: i32| coorddivision(v, 8);
    get_corpse_count(
        cell,
        meta,
        div(xi),
        div(yi),
        zi,
        building,
        max_corpse_count,
        zombie_count,
    )
}

/// Scalar part of `hasBuildingCorpseCount` after the caller admits the
/// actual nonnull `chunk.corpseCount` receiver. Null receiver handling is
/// a distinct native phase and is never coerced into false here.
pub fn has_building_corpse_count(data: &CorpseChunkCount, z: i32, building: BuildingIdx) -> bool {
    data.level(z)
        .and_then(|l| l.building_corpse_count.as_ref())
        .is_some_and(|m| m.contains_key(&building))
}

/// `PZMath.coorddivision(value, divisor)` (through `float`).
pub fn coorddivision(value: i32, divisor: i32) -> i32 {
    fastfloor(value as f32 / divisor as f32)
}
