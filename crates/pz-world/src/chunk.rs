//! `IsoCell`, `IsoChunk`, `IsoGridSquare` and lot placement
//! (`IsoCell.PlaceLot`, `CellLoader.LoadCellBinaryChunk`).
//!
//! Object creation is the generic path of `CellLoader.DoTileObjectCreation`
//! (one `IsoObject` per tile). Specialised object classes, stacking fixes,
//! world regions and pathfinding hooks are not ported yet.

use std::collections::HashMap;

use crate::defs::BuildingIdx;
use crate::fix2x::Fix2xTable;
use crate::iso_lot::IsoLot;
use crate::java::{fastfloor, GameRand};
use crate::map_files::{header_name, lotpack_name};
use crate::meta_grid::IsoMetaGrid;
use crate::tiledef::{TileDefFile, TILE_DEFINITION_FILES};

pub const CHUNK_WIDTH: i32 = 8;
pub const MIN_LEVEL: i32 = -32;
pub const MAX_LEVEL: i32 = 31;

/// What lot placement needs from `IsoSpriteManager.namedMap`.
pub trait SpriteLookup {
    /// Sprite id for a tile name, or `None` for a missing tile definition.
    fn sprite_id(&self, name: &str) -> Option<i32>;
}

/// Sprite names and ids registered from the `.tiles` files, as
/// `IsoSpriteManager.AddSprite(name, id)` does (a duplicate name keeps the
/// first id; patch files add no sprites).
#[derive(Default)]
pub struct TileDefSprites {
    named: HashMap<String, i32>,
}

impl TileDefSprites {
    pub fn load(media_dir: &std::path::Path) -> std::io::Result<Self> {
        let mut s = TileDefSprites::default();
        for (name, number) in TILE_DEFINITION_FILES {
            s.add_file(&TileDefFile::read(&media_dir.join(name), number)?);
        }
        Ok(s)
    }

    pub fn add_file(&mut self, file: &TileDefFile) {
        if file.patch {
            return;
        }
        for ts in &file.tilesets {
            for tile in &ts.tiles {
                self.named
                    .entry(tile.name.clone())
                    .or_insert(file.sprite_id(ts, tile));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.named.len()
    }

    pub fn is_empty(&self) -> bool {
        self.named.is_empty()
    }
}

impl SpriteLookup for TileDefSprites {
    fn sprite_id(&self, name: &str) -> Option<i32> {
        self.named.get(name).copied()
    }
}

/// `IsoObject` created from a map tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoObject {
    pub sprite_name: String,
    pub sprite_id: i32,
}

/// `IsoGridSquare` (map-loading fields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoGridSquare {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub room_id: i64,
    /// `roofHideBuilding`: the building of the `emptyoutside` room here.
    pub roof_hide_building: Option<BuildingIdx>,
    pub objects: Vec<IsoObject>,
}

impl IsoGridSquare {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        IsoGridSquare {
            x,
            y,
            z,
            room_id: -1,
            roof_hide_building: None,
            objects: Vec::new(),
        }
    }
}

/// `IsoChunk` (map-loading fields).
pub struct IsoChunk {
    pub wx: i32,
    pub wy: i32,
    pub min_level: i32,
    pub max_level: i32,
    /// `squares[z - minLevel][x + y*8]`.
    squares: Vec<Vec<Option<IsoGridSquare>>>,
    /// `lotheader`: header file name, set when the lot came from the
    /// top-level header.
    pub lotheader: Option<String>,
    pub modded: bool,
}

impl IsoChunk {
    pub fn new(wx: i32, wy: i32) -> Self {
        IsoChunk {
            wx,
            wy,
            min_level: 0,
            max_level: 0,
            squares: vec![vec![None; 64]],
            lotheader: None,
            modded: false,
        }
    }

    pub fn is_valid_level(&self, z: i32) -> bool {
        z >= self.min_level && z <= self.max_level
    }

    pub fn set_min_max_level(&mut self, min_level: i32, max_level: i32) {
        if min_level == self.min_level && max_level == self.max_level {
            return;
        }
        let mut levels = Vec::with_capacity((max_level - min_level + 1) as usize);
        for z in min_level..=max_level {
            if self.is_valid_level(z) {
                levels.push(std::mem::take(
                    &mut self.squares[(z - self.min_level) as usize],
                ));
            } else {
                levels.push(vec![None; 64]);
            }
        }
        self.min_level = min_level;
        self.max_level = max_level;
        self.squares = levels;
    }

