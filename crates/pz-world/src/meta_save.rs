//! `map_meta.bin`: `IsoMetaGrid.save`/`savePart`/`load(ByteBuffer)`.
//!
//! Part 0 is `"META"`, the world version, the grid's cell bounds and, for
//! every grid cell (x-major), the persistent state of its rooms (meta id
//! and four flag bits) and buildings (meta id, alarm, key, seen/visited,
//! loot respawn hour, alarm decay). Only cells with map data contribute
//! rooms and buildings, and user-defined ones are skipped.
//!
//! Part 1 is the world-wide state saved alongside: safehouses, non-PvP
//! zones, factions, designation (animal) zones, the stash system and the
//! names of unique randomized-building stories already spawned. These are
//! ported as plain data ([`MetaWorld`]) since the systems themselves are
//! not.
//!
//! The file is split into a raw form ([`MetaCells`], [`MetaWorld`]) that
//! reads and writes exactly what is there, and the grid side
//! ([`IsoMetaGrid::apply_meta_cells`] / [`IsoMetaGrid::meta_cells`]) that
//! matches records to the loaded map by meta id the way the original does.

use crate::IsoMetaGrid;
use pz_kahlua::java_io::{JavaReader, JavaWrite};
use std::io;
use std::path::Path;

/// `IsoWorld.getWorldVersion()` in this build.
pub const WORLD_VERSION: i32 = 249;
const MAGIC: [u8; 4] = *b"META";

/// `GameServer.server` / `GameClient.client`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetRole {
    #[default]
    SinglePlayer,
    Server,
    Client,
}

/// One room of a cell (`RoomDef` persistent state).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoomRecord {
    pub meta_id: i64,
    pub explored: bool,
    pub lights_active: bool,
    pub done_spawn: bool,
    pub roof_fixed: bool,
}

impl RoomRecord {
    fn flags(&self) -> i16 {
        (self.explored as i16)
            | (self.lights_active as i16) << 1
            | (self.done_spawn as i16) << 2
            | (self.roof_fixed as i16) << 3
    }
}

/// One building of a cell (`BuildingDef` persistent state).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildingRecord {
    pub meta_id: i64,
    pub alarmed: bool,
    pub key_id: i32,
    pub seen: bool,
    pub has_been_visited: bool,
    pub loot_respawn_hour: i32,
    /// Absent before world version 201 (reads as 0).
    pub alarm_decay: i32,
}

/// Part 0 of `map_meta.bin` as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetaCells {
    pub world_version: i32,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    /// `(rooms, buildings)` per cell, x-major: index
    /// `(x - min_x) * height + (y - min_y)`.
    pub cells: Vec<(Vec<RoomRecord>, Vec<BuildingRecord>)>,
}

impl MetaCells {
    /// Reads part 0 (the magic bytes are not checked, as in the original).
    pub fn read(r: &mut JavaReader<'_>) -> io::Result<MetaCells> {
        r.take(4)?;
        let world_version = r.get_i32()?;
        let (min_x, min_y, max_x, max_y) = (r.get_i32()?, r.get_i32()?, r.get_i32()?, r.get_i32()?);
        let mut cells = Vec::new();
        for _x in min_x..=max_x {
            for _y in min_y..=max_y {
                let n = r.get_i32()?;
                let mut rooms = Vec::new();
                for _ in 0..n.max(0) {
                    let meta_id = r.get_i64()?;
                    let flags = r.get_i16()?;
                    rooms.push(RoomRecord {
                        meta_id,
                        explored: flags & 1 != 0,
                        lights_active: flags & 2 != 0,
                        done_spawn: flags & 4 != 0,
                        roof_fixed: flags & 8 != 0,
                    });
                }
                let n = r.get_i32()?;
                let mut buildings = Vec::new();
                for _ in 0..n.max(0) {
                    buildings.push(BuildingRecord {
                        meta_id: r.get_i64()?,
                        alarmed: r.get_i8()? != 0,
                        key_id: r.get_i32()?,
                        seen: r.get_i8()? != 0,
                        has_been_visited: r.get_i8()? != 0,
                        loot_respawn_hour: r.get_i32()?,
                        alarm_decay: if world_version >= 201 {
                            r.get_i32()?
                        } else {
                            0
                        },
                    });
                }
                cells.push((rooms, buildings));
            }
        }
        Ok(MetaCells {
            world_version,
            min_x,
            min_y,
            max_x,
            max_y,
            cells,
        })
    }

