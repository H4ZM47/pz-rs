//! `IsoMetaGrid`, `IsoMetaCell`, `IsoMetaChunk` and the lot header reader
//! (`IsoMetaGrid.MetaGridLoaderThread.loadCell`).

use std::collections::HashMap;
use std::io;

use pz_fs::GameFs;

use crate::bra::BufferedRandomAccessFile;
use crate::defs::{
    building_id, room_id, BuildingDef, BuildingIdx, MetaObject, RoomDef, RoomIdx, RoomRect,
};
use crate::java::{coordmodulo, fastfloor, java_trim, GameRand};
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
    /// Keys of `isoBuildings`: buildings whose runtime `IsoBuilding` has been
    /// created (by [`IsoMetaGrid::get_room_by_id`]).
    pub iso_buildings: std::cell::RefCell<std::collections::HashSet<i64>>,
    pub vehicle_zones: Vec<ZoneId>,
    pub mannequin_zones: Vec<ZoneId>,
    /// Lazily created upstream (`null` until the first world-gen zone).
    pub world_gen_zones: Option<Vec<ZoneId>>,
    pub animal_zones: Vec<ZoneId>,
    pub room_tones: Vec<RoomTone>,
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
            iso_buildings: Default::default(),
            vehicle_zones: Vec::new(),
            mannequin_zones: Vec::new(),
            world_gen_zones: None,
            animal_zones: Vec::new(),
            room_tones: Vec::new(),
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

    fn get_cell_data_mut(&mut self, x: i32, y: i32) -> Option<&mut IsoMetaCell> {
        let (gx, gy) = (x - self.min_x, y - self.min_y);
        if gx < 0 || gy < 0 || gx >= self.width || gy >= self.height {
            return None;
        }
        self.cell_slot(gx, gy).as_mut()
    }

    /// `IsoMetaCell.getChunk(i)`: creates the chunk on first use, seeding its
    /// zombie intensity from the lot headers.
    pub(crate) fn meta_chunk_mut(
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

    pub fn is_valid_square(&self, x: i32, y: i32) -> bool {
        x >= self.min_x * 256
            && x < (self.max_x + 1) * 256
            && y >= self.min_y * 256
            && y < (self.max_y + 1) * 256
    }

    /// `IsoMetaCell.addRoom(room, cellX*256, cellY*256)`.
    fn add_room_to_cell(&mut self, cell_x: i32, cell_y: i32, room: RoomIdx) {
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
    /// consolidation and adjacent-cell flags. The building loop that follows
    /// (adjacent-cell rooms, alarms) is [`IsoMetaGrid::assign_alarms`].
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
            eprintln!("ERROR loading {}: {e}", path.display());
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
                eprintln!("duplicate RoomDef.metaID for room at x={rx}, y={ry}, level={level}, filename={}", path.file_name().unwrap_or_default().to_string_lossy());
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

    /// The building loop of `CreateStep2`: rooms are added to adjacent
    /// cells, then alarms are assigned from the sandbox `alarm` option.
    pub fn assign_alarms(
        &mut self,
        rand: &mut dyn GameRand,
        last_stand: bool,
        power_grid_exists: bool,
        alarm_option: i32,
        alarm_decay: impl Fn() -> i32,
    ) {
        for i in 0..self.buildings.len() {
            let bidx = self.buildings[i];
            self.add_rooms_to_adjacent_cells(bidx);
            let b = bidx.0 as usize;
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