    pub fn set_square(&mut self, x: i32, y: i32, z: i32, square: Option<IsoGridSquare>) {
        self.set_min_max_level(self.min_level.min(z), self.max_level.max(z));
        let zz = (z - self.min_level) as usize;
        self.squares[zz][(x + y * 8) as usize] = square;
    }

    pub fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<&IsoGridSquare> {
        if !(0..8).contains(&x) || !(0..8).contains(&y) || z > self.max_level || z < self.min_level
        {
            return None;
        }
        self.squares[(z - self.min_level) as usize][(y * 8 + x) as usize].as_ref()
    }

    pub fn get_grid_square_mut(&mut self, x: i32, y: i32, z: i32) -> Option<&mut IsoGridSquare> {
        if !(0..8).contains(&x) || !(0..8).contains(&y) || z > self.max_level || z < self.min_level
        {
            return None;
        }
        self.squares[(z - self.min_level) as usize][(y * 8 + x) as usize].as_mut()
    }

    pub fn squares(&self) -> impl Iterator<Item = &IsoGridSquare> {
        self.squares.iter().flatten().flatten()
    }
}

/// Runtime inputs chunk loading takes from other systems.
pub struct ChunkLoadEnv<'a> {
    /// `IsoSpriteManager.namedMap`.
    pub sprites: &'a dyn SpriteLookup,
    /// `IsoChunk.Fix2xMap`, read from the installed game.
    pub fix2x: &'a Fix2xTable,
    /// `Rand`.
    pub rand: &'a mut dyn GameRand,
}

struct PlaceError;

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
    if result.is_err() {
        eprintln!("Failed to load chunk, blocking out area");
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
    let info = lot.info(&meta.map_files).ok_or(PlaceError)?;
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
                    return Err(PlaceError);
                }
                let off = lot.offset_in_data[square_xyz as usize];
                if off == -1 {
                    continue;
                }
                let num_ints = *lot.data.get(off as usize).ok_or(PlaceError)?;
                if num_ints <= 0 {
                    continue;
                }
                if !done[di] {
                    done[di] = true;
                    *count += 1;
                }
                if ch.get_grid_square(x - wx, y - wy, z).is_none() {
                    ch.set_square(x - wx, y - wy, z, Some(IsoGridSquare::new(x, y, z)));
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
                            Some(IsoGridSquare::new(x + xx, y + yy, z)),
                        );
                    }
                }
                let room_id = meta.get_room_at(x, y, z).map_or(-1, |r| r.id);
                let roof_hide = meta.get_empty_outside_at(x, y, z).map(|r| r.building);
                let square = ch.get_grid_square_mut(x - wx, y - wy, z).unwrap();
                square.room_id = room_id;
                if let Some(b) = roof_hide {
                    square.roof_hide_building = b;
                }
                for n in 0..num_ints {
                    let tile_idx = *lot
                        .data
                        .get(off as usize + 1 + n as usize)
                        .ok_or(PlaceError)?;
                    let tile = info.tiles_used.get(tile_idx as usize).ok_or(PlaceError)?;
                    let tile = if info.fixed2x {
                        tile.clone()
                    } else {
                        env.fix2x.fix2x(tile, env.rand)
                    };
                    let Some(id) = env.sprites.sprite_id(&tile) else {
                        eprintln!("Missing tile definition: {tile}");
                        continue;
                    };
                    // `bClearObjects` is always true upstream (its solidfloor /
                    // hidewalls test only re-sets it), so the first tile with a
                    // definition always clears the square.
                    if n == 0 {
                        square.objects.clear();
                    }
                    square.objects.push(IsoObject {
                        sprite_name: tile,
                        sprite_id: id,
                    });
                }
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
        chunk.modded |= meta.map_files[mf]
            .info_file_modded
            .get(&pack)
            .copied()
            .unwrap_or(false);
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
            chunk.modded |= meta.map_files[i]
                .info_file_modded
                .get(&pack)
                .copied()
                .unwrap_or(false);
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
    cache: LotCache,
}

impl IsoCell {
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
    pub fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<&IsoGridSquare> {
        let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
        self.chunks
            .get(&(wx, wy))?
            .get_grid_square(x - wx * 8, y - wy * 8, z)
    }
}