    /// `savePart(output, 0, …)`: always the current version.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&MAGIC);
        out.put_i32(WORLD_VERSION);
        out.put_i32(self.min_x);
        out.put_i32(self.min_y);
        out.put_i32(self.max_x);
        out.put_i32(self.max_y);
        for (rooms, buildings) in &self.cells {
            out.put_i32(rooms.len() as i32);
            for r in rooms {
                out.put_i64(r.meta_id);
                out.put_i16(r.flags());
            }
            out.put_i32(buildings.len() as i32);
            for b in buildings {
                out.put_i64(b.meta_id);
                out.put_bool(b.alarmed);
                out.put_i32(b.key_id);
                out.put_bool(b.seen);
                out.put_bool(b.has_been_visited);
                out.put_i32(b.loot_respawn_hour);
                out.put_i32(b.alarm_decay);
            }
        }
    }
}

/// What applying [`MetaCells`] to a grid could not match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetaApplyReport {
    /// The file's grid size differs from the map's (only logged upstream).
    pub size_mismatch: bool,
    /// `(cellX, cellY, metaID)` of records whose room / building is not in
    /// the map ("invalid room metaID").
    pub unknown_rooms: Vec<(i32, i32, i64)>,
    pub unknown_buildings: Vec<(i32, i32, i64)>,
    pub total_buildings: usize,
    pub total_alarmed: usize,
}

impl IsoMetaGrid {
    /// The grid half of `load(ByteBuffer)`: copies each record onto the
    /// room / building with that meta id in that cell. Records for cells
    /// outside the grid or without map data are dropped silently; unknown
    /// meta ids are reported.
    pub fn apply_meta_cells(&mut self, m: &MetaCells) -> MetaApplyReport {
        let mut rep = MetaApplyReport {
            size_mismatch: m.max_x - m.min_x + 1 != self.width
                || m.max_y - m.min_y + 1 != self.height,
            ..MetaApplyReport::default()
        };
        let mut i = 0;
        for x in m.min_x..=m.max_x {
            for y in m.min_y..=m.max_y {
                let (rooms, buildings) = &m.cells[i];
                i += 1;
                rep.total_buildings += buildings.len();
                let Some(cell) = self.get_cell_data(x, y) else {
                    continue;
                };
                if cell.info.is_none() {
                    continue;
                }
                let room_ids: Vec<_> = rooms
                    .iter()
                    .map(|r| (r, cell.room_by_meta_id.get(&r.meta_id).copied()))
                    .collect();
                let building_ids: Vec<_> = buildings
                    .iter()
                    .map(|b| (b, cell.building_by_meta_id.get(&b.meta_id).copied()))
                    .collect();
                for (r, idx) in room_ids {
                    match idx {
                        Some(idx) => {
                            let def = &mut self.room_defs[idx.0 as usize];
                            def.explored = r.explored;
                            def.lights_active = r.lights_active;
                            def.done_spawn = r.done_spawn;
                            def.roof_fixed = r.roof_fixed;
                        }
                        None => rep.unknown_rooms.push((x, y, r.meta_id)),
                    }
                }
                for (b, idx) in building_ids {
                    match idx {
                        Some(idx) => {
                            rep.total_alarmed += b.alarmed as usize;
                            let def = &mut self.building_defs[idx.0 as usize];
                            def.alarmed = b.alarmed;
                            def.key_id = b.key_id;
                            def.seen = b.seen;
                            def.has_been_visited = b.has_been_visited;
                            def.loot_respawn_hour = b.loot_respawn_hour;
                            def.alarm_decay = b.alarm_decay;
                        }
                        None => rep.unknown_buildings.push((x, y, b.meta_id)),
                    }
                }
            }
        }
        rep
    }

