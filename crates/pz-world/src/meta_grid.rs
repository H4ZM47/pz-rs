//! `IsoMetaGrid`, `IsoMetaCell`, `IsoMetaChunk` and the lot header reader
//! (`IsoMetaGrid.MetaGridLoaderThread.loadCell`).

use std::collections::HashMap;
use std::io;

use pz_core::debug::{self, DebugType, FmtArg, StackElement};
use pz_fs::GameFs;

use crate::bra::BufferedRandomAccessFile;
use crate::defs::{
    building_id, room_id, BuildingDef, BuildingIdx, MetaObject, RoomDef, RoomIdx, RoomRect,
};
use crate::java::{coordmodulo, fastfloor, io_throwable, java_trim, GameRand};
use crate::map_files::{
    header_name, lot_directories, zombie_intensity_for_chunk, MapFiles, MapInfo, LOTHEADER_MAGIC,
};
use crate::zones::{ClipperOps, RoomTone, Zone, ZoneId, ZoneTables};

/// `IsoDirections` ordinals used by `LotHeader.adjacentCells`.
pub mod dir {
    pub const N: usize = 0;
    pub const NW: usize = 1;
    pub const W: usize = 2;
    pub const SW: usize = 3;
    pub const S: usize = 4;
    pub const SE: usize = 5;
    pub const E: usize = 6;
    pub const NE: usize = 7;
}

/// Lua-defined zombie density modifiers (`IsoWorld.getZombieVoronois`).
/// Each returns 1024 multipliers indexed `wy * 32 + wx` for a cell.
pub trait ZombieVoronoi {
    fn evaluate_cell_cutoff(&mut self, cell_x: i32, cell_y: i32) -> Vec<f64>;
    fn release_cell(&mut self, _cell_x: i32, _cell_y: i32) {}
}

/// `IsoMetaChunk` (rooms, zones and zombie intensity). Zone methods are in
/// [`crate::meta_zones`].
#[derive(Clone, Debug, Default)]
pub struct IsoMetaChunk {
    zombie_intensity: u8,
    rooms: Vec<RoomIdx>,
    pub(crate) zones: Vec<ZoneId>,
}

impl IsoMetaChunk {
    pub fn get_zombie_intensity(&self) -> u8 {
        self.zombie_intensity
    }
    pub fn set_zombie_intensity(&mut self, v: u8) {
        self.zombie_intensity = v;
    }
    pub fn rooms(&self) -> &[RoomIdx] {
        &self.rooms
    }
    pub fn add_room(&mut self, room: RoomIdx) {
        self.rooms.push(room);
    }
    pub fn remove_room(&mut self, room: RoomIdx) {
        if let Some(i) = self.rooms.iter().position(|&r| r == room) {
            self.rooms.remove(i);
        }
    }

    /// `getRoomAt`: newest user-defined room wins, else the newest room.
    pub fn get_room_at(&self, rooms: &[RoomDef], x: i32, y: i32, z: i32) -> Option<RoomIdx> {
        let mut non_user_defined = None;
        for &idx in self.rooms.iter().rev() {
            let room = &rooms[idx.0 as usize];
            if room.is_empty_outside() || room.level != z || !room.contains(x, y) {
                continue;
            }
            if room.user_defined {
                return Some(idx);
            }
            if non_user_defined.is_none() {
                non_user_defined = Some(idx);
            }
        }
        non_user_defined
    }

    /// `getAssociatedBuildingAt(x, y)`: the building of the first room
    /// (of a building with a level >= 0) whose rects grown by one tile to
    /// the north and west contain the point. `Err` is the NPE of a room
    /// without a building.
    pub fn get_associated_building_at(
        &self,
        rooms: &[RoomDef],
        buildings: &[BuildingDef],
        x: i32,
        y: i32,
    ) -> Result<Option<BuildingIdx>, String> {
        for &idx in &self.rooms {
            let room = &rooms[idx.0 as usize];
            let b = room.building.ok_or("NullPointerException")?;
            if buildings[b.0 as usize].max_level(rooms) < 0 {
                continue;
            }
            for rr in &room.rects {
                if rr.x - 1 > x || rr.y - 1 > y || x > rr.get_x2() || y > rr.get_y2() {
                    continue;
                }
                return Ok(Some(b));
            }
        }
        Ok(None)
    }

    /// `getBuildingsIntersecting(x, y, w, h, result)`: the buildings of this
    /// chunk's rooms that intersect the rectangle, each once, skipping the
    /// empty-outside rooms of buildings that have rooms. The original adds a
    /// room's null building (or throws for an empty-outside room without
    /// one); a room here always has a building, and one without is skipped.
    #[allow(clippy::too_many_arguments)]
    pub fn get_buildings_intersecting(
        &self,
        rooms: &[RoomDef],
        buildings: &[BuildingDef],
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        result: &mut Vec<BuildingIdx>,
    ) {
        for &idx in &self.rooms {
            let room = &rooms[idx.0 as usize];
            let Some(b) = room.building else { continue };
            if room.is_empty_outside() && !buildings[b.0 as usize].rooms.is_empty()
                || result.contains(&b)
                || !room.intersects(x, y, w, h)
            {
                continue;
            }
            result.push(b);
        }
    }

    pub fn get_empty_outside_at(
        &self,
        rooms: &[RoomDef],
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<RoomIdx> {
        self.rooms.iter().copied().find(|idx| {
            let room = &rooms[idx.0 as usize];
            room.is_empty_outside() && room.level == z && room.contains(x, y)
        })
    }
}

/// `IsoMetaCell` (map-data fields).
#[derive(Debug)]
pub struct IsoMetaCell {
    wx: i32,
    wy: i32,
    pub(crate) chunk_map: Vec<Option<IsoMetaChunk>>,
    /// Header name of `info` within the highest-priority `MapFiles` that has it.
    pub info: Option<(usize, String)>,
    pub rooms: HashMap<i64, RoomIdx>,
    pub room_by_meta_id: HashMap<i64, RoomIdx>,
    pub room_list: Vec<RoomIdx>,
    pub buildings: Vec<BuildingIdx>,
    pub building_by_meta_id: HashMap<i64, BuildingIdx>,
    pub vehicle_zones: Vec<ZoneId>,
    pub mannequin_zones: Vec<ZoneId>,
    /// Lazily created upstream (`null` until the first world-gen zone).
    pub world_gen_zones: Option<Vec<ZoneId>>,
    pub animal_zones: Vec<ZoneId>,
    /// `animalZonesGenerated` (`metagrid/metacell_<x>_<y>.bin`): the
    /// procedural animal paths were generated for this cell
    /// (`checkAnimalZonesGenerated`).
    pub animal_zones_generated: std::cell::Cell<bool>,
    pub room_tones: Vec<RoomTone>,
    /// Keys of `isoBuildings`: buildings whose runtime `IsoBuilding` has been
    /// created (by [`IsoMetaGrid::get_room_by_id`]).
    pub iso_buildings: std::cell::RefCell<std::collections::HashSet<i64>>,
}

impl IsoMetaCell {
    pub fn new(wx: i32, wy: i32) -> Self {
        IsoMetaCell {
            wx,
            wy,
            chunk_map: vec![None; 1024],
            info: None,
            rooms: HashMap::new(),
            room_by_meta_id: HashMap::new(),
            room_list: Vec::new(),
            buildings: Vec::new(),
            building_by_meta_id: HashMap::new(),
            vehicle_zones: Vec::new(),
            mannequin_zones: Vec::new(),
            world_gen_zones: None,
            animal_zones: Vec::new(),
            animal_zones_generated: Default::default(),
            room_tones: Vec::new(),
            iso_buildings: Default::default(),
        }
    }
    pub fn get_x(&self) -> i32 {
        self.wx
    }
    pub fn get_y(&self) -> i32 {
        self.wy
    }
    pub fn has_chunk(&self, x: i32, y: i32) -> bool {
        (0..32).contains(&x)
            && (0..32).contains(&y)
            && self.chunk_map[(y * 32 + x) as usize].is_some()
    }
    pub fn chunk(&self, x: i32, y: i32) -> Option<&IsoMetaChunk> {
        if !(0..32).contains(&x) || !(0..32).contains(&y) {
            return None;
        }
        self.chunk_map[(y * 32 + x) as usize].as_ref()
    }

    /// `IsoMetaCell.removeRoom(room)`: removes the room from the chunks of
    /// this cell its bounds cover. Like the original's `getChunk(i)`, a
    /// missing chunk would be created (empty); here it is skipped, which
    /// leaves the same rooms.
    pub fn remove_room(&mut self, rooms: &[RoomDef], room: RoomIdx) {
        let r = &rooms[room.0 as usize];
        let mut xmax = r.x2 / 8;
        if r.x2 % 8 == 0 {
            xmax -= 1;
        }
        let mut ymax = r.y2 / 8;
        if r.y2 % 8 == 0 {
            ymax -= 1;
        }
        let (ox, oy) = (self.wx * 256, self.wy * 256);
        for yy in r.y / 8..=ymax {
            for xx in r.x / 8..=xmax {
                if xx < ox / 8 || xx >= (ox + 256) / 8 || yy < oy / 8 || yy >= (oy + 256) / 8 {
                    continue;
                }
                let i = (xx - ox / 8 + (yy - oy / 8) * 32) as usize;
                if let Some(c) = self.chunk_map[i].as_mut() {
                    c.remove_room(room);
                }
            }
        }
    }