    /// The grid half of `savePart(output, 0, …)`.
    pub fn meta_cells(&self) -> MetaCells {
        let mut cells = Vec::with_capacity((self.width * self.height) as usize);
        for x in 0..self.width {
            for y in 0..self.height {
                let mut rooms = Vec::new();
                let mut buildings = Vec::new();
                if let Some(cell) = self.get_cell(x, y).filter(|c| c.info.is_some()) {
                    for idx in &cell.room_list {
                        let r = &self.room_defs[idx.0 as usize];
                        if r.user_defined {
                            continue;
                        }
                        rooms.push(RoomRecord {
                            meta_id: r.meta_id,
                            explored: r.explored,
                            lights_active: r.lights_active,
                            done_spawn: r.done_spawn,
                            roof_fixed: r.roof_fixed,
                        });
                    }
                    for idx in &cell.buildings {
                        let b = &self.building_defs[idx.0 as usize];
                        if b.user_defined {
                            continue;
                        }
                        buildings.push(BuildingRecord {
                            meta_id: b.meta_id,
                            alarmed: b.alarmed,
                            key_id: b.key_id,
                            seen: b.seen,
                            has_been_visited: b.has_been_visited,
                            loot_respawn_hour: b.loot_respawn_hour,
                            alarm_decay: b.alarm_decay,
                        });
                    }
                }
                cells.push((rooms, buildings));
            }
        }
        MetaCells {
            world_version: WORLD_VERSION,
            min_x: self.min_x,
            min_y: self.min_y,
            max_x: self.max_x,
            max_y: self.max_y,
            cells,
        }
    }
}

/// `SafeHouse` (saved fields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafeHouse {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub owner: String,
    pub hit_points: i32,
    pub players: Vec<String>,
    pub last_visited: i64,
    pub title: String,
    pub datetime_created: i64,
    pub location: Option<String>,
    pub players_respawn: Vec<String>,
}

impl SafeHouse {
    /// `new SafeHouse(x, y, w, h, player)`: the owner is also the first
    /// player. (The original's id and online id are not saved.)
    pub fn new(x: i32, y: i32, w: i32, h: i32, owner: &str) -> SafeHouse {
        SafeHouse {
            x,
            y,
            w,
            h,
            owner: owner.to_string(),
            hit_points: 0,
            players: vec![owner.to_string()],
            last_visited: 0,
            title: "Safehouse".to_string(),
            datetime_created: 0,
            location: None,
            players_respawn: Vec::new(),
        }
    }

    /// `addPlayer(player)`: no duplicates.
    pub fn add_player(&mut self, player: &str) {
        if !self.players.iter().any(|p| p == player) {
            self.players.push(player.to_string());
        }
    }

    fn save(&self, out: &mut Vec<u8>) {
        out.put_i32(self.x);
        out.put_i32(self.y);
        out.put_i32(self.w);
        out.put_i32(self.h);
        out.write_string_utf(Some(&self.owner));
        out.put_i32(self.hit_points);
        out.put_i32(self.players.len() as i32);
        for p in &self.players {
            out.write_string_utf(Some(p));
        }
        out.put_i64(self.last_visited);
        out.write_string_utf(Some(&self.title));
        out.put_i64(self.datetime_created);
        out.write_string_utf(self.location.as_deref());
        out.put_i32(self.players_respawn.len() as i32);
        for p in &self.players_respawn {
            out.write_string_utf(Some(p));
        }
    }

    fn load(r: &mut JavaReader<'_>, wv: i32, env: &mut dyn MetaLoadEnv) -> io::Result<SafeHouse> {
        let (x, y, w, h) = (r.get_i32()?, r.get_i32()?, r.get_i32()?, r.get_i32()?);
        let mut s = SafeHouse::new(x, y, w, h, &r.read_string_utf()?);
        if wv >= 216 {
            s.hit_points = r.get_i32()?;
        }
        for _ in 0..r.get_i32()?.max(0) {
            let p = r.read_string_utf()?;
            s.add_player(&p);
        }
        s.last_visited = r.get_i64()?;
        s.title = r.read_string_utf()?;
        if wv >= 223 {
            // `setDatetimeCreated`: 0 means "now".
            let created = r.get_i64()?;
            s.datetime_created = if created == 0 {
                env.now_millis()
            } else {
                created
            };
            // `setLocation`: an empty name is replaced by the nearest
            // spawn region's (or stays null).
            let loc = r.read_string_utf()?;
            s.location = if loc.is_empty() {
                env.nearest_spawn_region(x, y)
            } else {
                Some(loc)
            };
        }
        for _ in 0..r.get_i32()?.max(0) {
            s.players_respawn.push(r.read_string_utf()?);
        }
        Ok(s)
    }
}

/// `NonPvpZone` (saved fields).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NonPvpZone {
    pub x: i32,
    pub y: i32,
    pub x2: i32,
    pub y2: i32,
    pub size: i32,
    pub title: String,
}

/// `Faction` (saved fields). `tag` carries the tag colour's RGB; a faction
/// without a tag gets a random colour on load in the original, which is
/// never saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Faction {
    pub name: String,
    pub owner: String,
    pub tag: Option<(String, [f32; 3])>,
    pub players: Vec<String>,
}

/// `DesignationZone` (saved fields). Only `"AnimalZone"` zones
/// (`DesignationZoneAnimal`) survive loading.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignationZone {
    pub id: f64,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    pub h: i32,
    pub zone_type: String,
    pub name: String,
    pub hour_last_seen: i32,
}

/// `StashSystem.StashBuilding`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StashBuilding {
    pub stash_name: String,
    pub building_x: i32,
    pub building_y: i32,
}

/// `StashSystem`'s saved lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StashState {
    pub possible_stashes: Vec<StashBuilding>,
    pub buildings_to_do: Vec<StashBuilding>,
    pub already_read_map: Vec<String>,
}

/// Part 1 of `map_meta.bin`: the world-wide state saved with the grid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetaWorld {
    /// `SafeHouse.getSafehouseList()`.
    pub safehouses: Vec<SafeHouse>,
    /// `NonPvpZone.getAllZones()`.
    pub non_pvp_zones: Vec<NonPvpZone>,
    /// `Faction.getFactions()`.
    pub factions: Vec<Faction>,
    /// `DesignationZone.allZones`. Not cleared by a load: loaded zones are
    /// merged by id.
    pub designation_zones: Vec<DesignationZone>,
    /// `None` while `StashSystem.allStashes` is null, in which case nothing
    /// is written for it. Loading always initialises it.
    pub stash: Option<StashState>,
    /// `RBBasic.getUniqueRDSSpawned()`.
    pub unique_rds_spawned: Vec<String>,
}

/// Inputs part-1 loading takes from other systems.
pub trait MetaLoadEnv {
    /// `Calendar.getInstance().getTimeInMillis()`.
    fn now_millis(&mut self) -> i64;
    /// `SafeHouse.setLocation(null/"")`: the name of the spawn region whose
    /// map's zoom point is nearest to `(x, y)` (within 100000), if any.
    fn nearest_spawn_region(&mut self, x: i32, y: i32) -> Option<String>;
}

/// A [`MetaLoadEnv`] with the system clock and no spawn regions.
pub struct DefaultMetaEnv;

impl MetaLoadEnv for DefaultMetaEnv {
    fn now_millis(&mut self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
    fn nearest_spawn_region(&mut self, _x: i32, _y: i32) -> Option<String> {
        None
    }
}

fn write_stash_buildings(out: &mut Vec<u8>, v: &[StashBuilding]) {
    out.put_i32(v.len() as i32);
    for s in v {
        out.write_string_utf(Some(&s.stash_name));
        out.put_i32(s.building_x);
        out.put_i32(s.building_y);
    }
}

fn read_stash_buildings(r: &mut JavaReader<'_>) -> io::Result<Vec<StashBuilding>> {
    let mut v = Vec::new();
    for _ in 0..r.get_i32()?.max(0) {
        v.push(StashBuilding {
            stash_name: r.read_string_utf()?,
            building_x: r.get_i32()?,
            building_y: r.get_i32()?,
        });
    }
    Ok(v)
}

impl StashState {
    fn save(&self, out: &mut Vec<u8>) {
        write_stash_buildings(out, &self.possible_stashes);
        write_stash_buildings(out, &self.buildings_to_do);
        out.put_i32(self.already_read_map.len() as i32);
        for s in &self.already_read_map {
            out.write_string_utf(Some(s));
        }
    }