    /// `IsoMetaCell.removeRooms(rooms, userDefined)`: `user_defined` is -1
    /// (all), 0 (only rooms that are not user-defined) or 1 (only
    /// user-defined rooms).
    pub fn remove_rooms(&mut self, rooms: &[RoomDef], list: &[RoomIdx], user_defined: i32) {
        for &r in list {
            let ud = rooms[r.0 as usize].user_defined;
            if user_defined != -1 && (user_defined != 1 || !ud) && (user_defined != 0 || ud) {
                continue;
            }
            self.remove_room(rooms, r);
        }
    }

    /// `IsoMetaCell.getBuildingsIntersecting(x, y, w, h, result)`.
    #[allow(clippy::too_many_arguments)]
    pub fn get_buildings_intersecting(
        &self,
        rooms: &[RoomDef],
        buildings: &[BuildingDef],
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        result: &mut Vec<BuildingIdx>,
    ) {
        let mut xmax = (x + w) / 8;
        if (x + w) % 8 == 0 {
            xmax -= 1;
        }
        let mut ymax = (y + h) / 8;
        if (y + h) % 8 == 0 {
            ymax -= 1;
        }
        let (ox, oy) = (self.wx * 256, self.wy * 256);
        for yy in y / 8..=ymax {
            for xx in x / 8..=xmax {
                if xx < ox / 8 || xx >= (ox + 256) / 8 || yy < oy / 8 || yy >= (oy + 256) / 8 {
                    continue;
                }
                let i = (xx - ox / 8 + (yy - oy / 8) * 32) as usize;
                if let Some(c) = &self.chunk_map[i] {
                    c.get_buildings_intersecting(rooms, buildings, x, y, w, h, result);
                }
            }
        }
    }
}

/// `IsoMetaGrid`: all map cells' rooms and buildings.
pub struct IsoMetaGrid {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub min_non_procedural_x: i32,
    pub min_non_procedural_y: i32,
    pub max_non_procedural_x: i32,
    pub max_non_procedural_y: i32,
    pub width: i32,
    pub height: i32,
    pub(crate) grid: Vec<Option<IsoMetaCell>>,
    /// `IsoLot.MapFiles`, by priority (index 0 is the highest).
    pub map_files: Vec<MapFiles>,
    /// `IsoLot.InfoHeaders`: header name to the highest-priority map files index.
    pub info_headers: HashMap<String, usize>,
    /// `IsoLot.InfoHeaderNames`.
    pub info_header_names: Vec<String>,
    /// `IsoLot.InfoFileNames`.
    pub info_file_names: HashMap<String, std::path::PathBuf>,
    /// `IsoLot.InfoFileModded`.
    pub info_file_modded: HashMap<String, bool>,
    /// Room and building arenas.
    pub room_defs: Vec<RoomDef>,
    pub building_defs: Vec<BuildingDef>,
    /// `IsoMetaGrid.buildings`.
    pub buildings: Vec<BuildingIdx>,
    /// Zone arena; zones are never freed, removal only unlinks them.
    pub zone_defs: Vec<Zone>,
    /// `IsoMetaGrid.zones`.
    pub zones: Vec<ZoneId>,
    /// `IsoMetaGrid.vehiclesZones`.
    pub vehicles_zones: Vec<ZoneId>,
    /// `animalZoneHandler` contents in registration order (the original
    /// keys them by random UUID).
    pub animal_zones: Vec<ZoneId>,
    /// The most zones `animalZoneHandler`'s `HashMap` has held (its table
    /// never shrinks; it sets the save order).
    pub animal_zones_peak: usize,
    /// `cellsToSave` (absolute cell coordinates, each once): the cells
    /// whose `metacell_*.bin` the next save writes.
    pub cells_to_save: std::cell::RefCell<Vec<(i32, i32)>>,
    /// Runtime game tables used by zones.
    pub zone_tables: ZoneTables,
    /// The native clipper (`Clipper` / `ClipperOffset`).
    pub clipper: Option<Box<dyn ClipperOps + Send>>,
}

/// World generation cell bounds (`WorldGenParams.getMin/MaxXCell`).
#[derive(Clone, Copy, Debug)]
pub struct WorldGenBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl Default for WorldGenBounds {
    /// No procedural area: does not widen the grid.
    fn default() -> Self {
        WorldGenBounds {
            min_x: i32::MAX,
            min_y: i32::MAX,
            max_x: i32::MIN,
            max_y: i32::MIN,
        }
    }
}

/// Inputs to `IsoMetaGrid.Create` that come from other systems.
pub struct CreateParams<'a> {
    pub game_map: &'a str,
    pub world_gen: WorldGenBounds,
    pub rand: &'a mut dyn GameRand,
    pub voronois: Vec<Box<dyn ZombieVoronoi + 'a>>,
}

impl IsoMetaGrid {
    /// A grid with no map files, cells, rooms or buildings.
    pub fn empty() -> Self {
        IsoMetaGrid {
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            min_non_procedural_x: 0,
            min_non_procedural_y: 0,
            max_non_procedural_x: 0,
            max_non_procedural_y: 0,
            width: 1,
            height: 1,
            grid: std::iter::once(None).collect(),
            map_files: Vec::new(),
            info_headers: HashMap::new(),
            info_header_names: Vec::new(),
            info_file_names: HashMap::new(),
            info_file_modded: HashMap::new(),
            room_defs: Vec::new(),
            building_defs: Vec::new(),
            buildings: Vec::new(),
            zone_defs: Vec::new(),
            zones: Vec::new(),
            vehicles_zones: Vec::new(),
            animal_zones: Vec::new(),
            animal_zones_peak: 0,
            cells_to_save: Default::default(),
            zone_tables: ZoneTables::default(),
            clipper: None,
        }
    }