    fn load(r: &mut JavaReader<'_>) -> io::Result<StashState> {
        let possible_stashes = read_stash_buildings(r)?;
        let buildings_to_do = read_stash_buildings(r)?;
        let mut already_read_map = Vec::new();
        for _ in 0..r.get_i32()?.max(0) {
            already_read_map.push(r.read_string_utf()?);
        }
        Ok(StashState {
            possible_stashes,
            buildings_to_do,
            already_read_map,
        })
    }
}

impl MetaWorld {
    /// `savePart(output, 1, …)`. `out` must hold the file from its start:
    /// the server form records the absolute end of the stash data.
    pub fn save(&self, out: &mut Vec<u8>, role: NetRole) {
        out.put_i32(self.safehouses.len() as i32);
        for s in &self.safehouses {
            s.save(out);
        }
        out.put_i32(self.non_pvp_zones.len() as i32);
        for z in &self.non_pvp_zones {
            out.put_i32(z.x);
            out.put_i32(z.y);
            out.put_i32(z.x2);
            out.put_i32(z.y2);
            out.put_i32(z.size);
            out.write_string_utf(Some(&z.title));
        }
        out.put_i32(self.factions.len() as i32);
        for f in &self.factions {
            out.write_string_utf(Some(&f.name));
            out.write_string_utf(Some(&f.owner));
            out.put_i32(f.players.len() as i32);
            match &f.tag {
                Some((tag, rgb)) => {
                    out.put_i8(1);
                    out.write_string_utf(Some(tag));
                    for v in rgb {
                        out.put_f32(*v);
                    }
                }
                None => out.put_i8(0),
            }
            for p in &f.players {
                out.write_string_utf(Some(p));
            }
        }
        out.put_i32(self.designation_zones.len() as i32);
        for z in &self.designation_zones {
            out.put_f64(z.id);
            out.put_i32(z.x);
            out.put_i32(z.y);
            out.put_i32(z.z);
            out.put_i32(z.h);
            out.put_i32(z.w);
            out.write_string_utf(Some(&z.zone_type));
            out.write_string_utf(Some(&z.name));
            out.put_i32(z.hour_last_seen);
        }
        match role {
            NetRole::Server => {
                let at = out.len();
                out.put_i32(0);
                if let Some(s) = &self.stash {
                    s.save(out);
                }
                let end = (out.len() as i32).to_be_bytes();
                out[at..at + 4].copy_from_slice(&end);
            }
            NetRole::SinglePlayer => {
                if let Some(s) = &self.stash {
                    s.save(out);
                }
            }
            NetRole::Client => {}
        }
        out.put_i32(self.unique_rds_spawned.len() as i32);
        for s in &self.unique_rds_spawned {
            out.write_string_utf(Some(s));
        }
    }

    /// The part-1 half of `load(ByteBuffer)`. Safehouses, non-PvP zones,
    /// factions and the unique-story list are replaced; designation zones
    /// are merged.
    pub fn load(
        &mut self,
        r: &mut JavaReader<'_>,
        wv: i32,
        role: NetRole,
        env: &mut dyn MetaLoadEnv,
    ) -> io::Result<()> {
        self.safehouses.clear();
        for _ in 0..r.get_i32()?.max(0) {
            let s = SafeHouse::load(r, wv, env)?;
            self.safehouses.push(s);
        }
        self.non_pvp_zones.clear();
        for _ in 0..r.get_i32()?.max(0) {
            self.non_pvp_zones.push(NonPvpZone {
                x: r.get_i32()?,
                y: r.get_i32()?,
                x2: r.get_i32()?,
                y2: r.get_i32()?,
                size: r.get_i32()?,
                title: r.read_string_utf()?,
            });
        }
        self.factions.clear();
        for _ in 0..r.get_i32()?.max(0) {
            let name = r.read_string_utf()?;
            let owner = r.read_string_utf()?;
            let n = r.get_i32()?;
            let tag = if r.get_i8()? != 0 {
                let tag = r.read_string_utf()?;
                // `setTagColor` raises each channel to at least 0.19.
                let mut rgb = [r.get_f32()?, r.get_f32()?, r.get_f32()?];
                for v in &mut rgb {
                    if *v < 0.19 {
                        *v = 0.19;
                    }
                }
                Some((tag, rgb))
            } else {
                None
            };
            let mut players = Vec::new();
            for _ in 0..n.max(0) {
                players.push(r.read_string_utf()?);
            }
            self.factions.push(Faction {
                name,
                owner,
                tag,
                players,
            });
        }
        for _ in 0..r.get_i32()?.max(0) {
            self.load_designation_zone(r)?;
        }
        match role {
            NetRole::Server => {
                r.get_i32()?;
                self.stash = Some(StashState::load(r)?);
            }
            NetRole::Client => {
                let pos = r.get_i32()?;
                r.set_position(pos.max(0) as usize)?;
            }
            NetRole::SinglePlayer => self.stash = Some(StashState::load(r)?),
        }
        self.unique_rds_spawned.clear();
        for _ in 0..r.get_i32()?.max(0) {
            self.unique_rds_spawned.push(r.read_string_utf()?);
        }
        Ok(())
    }