    pub fn get_cell(&self, x: i32, y: i32) -> Option<&IsoMetaCell> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        self.grid[(x * self.height + y) as usize].as_ref()
    }

    /// `setCell(x, y, cell)` (grid-relative). Panics outside the grid, like
    /// the original's array store; the debug-only coordinate check is not
    /// ported.
    pub fn set_cell(&mut self, x: i32, y: i32, cell: Option<IsoMetaCell>) {
        assert!(
            x >= 0 && y >= 0 && x < self.width && y < self.height,
            "cell {x},{y} outside the grid"
        );
        *self.cell_slot(x, y) = cell;
    }

    /// An empty grid covering cells `min..=max` with no map files, as the
    /// state `Create()` leaves when no lot headers exist in that range.
    pub fn with_bounds(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> IsoMetaGrid {
        let wg = WorldGenBounds {
            min_x,
            min_y,
            max_x,
            max_y,
        };
        let mut grid =
            Self::from_map_files_unloaded(Vec::new(), wg).expect("with_bounds needs min <= max");
        grid.min_x = min_x;
        grid.min_y = min_y;
        grid.max_x = max_x;
        grid.max_y = max_y;
        grid.width = max_x - min_x + 1;
        grid.height = max_y - min_y + 1;
        grid.grid.clear();
        grid.grid
            .resize_with((grid.width * grid.height) as usize, || None);
        grid.min_non_procedural_x = min_x;
        grid.min_non_procedural_y = min_y;
        grid.max_non_procedural_x = max_x;
        grid.max_non_procedural_y = max_y;
        grid
    }

    pub(crate) fn cell_slot(&mut self, x: i32, y: i32) -> &mut Option<IsoMetaCell> {
        &mut self.grid[(x * self.height + y) as usize]
    }

    /// `getCellData(cellX, cellY)` in absolute cell coordinates.
    pub fn get_cell_data(&self, x: i32, y: i32) -> Option<&IsoMetaCell> {
        self.get_cell(x - self.min_x, y - self.min_y)
    }

    /// `getCellData(cellX, cellY)`, mutable.
    pub fn get_cell_data_mut(&mut self, x: i32, y: i32) -> Option<&mut IsoMetaCell> {
        let (gx, gy) = (x - self.min_x, y - self.min_y);
        if gx < 0 || gy < 0 || gx >= self.width || gy >= self.height {
            return None;
        }
        self.cell_slot(gx, gy).as_mut()
    }

    /// `IsoMetaCell.getChunk(i)`: creates the chunk on first use, seeding its
    /// zombie intensity from the lot headers.
    pub fn meta_chunk_mut(
        &mut self,
        cell_x: i32,
        cell_y: i32,
        i: usize,
    ) -> Option<&mut IsoMetaChunk> {
        let exists = self.get_cell_data(cell_x, cell_y)?.chunk_map[i].is_some();
        if !exists {
            let info = self.get_cell_data(cell_x, cell_y)?.info.clone();
            let header = info
                .as_ref()
                .and_then(|(mf, name)| self.map_files[*mf].info_headers.get(name));
            let intensity = zombie_intensity_for_chunk(
                &self.map_files,
                header,
                (i % 32) as i32,
                (i / 32) as i32,
            );
            let chunk = IsoMetaChunk {
                zombie_intensity: if intensity >= 0 { intensity as u8 } else { 0 },
                rooms: Vec::new(),
                zones: Vec::new(),
            };
            self.get_cell_data_mut(cell_x, cell_y)?.chunk_map[i] = Some(chunk);
        }
        self.get_cell_data_mut(cell_x, cell_y)?.chunk_map[i].as_mut()
    }

    /// `getChunkData(chunkX, chunkY)` without creating the chunk.
    pub fn get_chunk_data(&self, chunk_x: i32, chunk_y: i32) -> Option<&IsoMetaChunk> {
        let cell_x = fastfloor(chunk_x as f32 / 32.0);
        let cell_y = fastfloor(chunk_y as f32 / 32.0);
        self.get_cell_data(cell_x, cell_y)?
            .chunk(chunk_x - cell_x * 32, chunk_y - cell_y * 32)
    }

    pub fn get_chunk_data_from_tile(&self, x: i32, y: i32) -> Option<&IsoMetaChunk> {
        self.get_chunk_data(fastfloor(x as f32 / 8.0), fastfloor(y as f32 / 8.0))
    }

    /// `getRoomAt(x, y, z)`. A missing chunk has no rooms, so not creating it
    /// here is equivalent apart from allocation.
    pub fn get_room_at(&self, x: i32, y: i32, z: i32) -> Option<&RoomDef> {
        let idx = self
            .get_chunk_data_from_tile(x, y)?
            .get_room_at(&self.room_defs, x, y, z)?;
        Some(&self.room_defs[idx.0 as usize])
    }

    pub fn get_empty_outside_at(&self, x: i32, y: i32, z: i32) -> Option<&RoomDef> {
        let idx =
            self.get_chunk_data_from_tile(x, y)?
                .get_empty_outside_at(&self.room_defs, x, y, z)?;
        Some(&self.room_defs[idx.0 as usize])
    }

    /// `getAssociatedBuildingAt(x, y)`: this square's chunk first, then the
    /// neighbouring chunks across the edges the square touches.
    pub fn get_associated_building_at(
        &self,
        x: i32,
        y: i32,
    ) -> Result<Option<BuildingIdx>, String> {
        let at = |dx: i32, dy: i32| -> Result<Option<BuildingIdx>, String> {
            match self.get_chunk_data_from_tile(x + dx, y + dy) {
                Some(c) => c.get_associated_building_at(&self.room_defs, &self.building_defs, x, y),
                None => Ok(None),
            }
        };
        if let Some(b) = at(0, 0)? {
            return Ok(Some(b));
        }
        let xmod = coordmodulo(x, 8);
        let ymod = coordmodulo(y, 8);
        // (condition, IsoDirections dx, dy) in the original order.
        let checks = [
            (xmod == 7, 1, 0),
            (xmod == 0, -1, 0),
            (ymod == 0, 0, -1),
            (xmod == 0 || ymod == 0, -1, -1),
            (xmod == 7 || ymod == 7, 1, 1),
            (xmod == 0 || ymod == 7, -1, 1),
            (xmod == 7 || ymod == 0, 1, -1),
        ];
        for (cond, dx, dy) in checks {
            if cond {
                if let Some(b) = at(dx, dy)? {
                    return Ok(Some(b));
                }
            }
        }
        Ok(None)
    }

    /// `getRoomByID(id)`: the room, creating its runtime `IsoRoom` /
    /// `IsoBuilding` on first use. A new `IsoBuilding` draws its
    /// `scoreUpdate` from `Rand`; that draw is the only state kept here (the
    /// runtime room and building objects are not ported). A room without a
    /// building would also get a new `BuildingDef` (another draw); map data
    /// has none, and that case is not ported.
    pub fn get_room_by_id(&self, id: i64, rand: &mut dyn GameRand) -> Option<&RoomDef> {
        let cell = self.get_cell_data(room_id::get_cell_x(id), room_id::get_cell_y(id))?;
        let room = &self.room_defs[cell.rooms.get(&id)?.0 as usize];
        if let Some(b) = room.building {
            let bid = self.building_defs[b.0 as usize].id;
            if cell.iso_buildings.borrow_mut().insert(bid) {
                // `new IsoBuilding()`: `scoreUpdate = -120 + Rand.Next(120)`.
                rand.next(120);
            }
        }
        Some(room)
    }

    pub fn get_room_def_by_id(&self, id: i64) -> Option<&RoomDef> {
        let cell = self.get_cell_data(room_id::get_cell_x(id), room_id::get_cell_y(id))?;
        Some(&self.room_defs[cell.rooms.get(&id)?.0 as usize])
    }

    /// `isValidChunk(wx, wy)`: the chunk's first square is inside the grid.
    pub fn is_valid_chunk(&self, wx: i32, wy: i32) -> bool {
        let (x, y) = (wx.wrapping_mul(8), wy.wrapping_mul(8));
        x >= self.min_x * 256
            && x < (self.max_x + 1) * 256
            && y >= self.min_y * 256
            && y < (self.max_y + 1) * 256
    }

    /// `isValidSquare(x, y)`, with Java's `int` overflow: a grid with no
    /// cells yet (`minX = 10000000`, `maxX = -10000000`) wraps around and
    /// takes every square.
    pub fn is_valid_square(&self, x: i32, y: i32) -> bool {
        x >= self.min_x.wrapping_mul(256)
            && x < self.max_x.wrapping_add(1).wrapping_mul(256)
            && y >= self.min_y.wrapping_mul(256)
            && y < self.max_y.wrapping_add(1).wrapping_mul(256)
    }

    /// `IsoMetaCell.addRoom(room, cellX*256, cellY*256)`.
    pub fn add_room_to_cell(&mut self, cell_x: i32, cell_y: i32, room: RoomIdx) {
        let (ox, oy) = (cell_x * 256, cell_y * 256);
        let r = &self.room_defs[room.0 as usize];
        let (rx, ry, rx2, ry2) = (r.x, r.y, r.x2, r.y2);
        let mut xmax = rx2 / 8;
        if rx2 % 8 == 0 {
            xmax -= 1;
        }
        let mut ymax = ry2 / 8;
        if ry2 % 8 == 0 {
            ymax -= 1;
        }
        for yy in ry / 8..=ymax {
            for xx in rx / 8..=xmax {
                if xx < ox / 8 || xx >= (ox + 256) / 8 || yy < oy / 8 || yy >= (oy + 256) / 8 {
                    continue;
                }
                let i = (xx - ox / 8 + (yy - oy / 8) * 32) as usize;
                if let Some(c) = self.meta_chunk_mut(cell_x, cell_y, i) {
                    c.add_room(room);
                }
            }
        }
    }

    fn higher_priority_300x300_cell_exists(&self, priority: usize, c300x: i32, c300y: i32) -> bool {
        self.map_files
            .iter()
            .take(priority)
            .any(|mf| mf.has_cell300(c300x, c300y))
    }

    /// `IsoMetaGrid.Create()` for the parts that read map files: directory
    /// scan, lot headers (rooms, buildings, zombie intensity), building
    /// consolidation, adjacent-cell flags and the adjacent-cell rooms of
    /// `CreateStep2`'s building loop. The alarms of that loop are
    /// [`IsoMetaGrid::assign_alarms`].
    ///
    /// The original reads headers on 8 threads (rows `minY+i`, step 8) that
    /// share `Rand`; this port reads them in that per-thread row order on one
    /// thread, thread 0 first.
    pub fn create(fs: &GameFs, params: CreateParams<'_>) -> io::Result<IsoMetaGrid> {
        let CreateParams {
            game_map,
            world_gen,
            rand,
            mut voronois,
        } = params;
        let mut map_files = Vec::new();
        for (i, lot_dir) in lot_directories(fs, game_map).into_iter().enumerate() {
            let Some(path) = fs.resolve(&format!("media/maps/{lot_dir}/")) else {
                continue;
            };
            if !path.is_dir() {
                continue;
            }
            let abs = std::path::absolute(&path).unwrap_or(path);
            let index = map_files.len();
            map_files.push(MapFiles::new(&lot_dir, abs, i, index));
        }
        let mut grid = Self::from_map_files(fs, map_files, world_gen)?;
        grid.load_headers(rand, &mut voronois);
        grid.consolidate_buildings();
        grid.init_incomplete_cells();
        // The first half of `CreateStep2`'s building loop. The original
        // interleaves it with the alarm draws ([`IsoMetaGrid::assign_alarms`]);
        // it draws nothing and the alarms read no chunk room lists, so
        // running it first gives the same state, and every caller gets the
        // rooms of buildings that cross into the cells east, south and
        // south-east.
        for i in 0..grid.buildings.len() {
            let b = grid.buildings[i];
            grid.add_rooms_to_adjacent_cells(b);
        }
        Ok(grid)
    }

    fn from_map_files(
        fs: &GameFs,
        mut map_files: Vec<MapFiles>,
        wg: WorldGenBounds,
    ) -> io::Result<IsoMetaGrid> {
        for mf in &mut map_files {
            // `ChooseGameInfo.getMapDetails(name).isFixed2x()`
            let fixed2x = MapInfo::load(fs, &mf.map_directory_name)
                .map(|m| m.fixed2x)
                .unwrap_or(false);
            if mf.load(fixed2x, false) {
                mf.post_load();
            }
        }
        Self::from_map_files_unloaded(map_files, wg)
    }

    fn from_map_files_unloaded(
        map_files: Vec<MapFiles>,
        wg: WorldGenBounds,
    ) -> io::Result<IsoMetaGrid> {
        let (mut min_x, mut min_y, mut max_x, mut max_y) =
            (10000000, 10000000, -10000000, -10000000);
        let mut info_headers = HashMap::new();
        let mut info_header_names: Vec<String> = Vec::new();
        let mut info_file_names = HashMap::new();
        let mut info_file_modded = HashMap::new();
        for (i, mf) in map_files.iter().enumerate().rev() {
            for (k, v) in &mf.info_file_names {
                info_file_names.insert(k.clone(), v.clone());
            }
            for (k, v) in &mf.info_file_modded {
                info_file_modded.insert(k.clone(), *v);
            }
            for k in mf.info_headers.keys() {
                info_headers.insert(k.clone(), i);
            }
            info_header_names.retain(|n| !mf.info_header_names.contains(n));
            info_header_names.extend(mf.info_header_names.iter().cloned());
            min_x = min_x.min(mf.min_x);
            min_y = min_y.min(mf.min_y);
            max_x = max_x.max(mf.max_x);
            max_y = max_y.max(mf.max_y);
        }
        if min_x > max_x {
            min_y = 0;
            min_x = 0;
            max_y = 1;
            max_x = 1;
        }
        let (np_min_x, np_min_y, np_max_x, np_max_y) = (min_x, min_y, max_x, max_y);
        min_x = min_x.min(wg.min_x);
        min_y = min_y.min(wg.min_y);
        max_x = max_x.max(wg.max_x);
        max_y = max_y.max(wg.max_y);
        if max_x < min_x || max_y < min_y {
            return Err(io::Error::other("Failed to find any .lotheader files"));
        }
        let width = max_x - min_x + 1;
        let height = max_y - min_y + 1;
        let mut grid = Vec::new();
        grid.resize_with((width * height) as usize, || None);
        Ok(IsoMetaGrid {
            min_x,
            min_y,
            max_x,
            max_y,
            min_non_procedural_x: np_min_x,
            min_non_procedural_y: np_min_y,
            max_non_procedural_x: np_max_x,
            max_non_procedural_y: np_max_y,
            width,
            height,
            grid,
            map_files,
            info_headers,
            info_header_names,
            info_file_names,
            info_file_modded,
            room_defs: Vec::new(),
            building_defs: Vec::new(),
            buildings: Vec::new(),
            zone_defs: Vec::new(),
            zones: Vec::new(),
            vehicles_zones: Vec::new(),
            animal_zones: Vec::new(),
            animal_zones_peak: 0,
            cells_to_save: Default::default(),
            zone_tables: ZoneTables::default(),
            clipper: None,
        })
    }

    fn load_headers(
        &mut self,
        rand: &mut dyn GameRand,
        voronois: &mut [Box<dyn ZombieVoronoi + '_>],
    ) {
        for thread in 0..8 {
            let mut w_y = self.min_y + thread;
            while w_y <= self.max_y {
                for w_x in self.min_x..=self.max_x {
                    for mf in 0..self.map_files.len() {
                        self.load_cell(mf, w_x, w_y, rand, voronois);
                    }
                }
                w_y += 8;
            }
        }
    }

    /// `MetaGridLoaderThread.loadCell(mapFiles, wX, wY)`.
    fn load_cell(
        &mut self,
        mf: usize,
        w_x: i32,
        w_y: i32,
        rand: &mut dyn GameRand,
        voronois: &mut [Box<dyn ZombieVoronoi + '_>],
    ) {
        let name = header_name(w_x, w_y);
        let Some(path) = self.map_files[mf].info_file_names.get(&name).cloned() else {
            return;
        };
        if !self.map_files[mf].info_headers.contains_key(&name) || !path.exists() {
            return;
        }
        let (gx, gy) = (w_x - self.min_x, w_y - self.min_y);
        if self.cell_slot(gx, gy).is_none() {
            let mut cell = IsoMetaCell::new(w_x, w_y);
            cell.info = Some((mf, name.clone()));
            *self.cell_slot(gx, gy) = Some(cell);
        }
        if let Err(e) = self.read_header(mf, &name, &path, w_x, w_y, rand, voronois) {
            // `DebugLog.log("ERROR loading " + fo.getAbsolutePath())` then
            // `ExceptionLogger.logException(e)`.
            let abs = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
            debug::log(&format!("ERROR loading {}", abs.display()));
            debug::log_exception(&io_throwable(&e), Some(&loader_thread_caller()));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn read_header(
        &mut self,
        mf: usize,
        name: &str,
        path: &std::path::Path,
        w_x: i32,
        w_y: i32,
        rand: &mut dyn GameRand,
        voronois: &mut [Box<dyn ZombieVoronoi + '_>],
    ) -> io::Result<()> {
        let mut f = BufferedRandomAccessFile::open(path, 4096)?;
        let mut magic = [0u8; 4];
        f.read_into(&mut magic)?;
        if magic != LOTHEADER_MAGIC {
            f.seek(0)?;
        }
        let priority = self.map_files[mf].priority;
        let version = f.read_int_le()?;
        {
            let info = self.map_files[mf].info_headers.get_mut(name).unwrap();
            info.version = version;
            if !(0..=1).contains(&version) {
                return Err(io::Error::other(format!("Unsupported version {version}")));
            }
            let tilecount = f.read_int_le()?;
            for _ in 0..tilecount {
                let s = f.get_next_line()?;
                // NPE on a null line upstream.
                let s = s.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
                info.tiles_used.push(java_trim(&s).to_string());
            }
            if version == 0 {
                f.read()?;
            }
            info.width = f.read_int_le()?;
            info.height = f.read_int_le()?;
            if info.width != 8 || info.height != info.width {
                return Err(io::Error::other(format!(
                    "invalid chunk width {}x{} : {}",
                    info.width,
                    info.height,
                    path.display()
                )));
            }
            if version == 0 {
                info.min_level = 0;
                info.max_level = f.read_int_le()? - 1;
            } else {
                info.min_level = f.read_int_le()?;
                info.max_level = f.read_int_le()?;
            }
        }
        let mut room_list: Vec<RoomIdx> = Vec::new();
        let num_rooms = f.read_int_le()?;
        for _ in 0..num_rooms {
            let s = f
                .get_next_line()?
                .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
            let mut room = RoomDef::new(0, s);
            room.level = f.read_int_le()?;
            let rects = f.read_int_le()?;
            for _ in 0..rects {
                let rx = f.read_int_le()?;
                let ry = f.read_int_le()?;
                let rw = f.read_int_le()?;
                let rh = f.read_int_le()?;
                room.rects
                    .push(RoomRect::new(rx + w_x * 256, ry + w_y * 256, rw, rh));
            }
            room.CalculateBounds();
            let n_objects = f.read_int_le()?;
            for _ in 0..n_objects {
                let e = f.read_int_le()?;
                let x = f.read_int_le()?;
                let y = f.read_int_le()?;
                room.objects.push(MetaObject {
                    kind: e,
                    x: x + w_x * 256 - room.x,
                    y: y + w_y * 256 - room.y,
                    used: false,
                });
            }
            room.lights_active.set(rand.next(4) == 0);
            room_list.push(RoomIdx(self.room_defs.len() as u32));
            self.room_defs.push(room);
        }
        let mut had_error = false;
        for &idx in &room_list {
            let cell = self.get_cell_data_mut(w_x, w_y).unwrap();
            let id = room_id::make_id(w_x, w_y, cell.room_list.len() as i32);
            let room = &mut self.room_defs[idx.0 as usize];
            room.id = id;
            room.meta_id = room.calculate_meta_id(w_x, w_y);
            let (meta_id, rx, ry, level) = (room.meta_id, room.x, room.y, room.level);
            let cell = self.get_cell_data_mut(w_x, w_y).unwrap();
            cell.rooms.insert(id, idx);
            if cell.room_by_meta_id.contains_key(&meta_id) && !had_error {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let _ =
                    debug::with(|d| {
                        d.error(
                        DebugType::General,
                        Some(&loader_thread_caller()),
                        "duplicate RoomDef.metaID for room at x=%d, y=%d, level=%d, filename=%s",
                        &[rx.into(), ry.into(), level.into(), FmtArg::from(&*file_name)],
                    )
                    });
                had_error = true;
            }
            cell.room_by_meta_id.insert(meta_id, idx);
            cell.room_list.push(idx);
        }
        let num_buildings = f.read_int_le()?;
        for n in 0..num_buildings {
            let mut b = BuildingDef::new(rand);
            let bidx = BuildingIdx(self.building_defs.len() as u32);
            let numb_rooms = f.read_int_le()?;
            b.id = building_id::make_id(w_x, w_y, n);
            for _ in 0..numb_rooms {
                let room_index = f.read_int_le()?;
                let ridx = *room_list
                    .get(room_index as usize)
                    .ok_or_else(|| io::Error::other("room index out of bounds"))?;
                let room = &mut self.room_defs[ridx.0 as usize];
                room.building = Some(bidx);
                if room.is_empty_outside() {
                    b.emptyoutside.push(ridx);
                } else {
                    b.rooms.push(ridx);
                }
            }
            b.CalculateBounds(&self.room_defs);
            let (c300x, c300y) = (b.x / 300, b.y / 300);
            if self.higher_priority_300x300_cell_exists(priority, c300x, c300y) {
                // The def stays referenced by its rooms but is not registered.
                self.building_defs.push(b);
                continue;
            }
            b.meta_id = b.calculate_meta_id(&self.room_defs, w_x, w_y);
            let meta_id = b.meta_id;
            self.building_defs.push(b);
            let cell = self.get_cell_data_mut(w_x, w_y).unwrap();
            cell.buildings.push(bidx);
            cell.building_by_meta_id.insert(meta_id, bidx);
        }
        let mut zombie_intensity = [0u8; 1024];
        let n_bytes = f.read_into(&mut zombie_intensity)?;
        if n_bytes != zombie_intensity.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("wx={w_x}, wy={w_y}, nBytes={n_bytes}, this.zombieIntensity.length=1024"),
            ));
        }
        let values: Vec<Vec<f64>> = voronois
            .iter_mut()
            .map(|v| v.evaluate_cell_cutoff(w_x, w_y))
            .collect();
        for wx in 0..32 {
            for wy in 0..32 {
                let mut intensity = zombie_intensity[(wx * 32 + wy) as usize] as i32;
                let mut v2 = 1.0f64;
                for vals in &values {
                    v2 *= vals[(wy * 32 + wx) as usize];
                }
                intensity = (intensity as f64 * v2) as i32;
                intensity = intensity.clamp(0, 255);
                self.map_files[mf]
                    .info_headers
                    .get_mut(name)
                    .unwrap()
                    .set_zombie_intensity((wx + wy * 32) as usize, intensity as u8);
                let cell = self.get_cell_data(w_x, w_y).unwrap();
                if !cell.has_chunk(wx, wy) {
                    continue;
                }
                let (cx, cy) = (cell.get_x(), cell.get_y());
                if self.higher_priority_300x300_cell_exists(
                    priority,
                    (cx * 256 + wx * 8) / 300,
                    (cy * 256 + wy * 8) / 300,
                ) {
                    continue;
                }
                let cell = self.get_cell_data_mut(w_x, w_y).unwrap();
                cell.chunk_map[(wy * 32 + wx) as usize]
                    .as_mut()
                    .unwrap()
                    .set_zombie_intensity(intensity as u8);
            }
        }
        for v in voronois.iter_mut() {
            v.release_cell(w_x, w_y);
        }
        Ok(())
    }

    /// `consolidateBuildings()`: header maps are visited in
    /// `java.util.HashMap` iteration order, which fixes `buildings` order.
    fn consolidate_buildings(&mut self) {
        for mf in 0..self.map_files.len() {
            let priority = self.map_files[mf].priority;
            let names: Vec<&str> = self.map_files[mf]
                .info_header_names
                .iter()
                .map(String::as_str)
                .collect();
            let ordered: Vec<(i32, i32)> = crate::java::hashmap_order(&names)
                .into_iter()
                .map(|n| {
                    let h = &self.map_files[mf].info_headers[n];
                    (h.cell_x, h.cell_y)
                })
                .collect();
            for (cell_x, cell_y) in ordered {
                // NPE upstream if the header failed to create a cell.
                let Some(cell) = self.get_cell_data(cell_x, cell_y) else {
                    continue;
                };
                let builds = cell.buildings.clone();
                let (mx, my) = (cell.get_x(), cell.get_y());
                for bidx in builds {
                    let b = &self.building_defs[bidx.0 as usize];
                    if self.higher_priority_300x300_cell_exists(priority, b.x / 300, b.y / 300) {
                        continue;
                    }
                    self.buildings.push(bidx);
                    let rooms: Vec<RoomIdx> = b
                        .rooms
                        .iter()
                        .chain(b.emptyoutside.iter())
                        .copied()
                        .collect();
                    for r in rooms {
                        self.add_room_to_cell(mx, my, r);
                    }
                }
            }
        }
    }

    fn init_incomplete_cells(&mut self) {
        for mf in &mut self.map_files {
            for cell_y in mf.min_y..=mf.max_y {
                for cell_x in mf.min_x..=mf.max_x {
                    if mf.get_lot_header(cell_x, cell_y).is_none() {
                        continue;
                    }
                    let mut adj = [false; 8];
                    let cx = coordmodulo(cell_x * 256, 300) > 0;
                    let cy = coordmodulo(cell_y * 256, 300) > 0;
                    if cx {
                        adj[dir::W] = mf.has_cell(cell_x - 1, cell_y);
                        adj[dir::E] = mf.has_cell(cell_x + 1, cell_y);
                    }
                    if cy {
                        adj[dir::N] = mf.has_cell(cell_x, cell_y - 1);
                        adj[dir::S] = mf.has_cell(cell_x, cell_y + 1);
                    }
                    if cx && cy {
                        adj[dir::NW] = mf.has_cell(cell_x - 1, cell_y - 1);
                        adj[dir::NE] = mf.has_cell(cell_x + 1, cell_y - 1);
                        adj[dir::SE] = mf.has_cell(cell_x + 1, cell_y + 1);
                        adj[dir::SW] = mf.has_cell(cell_x - 1, cell_y + 1);
                    }
                    let h = mf.get_lot_header_mut(cell_x, cell_y).unwrap();
                    for (i, set) in [(dir::W, cx), (dir::E, cx), (dir::N, cy), (dir::S, cy)] {
                        if set {
                            h.adjacent_cells[i] = adj[i];
                        }
                    }
                    if cx && cy {
                        for i in [dir::NW, dir::NE, dir::SE, dir::SW] {
                            h.adjacent_cells[i] = adj[i];
                        }
                    }
                }
            }
        }
    }

    /// `addRoomsToAdjacentCells(buildingDef)`: a building that crosses
    /// into the cells east, south and south-east of its own also registers
    /// its rooms and empty-outside rooms with those cells' chunks.
    pub fn add_rooms_to_adjacent_cells(&mut self, b: BuildingIdx) {
        let def = &self.building_defs[b.0 as usize];
        let cell_x = def.x / 256;
        let cell_y = def.y / 256;
        let cell_x2 = (def.x2 - 1) / 256;
        let cell_y2 = (def.y2 - 1) / 256;
        if cell_x2 == cell_x && cell_y2 == cell_y {
            return;
        }
        let rooms = def.rooms.clone();
        let empty = def.emptyoutside.clone();
        for dy in 0..=1 {
            for dx in 0..=1 {
                if dx == 0 && dy == 0 || self.get_cell_data(cell_x + dx, cell_y + dy).is_none() {
                    continue;
                }
                for &r in rooms.iter().chain(empty.iter()) {
                    self.add_room_to_cell(cell_x + dx, cell_y + dy, r);
                }
            }
        }
    }

    /// The alarm half of `CreateStep2`'s building loop: alarms are assigned
    /// from the sandbox `alarm` option. The other half
    /// (`addRoomsToAdjacentCells`) already ran in [`IsoMetaGrid::create`].
    pub fn assign_alarms(
        &mut self,
        rand: &mut dyn GameRand,
        last_stand: bool,
        power_grid_exists: bool,
        alarm_option: i32,
        alarm_decay: impl Fn() -> i32,
    ) {
        for i in 0..self.buildings.len() {
            let b = self.buildings[i].0 as usize;
            if last_stand || self.building_defs[b].rooms.len() <= 2 {
                continue;
            }
            let mut rand_alarm = if power_grid_exists { 9 } else { 11 };
            match alarm_option {
                1 => rand_alarm = -1,
                2 => rand_alarm += 5,
                3 => rand_alarm += 3,
                5 => rand_alarm -= 3,
                6 => rand_alarm -= 5,
                _ => {}
            }
            if rand_alarm > -1 {
                self.building_defs[b].alarmed = rand.next(rand_alarm) == 0;
            }
            if self.building_defs[b].alarmed {
                self.building_defs[b].alarm_decay = alarm_decay();
            }
        }
    }
}

/// Building and room editing used by `zombie.basements.Basements`
/// (`addBasementBuildingDefsToLotHeaders`, `addBasementBuildingDefsToMetaGrid`
/// and the merge of access pieces into the building they touch).
impl IsoMetaGrid {
    /// A grid covering cells `min..=max` with no map files and no cells;
    /// add cells with [`IsoMetaGrid::create_cell`].
    pub fn blank(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> Self {
        let mut g = Self::empty();
        g.min_x = min_x;
        g.min_y = min_y;
        g.max_x = max_x;
        g.max_y = max_y;
        g.min_non_procedural_x = min_x;
        g.min_non_procedural_y = min_y;
        g.max_non_procedural_x = max_x;
        g.max_non_procedural_y = max_y;
        g.width = max_x - min_x + 1;
        g.height = max_y - min_y + 1;
        g.grid = Vec::new();
        g.grid.resize_with((g.width * g.height) as usize, || None);
        g
    }

    /// `new IsoMetaCell(x, y)` stored at absolute cell `(x, y)`, or the cell
    /// already there. `None` outside the grid.
    pub fn create_cell(&mut self, x: i32, y: i32) -> Option<&mut IsoMetaCell> {
        let (gx, gy) = (x - self.min_x, y - self.min_y);
        if gx < 0 || gy < 0 || gx >= self.width || gy >= self.height {
            return None;
        }
        let slot = self.cell_slot(gx, gy);
        if slot.is_none() {
            *slot = Some(IsoMetaCell::new(x, y));
        }
        slot.as_mut()
    }

    /// Adds `def` and its rooms to the arenas, as the tail of
    /// `createBasementBuildingDefs` does: each room's `building` is set,
    /// `rooms` / `emptyoutside` are appended to the def's lists in order and
    /// the def's bounds are calculated. Ids, meta ids and cells are left to
    /// [`IsoMetaGrid::add_building_to_lot_header`] /
    /// [`IsoMetaGrid::merge_buildings`].
    pub fn insert_building(
        &mut self,
        mut def: BuildingDef,
        rooms: Vec<RoomDef>,
        emptyoutside: Vec<RoomDef>,
    ) -> BuildingIdx {
        let bidx = BuildingIdx(self.building_defs.len() as u32);
        for (list, is_empty) in [(rooms, false), (emptyoutside, true)] {
            for mut room in list {
                room.building = Some(bidx);
                let ridx = RoomIdx(self.room_defs.len() as u32);
                self.room_defs.push(room);
                if is_empty {
                    def.emptyoutside.push(ridx);
                } else {
                    def.rooms.push(ridx);
                }
            }
        }
        def.CalculateBounds(&self.room_defs);
        self.building_defs.push(def);
        bidx
    }

    /// Registers a new building with cell `(cell_x, cell_y)` and the grid:
    /// [`IsoMetaGrid::insert_building`], then the no-merge branch of
    /// `addBasementBuildingDefsToLotHeaders` and the per-building step of
    /// `addBasementBuildingDefsToMetaGrid`, both for that cell (the original
    /// uses `x / 256`, `y / 256` of the def). `None` if the cell does not
    /// exist (the original skips the building).
    ///
    /// The original runs the lot-header step for every basement before the
    /// meta-grid step of any; callers reproducing that order use the two
    /// steps directly.
    pub fn add_building(
        &mut self,
        cell_x: i32,
        cell_y: i32,
        def: BuildingDef,
        rooms: Vec<RoomDef>,
        emptyoutside: Vec<RoomDef>,
    ) -> Option<BuildingIdx> {
        self.get_cell_data(cell_x, cell_y)?;
        let b = self.insert_building(def, rooms, emptyoutside);
        self.register_building_in_cell(cell_x, cell_y, b);
        self.add_building_rooms_to_cell(cell_x, cell_y, b);
        Some(b)
    }

    /// The no-merge branch of `addBasementBuildingDefsToLotHeaders` for
    /// building `b` in cell `x / 256`, `y / 256`. `false` if there is no
    /// such cell.
    pub fn add_building_to_lot_header(&mut self, b: BuildingIdx) -> bool {
        let d = &self.building_defs[b.0 as usize];
        let (cell_x, cell_y) = (d.x / 256, d.y / 256);
        if self.get_cell_data(cell_x, cell_y).is_none() {
            return false;
        }
        self.register_building_in_cell(cell_x, cell_y, b);
        true
    }

    fn register_building_in_cell(&mut self, cell_x: i32, cell_y: i32, b: BuildingIdx) {
        let index = self.get_cell_data(cell_x, cell_y).unwrap().buildings.len() as i32;
        let def = &mut self.building_defs[b.0 as usize];
        def.id = building_id::make_id(cell_x, cell_y, index);
        def.meta_id = def.calculate_meta_id(&self.room_defs, cell_x, cell_y);
        let (meta_id, bx, by) = (def.meta_id, def.x, def.y);
        // Only the rooms: the original does not merge `emptyoutside` here.
        let rooms = def.rooms.clone();
        self.merge_rooms_onto_meta_cell(cell_x, cell_y, &rooms);
        let cell = self.get_cell_data_mut(cell_x, cell_y).unwrap();
        cell.buildings.push(b);
        if cell.building_by_meta_id.contains_key(&meta_id) {
            let _ = debug::with(|d| {
                d.error(
                    DebugType::General,
                    None,
                    "duplicate BuildingDef.metaID for building at x=%d, y=%d",
                    &[bx.into(), by.into()],
                )
            });
        }
        cell.building_by_meta_id.insert(meta_id, b);
    }

    /// The per-building step of `addBasementBuildingDefsToMetaGrid` for cell
    /// `x / 256`, `y / 256`: rooms and empty-outside rooms go into that
    /// cell's chunks and the adjacent cells', and the building is appended
    /// to [`IsoMetaGrid::buildings`] unless already there. `false` if there
    /// is no such cell.
    pub fn add_building_to_meta_grid(&mut self, b: BuildingIdx) -> bool {
        let d = &self.building_defs[b.0 as usize];
        let (cell_x, cell_y) = (d.x / 256, d.y / 256);
        if self.get_cell_data(cell_x, cell_y).is_none() {
            return false;
        }
        self.add_building_rooms_to_cell(cell_x, cell_y, b);
        true
    }

    fn add_building_rooms_to_cell(&mut self, cell_x: i32, cell_y: i32, b: BuildingIdx) {
        let d = &self.building_defs[b.0 as usize];
        let (rooms, empty) = (d.rooms.clone(), d.emptyoutside.clone());
        self.add_rooms_to_cell(cell_x, cell_y, &rooms);
        self.add_rooms_to_cell(cell_x, cell_y, &empty);
        self.add_rooms_to_adjacent_cells(b);
        if !self.buildings.contains(&b) {
            self.buildings.push(b);
        }
    }

    /// The merged-rooms step of `addBasementBuildingDefsToMetaGrid`: rooms
    /// merged into `merge_def` go into the chunks of its cell (`x / 256`,
    /// `y / 256`) and of the adjacent cells. `false` if there is no such
    /// cell.
    pub fn add_merged_rooms_to_meta_grid(
        &mut self,
        merge_def: BuildingIdx,
        rooms: &[RoomIdx],
        emptyoutside: &[RoomIdx],
    ) -> bool {
        let d = &self.building_defs[merge_def.0 as usize];
        let (cell_x, cell_y) = (d.x / 256, d.y / 256);
        if self.get_cell_data(cell_x, cell_y).is_none() {
            return false;
        }
        self.add_rooms_to_cell(cell_x, cell_y, rooms);
        self.add_rooms_to_cell(cell_x, cell_y, emptyoutside);
        self.add_rooms_to_adjacent_cells_of(merge_def, rooms);
        self.add_rooms_to_adjacent_cells_of(merge_def, emptyoutside);
        true
    }

    /// `IsoMetaCell.addRooms(rooms, cellX*256, cellY*256)`.
    pub fn add_rooms_to_cell(&mut self, cell_x: i32, cell_y: i32, rooms: &[RoomIdx]) {
        for &r in rooms {
            self.add_room_to_cell(cell_x, cell_y, r);
        }
    }

    /// `addRoomsToAdjacentCells(buildingDef, roomDefs)`: like
    /// [`IsoMetaGrid::add_rooms_to_adjacent_cells`] for the given rooms; the
    /// building's own cell comes from `fastfloor`.
    pub fn add_rooms_to_adjacent_cells_of(&mut self, b: BuildingIdx, rooms: &[RoomIdx]) {
        let def = &self.building_defs[b.0 as usize];
        let cell_x = fastfloor(def.x as f32 / 256.0);
        let cell_y = fastfloor(def.y as f32 / 256.0);
        let cell_x2 = (def.x2 - 1) / 256;
        let cell_y2 = (def.y2 - 1) / 256;
        if cell_x2 == cell_x && cell_y2 == cell_y {
            return;
        }
        for dy in 0..=1 {
            for dx in 0..=1 {
                if dx == 0 && dy == 0 || self.get_cell_data(cell_x + dx, cell_y + dy).is_none() {
                    continue;
                }
                self.add_rooms_to_cell(cell_x + dx, cell_y + dy, rooms);
            }
        }
    }

    /// `removeRoomsFromAdjacentCells(rooms, cellX1, cellY1, cellX2, cellY2,
    /// userDefined)`.
    pub fn remove_rooms_from_adjacent_cells(
        &mut self,
        rooms: &[RoomIdx],
        (cell_x1, cell_y1): (i32, i32),
        (cell_x2, cell_y2): (i32, i32),
        user_defined: i32,
    ) {
        if cell_x2 == cell_x1 && cell_y2 == cell_y1 {
            return;
        }
        for cy in cell_y1..=cell_y2 {
            for cx in cell_x1..=cell_x2 {
                if cx == cell_x1 && cy == cell_y1 {
                    continue;
                }
                let (gx, gy) = (cx - self.min_x, cy - self.min_y);
                if gx < 0 || gy < 0 || gx >= self.width || gy >= self.height {
                    continue;
                }
                let idx = (gx * self.height + gy) as usize;
                if let Some(cell) = self.grid[idx].as_mut() {
                    cell.remove_rooms(&self.room_defs, rooms, user_defined);
                }
            }
        }
    }

    /// `getBuildingsIntersecting(x, y, w, h, result)`. The cell range runs
    /// to `(y + height) / 256` and `(x + width) / 256` with the grid's
    /// `width` / `height` in cells, as in the original.
    pub fn get_buildings_intersecting(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        result: &mut Vec<BuildingIdx>,
    ) {
        for cy in y / 256..=(y + self.height) / 256 {
            for cx in x / 256..=(x + self.width) / 256 {
                if cx < self.min_x || cx > self.max_x || cy < self.min_y || cy > self.max_y {
                    continue;
                }
                let Some(cell) = self.get_cell(cx - self.min_x, cy - self.min_y) else {
                    continue;
                };
                cell.get_buildings_intersecting(
                    &self.room_defs,
                    &self.building_defs,
                    x,
                    y,
                    w,
                    h,
                    result,
                );
            }
        }
    }

    /// `target.addRoomsOf(source, tempRooms)` on two buildings of the arena.
    pub fn add_rooms_of(&mut self, target: BuildingIdx, source: BuildingIdx) {
        let (t, s) = (target.0 as usize, source.0 as usize);
        if t == s {
            // `addRoomsOf(this)` appends to the list it walks and never
            // ends upstream; no caller does it.
            panic!("add_rooms_of: a building cannot absorb itself");
        }
        let (target_def, source_def) = if t < s {
            let (a, b) = self.building_defs.split_at_mut(s);
            (&mut a[t], &mut b[0])
        } else {
            let (a, b) = self.building_defs.split_at_mut(t);
            (&mut b[0], &mut a[s])
        };
        target_def.add_rooms_of(target, source_def, &mut self.room_defs);
    }

    /// `Basements.mergeRoomsOntoMetaCell(rooms, metaCell)`: each room gets
    /// the next room id of the cell (`roomList.size()`) and its meta id, and
    /// is added to `rooms`, `room_list` and `room_by_meta_id`. Chunks are
    /// not touched.
    pub fn merge_rooms_onto_meta_cell(&mut self, cell_x: i32, cell_y: i32, rooms: &[RoomIdx]) {
        for &idx in rooms {
            let cell = self.get_cell_data(cell_x, cell_y).expect("meta cell");
            let (cx, cy) = (cell.get_x(), cell.get_y());
            let id = room_id::make_id(cx, cy, cell.room_list.len() as i32);
            let room = &mut self.room_defs[idx.0 as usize];
            room.id = id;
            room.meta_id = room.calculate_meta_id(cx, cy);
            let (meta_id, rx, ry, level) = (room.meta_id, room.x, room.y, room.level);
            let cell = self.get_cell_data_mut(cell_x, cell_y).unwrap();
            if cell.rooms.contains_key(&id) {
                let _ = debug::with(|d| {
                d.error(
                    DebugType::General,
                    None,
                    "duplicate RoomDef.ID for room at x=%d, y=%d, level=%d",
                    &[rx.into(), ry.into(), level.into()],
                )
            });
            }
            cell.rooms.insert(id, idx);
            cell.room_list.push(idx);
            if cell.room_by_meta_id.contains_key(&meta_id) {
                let _ = debug::with(|d| {
                d.error(
                    DebugType::General,
                    None,
                    "duplicate RoomDef.metaID for room at x=%d, y=%d, level=%d",
                    &[rx.into(), ry.into(), level.into()],
                )
            });
            }
            cell.room_by_meta_id.insert(meta_id, idx);
        }
    }

    /// `Basements.removeBuildingFromMetaCell(buildingDef, metaCell)`: drops
    /// the building from `buildings` / `building_by_meta_id` and its rooms
    /// and empty-outside rooms from `rooms` (by their current id),
    /// `room_list` and `room_by_meta_id`. Chunk room lists are left alone,
    /// as in the original. Follow with
    /// [`IsoMetaGrid::recalculate_building_and_room_ids`].
    pub fn remove_building_from_meta_cell(&mut self, b: BuildingIdx, cell_x: i32, cell_y: i32) {
        let def = &self.building_defs[b.0 as usize];
        let (meta_id, bid) = (def.meta_id, def.id);
        let all: Vec<RoomIdx> = def.rooms.iter().chain(&def.emptyoutside).copied().collect();
        let cell = self
            .grid
            .get_mut({
                let (gx, gy) = (cell_x - self.min_x, cell_y - self.min_y);
                assert!(gx >= 0 && gy >= 0 && gx < self.width && gy < self.height);
                (gx * self.height + gy) as usize
            })
            .and_then(Option::as_mut)
            .expect("meta cell");
        if let Some(i) = cell.buildings.iter().position(|&x| x == b) {
            cell.buildings.remove(i);
        }
        cell.building_by_meta_id.remove(&meta_id);
        debug_assert!(!cell.iso_buildings.borrow().contains(&bid));
        for r in all {
            let room = &self.room_defs[r.0 as usize];
            cell.rooms.remove(&room.id);
            if let Some(i) = cell.room_list.iter().position(|&x| x == r) {
                cell.room_list.remove(i);
            }
            cell.room_by_meta_id.remove(&room.meta_id);
        }
    }

    /// `Basements.recalculateBuildingAndRoomIDs(metaCell)`: room ids follow
    /// `room_list` order and building ids `buildings` order; `rooms` and the
    /// runtime-building keys are rebuilt with the new ids. Meta-id maps are
    /// unchanged.
    pub fn recalculate_building_and_room_ids(&mut self, cell_x: i32, cell_y: i32) {
        let (gx, gy) = (cell_x - self.min_x, cell_y - self.min_y);
        assert!(gx >= 0 && gy >= 0 && gx < self.width && gy < self.height);
        let cell = self.grid[(gx * self.height + gy) as usize]
            .as_mut()
            .expect("meta cell");
        let (cx, cy) = (cell.get_x(), cell.get_y());
        cell.rooms.clear();
        for (i, &r) in cell.room_list.iter().enumerate() {
            let id = room_id::make_id(cx, cy, i as i32);
            self.room_defs[r.0 as usize].id = id;
            cell.rooms.insert(id, r);
        }
        // `isoBuildings` is keyed by `def.id`; re-key the entries whose
        // building is in this cell.
        let mut by_old_id: HashMap<i64, i64> = HashMap::new();
        for (i, &b) in cell.buildings.iter().enumerate() {
            let def = &mut self.building_defs[b.0 as usize];
            let new_id = building_id::make_id(cx, cy, i as i32);
            by_old_id.entry(def.id).or_insert(new_id);
            def.id = new_id;
        }
        let old: Vec<i64> = cell.iso_buildings.borrow_mut().drain().collect();
        let mut iso = cell.iso_buildings.borrow_mut();
        for id in old {
            iso.insert(by_old_id.get(&id).copied().unwrap_or(id));
        }
    }

    /// `Basements.getBuildingToMergeWith(buildingDef)`: the single building
    /// with no level below 0 that `b` is adjacent to (same level), found
    /// among the buildings intersecting `b`'s bounds grown by one tile.
    /// `None` if `b` itself goes below 0, or if there is none or more than
    /// one.
    pub fn get_building_to_merge_with(&mut self, b: BuildingIdx) -> Option<BuildingIdx> {
        if self.building_defs[b.0 as usize].get_min_level(&self.room_defs) < 0 {
            return None;
        }
        let d = &self.building_defs[b.0 as usize];
        let mut candidates = Vec::new();
        self.get_buildings_intersecting(
            d.x - 1,
            d.y - 1,
            d.get_w() + 2,
            d.get_h() + 2,
            &mut candidates,
        );
        let mut merge = None;
        for c in candidates {
            if self.building_defs[c.0 as usize].get_min_level(&self.room_defs) < 0 {
                continue;
            }
            let (bd, cd) = (
                &self.building_defs[b.0 as usize],
                &self.building_defs[c.0 as usize],
            );
            if !bd.is_adjacent(&self.room_defs, cd) {
                continue;
            }
            if merge.is_some() {
                return None;
            }
            merge = Some(c);
        }
        merge
    }

    /// `Basements.mergeBuildings(buildingDef, mergeDef)`: `b`'s rooms join
    /// `merge_def`. If `merge_def` lives in another cell than the lower of
    /// the two cells, it is moved there first (removed from its cell, ids
    /// recalculated in both). `b`'s rooms and empty-outside rooms are merged
    /// onto the cell, `merge_def` absorbs them (`addRoomsOf`) and its meta id
    /// is recalculated. Returns `false` on the original's early returns (a
    /// missing cell). The caller keeps the `MergedRooms` bookkeeping
    /// (`b`'s room lists, for [`IsoMetaGrid::add_merged_rooms_to_meta_grid`])
    /// and drops `b` from its own list either way.
    pub fn merge_buildings(&mut self, b: BuildingIdx, merge_def: BuildingIdx) -> bool {
        let (bd, md) = (
            &self.building_defs[b.0 as usize],
            &self.building_defs[merge_def.0 as usize],
        );
        let (cell_x1, cell_y1) = (bd.x / 256, bd.y / 256);
        let (cell_x2, cell_y2) = (md.x / 256, md.y / 256);
        let cell_x = cell_x1.min(cell_x2);
        let cell_y = cell_y1.min(cell_y2);
        if self.get_cell_data(cell_x, cell_y).is_none() {
            return false;
        }
        if cell_x != cell_x2 || cell_y != cell_y2 {
            if self.get_cell_data(cell_x2, cell_y2).is_none() {
                return false;
            }
            self.remove_building_from_meta_cell(merge_def, cell_x2, cell_y2);
            self.recalculate_building_and_room_ids(cell_x2, cell_y2);
            let cell = self.get_cell_data_mut(cell_x, cell_y).unwrap();
            let (cx, cy, n) = (cell.get_x(), cell.get_y(), cell.buildings.len() as i32);
            cell.buildings.push(merge_def);
            let md = &mut self.building_defs[merge_def.0 as usize];
            md.id = building_id::make_id(cx, cy, n);
            let moved: Vec<RoomIdx> = md.rooms.iter().chain(&md.emptyoutside).copied().collect();
            let cell = self.get_cell_data_mut(cell_x, cell_y).unwrap();
            cell.room_list.extend(moved);
            self.recalculate_building_and_room_ids(cell_x, cell_y);
        }
        let bd = &self.building_defs[b.0 as usize];
        let (rooms, empty) = (bd.rooms.clone(), bd.emptyoutside.clone());
        self.merge_rooms_onto_meta_cell(cell_x, cell_y, &rooms);
        self.merge_rooms_onto_meta_cell(cell_x, cell_y, &empty);
        let old_meta = self.building_defs[merge_def.0 as usize].meta_id;
        self.get_cell_data_mut(cell_x, cell_y)
            .unwrap()
            .building_by_meta_id
            .remove(&old_meta);
        self.add_rooms_of(merge_def, b);
        let md = &mut self.building_defs[merge_def.0 as usize];
        md.meta_id = md.calculate_meta_id(&self.room_defs, cell_x, cell_y);
        let meta = md.meta_id;
        self.get_cell_data_mut(cell_x, cell_y)
            .unwrap()
            .building_by_meta_id
            .insert(meta, merge_def);
        true
    }
}

#[cfg(test)]
mod building_api_tests {
    use super::*;

    struct Zero;
    impl GameRand for Zero {
        fn next(&mut self, _max: i32) -> i32 {
            0
        }
    }

    fn room(name: &str, level: i32, rects: &[(i32, i32, i32, i32)]) -> RoomDef {
        let mut r = RoomDef::new(0, name.into());
        r.level = level;
        for &(x, y, w, h) in rects {
            r.rects.push(RoomRect::new(x, y, w, h));
        }
        r.CalculateBounds();
        r
    }

    fn grid() -> IsoMetaGrid {
        let mut g = IsoMetaGrid::blank(0, 0, 1, 1);
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            g.create_cell(x, y).unwrap();
        }
        g
    }

    fn chunk_rooms(g: &IsoMetaGrid, wx: i32, wy: i32) -> Vec<RoomIdx> {
        g.get_chunk_data(wx, wy)
            .map(|c| c.rooms().to_vec())
            .unwrap_or_default()
    }

    #[test]
    fn add_building_assigns_ids_and_fills_cell() {
        let mut g = grid();
        let b0 = g
            .add_building(
                0,
                0,
                BuildingDef::new(&mut Zero),
                vec![room("kitchen", 0, &[(10, 10, 5, 5)])],
                vec![room("emptyoutside", 0, &[(8, 8, 2, 2)])],
            )
            .unwrap();
        let b1 = g
            .add_building(
                0,
                0,
                BuildingDef::new(&mut Zero),
                vec![
                    room("basement", -1, &[(40, 40, 4, 4)]),
                    room("storage", -1, &[(44, 40, 4, 4)]),
                ],
                vec![],
            )
            .unwrap();
        let cell = g.get_cell_data(0, 0).unwrap();
        // Rooms only (not empty-outside) are merged onto the cell lists.
        assert_eq!(cell.room_list, vec![RoomIdx(0), RoomIdx(2), RoomIdx(3)]);
        assert_eq!(cell.buildings, vec![b0, b1]);
        assert_eq!(
            g.building_defs[b1.0 as usize].id,
            building_id::make_id(0, 0, 1)
        );
        assert_eq!(g.room_defs[3].id, room_id::make_id(0, 0, 2));
        assert_eq!(cell.rooms[&room_id::make_id(0, 0, 2)], RoomIdx(3));
        assert_eq!(g.room_defs[1].id, 0, "empty-outside room keeps its id");
        assert_eq!(cell.room_by_meta_id[&g.room_defs[2].meta_id], RoomIdx(2));
        assert_eq!(
            cell.building_by_meta_id[&g.building_defs[b1.0 as usize].meta_id],
            b1
        );
        // Chunk lists: rooms, then empty-outside rooms, per building.
        assert_eq!(chunk_rooms(&g, 1, 1), vec![RoomIdx(0), RoomIdx(1)]);
        assert_eq!(chunk_rooms(&g, 5, 5), vec![RoomIdx(2), RoomIdx(3)]);
        assert_eq!(g.buildings, vec![b0, b1]);
        assert_eq!(g.get_room_at(46, 41, -1).unwrap().name, "storage");
    }

    #[test]
    fn building_across_cells_reaches_adjacent_chunks() {
        let mut g = grid();
        let b = g
            .add_building(
                0,
                0,
                BuildingDef::new(&mut Zero),
                vec![room("hall", 0, &[(250, 250, 12, 12)])],
                vec![],
            )
            .unwrap();
        for (wx, wy) in [(31, 31), (32, 31), (31, 32), (32, 32)] {
            assert_eq!(chunk_rooms(&g, wx, wy), vec![RoomIdx(0)], "{wx},{wy}");
        }
        assert_eq!(g.get_room_at(256, 256, 0).unwrap().building, Some(b));
        assert!(g.get_cell_data(1, 1).unwrap().room_list.is_empty());
    }

    #[test]
    fn adjacency_and_intersection() {
        let mut g = grid();
        let house = g
            .add_building(
                1,
                0,
                BuildingDef::new(&mut Zero),
                vec![room("living", 0, &[(256, 20, 10, 10)])],
                vec![room("emptyoutside", 0, &[(256, 30, 10, 2)])],
            )
            .unwrap();
        let rd = &g.room_defs;
        let hd = &g.building_defs[house.0 as usize];
        assert!(hd.is_adjacent_rect(rd, 250, 20, 6, 2, 0));
        assert!(!hd.is_adjacent_rect(rd, 250, 20, 5, 2, 0));
        assert!(!hd.is_adjacent_rect(rd, 250, 20, 6, 2, 1));
        assert!(hd.intersects(rd, 260, 25, 1, 1, 0));
        let mut out = Vec::new();
        // The cell range ends at `(x + width) / 256` with `width` = 2 cells
        // here, so cell 1 is only searched from x >= 254.
        g.get_buildings_intersecting(253, 19, 9, 4, &mut out);
        assert!(out.is_empty());
        g.get_buildings_intersecting(254, 19, 9, 4, &mut out);
        assert_eq!(out, vec![house]);
        out.clear();
        g.get_buildings_intersecting(200, 0, 40, 40, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn merge_access_piece_into_neighbour_moves_it_to_lower_cell() {
        let mut g = grid();
        // Two buildings in cell (1, 0); the house is the second.
        let other = g
            .add_building(
                1,
                0,
                BuildingDef::new(&mut Zero),
                vec![room("shed", 0, &[(400, 100, 4, 4)])],
                vec![],
            )
            .unwrap();
        let house = g
            .add_building(
                1,
                0,
                BuildingDef::new(&mut Zero),
                vec![
                    room("living", 0, &[(256, 20, 10, 10)]),
                    room("cellar", -1, &[(256, 20, 4, 4)]),
                ],
                vec![room("emptyoutside", 0, &[(256, 30, 10, 2)])],
            )
            .unwrap();
        // Access piece west of the house, in cell (0, 0). It must start at
        // x >= 255 for cell 1 to be searched (see `get_buildings_intersecting`).
        let access = g.insert_building(
            BuildingDef::new(&mut Zero),
            vec![room("stairs", 0, &[(255, 22, 1, 3)])],
            vec![room("emptyoutside", 0, &[(255, 25, 1, 1)])],
        );
        // The house goes below 0, so it is not a candidate.
        assert_eq!(g.get_building_to_merge_with(access), None);
        g.building_defs[house.0 as usize].reset_min_max_level();
        let cellar = g.building_defs[house.0 as usize].rooms[1];
        g.room_defs[cellar.0 as usize].level = 0;
        assert_eq!(g.get_building_to_merge_with(access), Some(house));
        assert!(g.merge_buildings(access, house));

        let c1 = g.get_cell_data(1, 0).unwrap();
        assert_eq!(c1.buildings, vec![other]);
        assert_eq!(c1.room_list, vec![RoomIdx(0)]);
        assert_eq!(c1.rooms.len(), 1);
        assert_eq!(c1.rooms[&room_id::make_id(1, 0, 0)], RoomIdx(0));

        let c0 = g.get_cell_data(0, 0).unwrap();
        assert_eq!(c0.buildings, vec![house]);
        let hd = &g.building_defs[house.0 as usize];
        assert_eq!(hd.id, building_id::make_id(0, 0, 0));
        // living, cellar, emptyoutside moved; then stairs and its emptyoutside.
        let expect = vec![RoomIdx(1), RoomIdx(2), RoomIdx(3), RoomIdx(4), RoomIdx(5)];
        assert_eq!(c0.room_list, expect);
        for (i, r) in expect.iter().enumerate() {
            assert_eq!(
                g.room_defs[r.0 as usize].id,
                room_id::make_id(0, 0, i as i32)
            );
            assert_eq!(c0.rooms[&room_id::make_id(0, 0, i as i32)], *r);
            assert_eq!(g.room_defs[r.0 as usize].building, Some(house));
        }
        assert_eq!(hd.rooms, vec![RoomIdx(1), RoomIdx(2), RoomIdx(4)]);
        assert_eq!(hd.emptyoutside, vec![RoomIdx(3), RoomIdx(5)]);
        assert_eq!((hd.x, hd.y, hd.x2, hd.y2), (255, 20, 266, 32));
        assert_eq!(c0.building_by_meta_id[&hd.meta_id], house);
        assert_eq!(hd.meta_id, hd.calculate_meta_id(&g.room_defs, 0, 0));

        // Meta-grid step for the merged rooms: into the chunks of cell (0, 0)
        // and, since the house now spans cells, of cell (1, 0).
        assert!(g.add_merged_rooms_to_meta_grid(house, &[RoomIdx(4)], &[RoomIdx(5)]));
        assert_eq!(
            chunk_rooms(&g, 31, 3),
            vec![RoomIdx(4), RoomIdx(5)],
            "access piece chunk"
        );
        assert_eq!(g.get_room_at(255, 23, 0).unwrap().name, "stairs");
    }

    #[test]
    fn remove_and_recalculate() {
        let mut g = grid();
        let mk = |g: &mut IsoMetaGrid, x| {
            g.add_building(
                0,
                0,
                BuildingDef::new(&mut Zero),
                vec![room("a", 0, &[(x, 0, 2, 2)]), room("b", 0, &[(x, 2, 2, 2)])],
                vec![],
            )
            .unwrap()
        };
        let a = mk(&mut g, 0);
        let b = mk(&mut g, 10);
        let c = mk(&mut g, 20);
        g.get_cell_data(0, 0)
            .unwrap()
            .iso_buildings
            .borrow_mut()
            .insert(building_id::make_id(0, 0, 2));
        g.remove_building_from_meta_cell(b, 0, 0);
        let cell = g.get_cell_data(0, 0).unwrap();
        assert_eq!(cell.buildings, vec![a, c]);
        // `rooms` lost b's ids (2, 3); recalculation renumbers.
        assert_eq!(cell.rooms.len(), 4);
        g.recalculate_building_and_room_ids(0, 0);
        let cell = g.get_cell_data(0, 0).unwrap();
        assert_eq!(
            cell.room_list,
            vec![RoomIdx(0), RoomIdx(1), RoomIdx(4), RoomIdx(5)]
        );
        assert_eq!(g.room_defs[4].id, room_id::make_id(0, 0, 2));
        assert_eq!(
            g.building_defs[c.0 as usize].id,
            building_id::make_id(0, 0, 1)
        );
        assert_eq!(
            *cell.iso_buildings.borrow(),
            [building_id::make_id(0, 0, 1)].into_iter().collect()
        );
        // Chunks still list b's rooms (the original leaves them).
        assert_eq!(chunk_rooms(&g, 1, 0).len(), 2);
    }
}

/// The frame `IsoMetaGrid.MetaGridLoaderThread.loadCell` logs from.
fn loader_thread_caller() -> StackElement {
    StackElement::new("zombie.iso.IsoMetaGrid$MetaGridLoaderThread", "loadCell")
}