    /// `DesignationZone.load`.
    fn load_designation_zone(&mut self, r: &mut JavaReader<'_>) -> io::Result<()> {
        let id = r.get_f64()?;
        let (x, y, z, h, w) = (
            r.get_i32()?,
            r.get_i32()?,
            r.get_i32()?,
            r.get_i32()?,
            r.get_i32()?,
        );
        let zone_type = r.read_string_utf()?;
        let name = r.read_string_utf()?;
        let hour_last_seen = r.get_i32()?;
        if zone_type != "AnimalZone" {
            return Ok(());
        }
        // `Double.equals`: bitwise, NaN equal to NaN.
        let same = |a: f64| a.to_bits() == id.to_bits() || (a.is_nan() && id.is_nan());
        if let Some(zone) = self
            .designation_zones
            .iter_mut()
            .find(|d| d.zone_type == "AnimalZone" && same(d.id))
        {
            zone.name = name;
            zone.w = w;
            zone.h = h;
            zone.hour_last_seen = hour_last_seen;
            return Ok(());
        }
        // `new DesignationZoneAnimal(name, x, y, z, x + w, y + h, false)`:
        // corners are put in order. (The constructor's random id could
        // collide with an existing zone and drop the new one; that is not
        // modelled.)
        let (mut x1, mut x2) = (x, x.wrapping_add(w));
        let (mut y1, mut y2) = (y, y.wrapping_add(h));
        if x1 > x2 {
            std::mem::swap(&mut x1, &mut x2);
        }
        if y1 > y2 {
            std::mem::swap(&mut y1, &mut y2);
        }
        self.designation_zones.push(DesignationZone {
            id,
            x: x1,
            y: y1,
            z,
            w: x2.wrapping_sub(x1),
            h: y2.wrapping_sub(y1),
            zone_type,
            name,
            hour_last_seen,
        });
        Ok(())
    }
}

/// `IsoMetaGrid.save(ByteBuffer)`: the whole of `map_meta.bin`.
pub fn save_map_meta(grid: &IsoMetaGrid, world: &MetaWorld, role: NetRole) -> Vec<u8> {
    let mut out = Vec::new();
    grid.meta_cells().write(&mut out);
    world.save(&mut out, role);
    out
}

/// `IsoMetaGrid.load(ByteBuffer)`. Part 0 is applied once fully read; part 1
/// item by item, so on an error the earlier items stay loaded.
pub fn load_map_meta(
    grid: &mut IsoMetaGrid,
    world: &mut MetaWorld,
    data: &[u8],
    role: NetRole,
    env: &mut dyn MetaLoadEnv,
) -> io::Result<MetaApplyReport> {
    let mut r = JavaReader::new(data);
    let cells = MetaCells::read(&mut r)?;
    let report = grid.apply_meta_cells(&cells);
    world.load(&mut r, cells.world_version, role, env)?;
    Ok(report)
}

/// `IsoMetaGrid.load()` from a save folder: a missing file changes nothing.
pub fn load_map_meta_file(
    grid: &mut IsoMetaGrid,
    world: &mut MetaWorld,
    save_dir: &Path,
    env: &mut dyn MetaLoadEnv,
) -> io::Result<Option<MetaApplyReport>> {
    match std::fs::read(save_dir.join("map_meta.bin")) {
        Ok(data) => load_map_meta(grid, world, &data, NetRole::SinglePlayer, env).map(Some),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Writes `map_meta.bin` in `save_dir`.
pub fn save_map_meta_file(
    grid: &IsoMetaGrid,
    world: &MetaWorld,
    save_dir: &Path,
) -> io::Result<()> {
    std::fs::write(
        save_dir.join("map_meta.bin"),
        save_map_meta(grid, world, NetRole::SinglePlayer),
    )
}

/// The raw file: both parts as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct MapMetaFile {
    pub cells: MetaCells,
    pub world: MetaWorld,
}

impl MapMetaFile {
    /// Parses a whole file (part 1 loaded into an empty [`MetaWorld`]).
    pub fn parse(data: &[u8], role: NetRole, env: &mut dyn MetaLoadEnv) -> io::Result<MapMetaFile> {
        let mut r = JavaReader::new(data);
        let cells = MetaCells::read(&mut r)?;
        let mut world = MetaWorld::default();
        world.load(&mut r, cells.world_version, role, env)?;
        Ok(MapMetaFile { cells, world })
    }

    pub fn to_bytes(&self, role: NetRole) -> Vec<u8> {
        let mut out = Vec::new();
        self.cells.write(&mut out);
        self.world.save(&mut out, role);
        out
    }
}

#[cfg(test)]
mod tests;
