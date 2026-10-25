//! The parts of `IsoMetaGrid.save()` / `saveToBufferMap` after
//! `map_zone.bin`, and their loads in `IsoWorld.init`:
//!
//! * `map_animals.bin`: `saveAnimalZones` / `loadAnimalZones`, the animal
//!   zones (`animalZoneHandler`) and their junctions;
//! * `metagrid/metacell_<x>_<y>.bin`: `saveCells` / `loadCells` with
//!   `IsoMetaCell.save` / `load`, one byte per queued cell
//!   (`animalZonesGenerated`).
//!
//! `map_animals.bin` layout: `"ZONE"`, the world version, the string table
//! of `saveStringMap` (every zone's name, original name and type in
//! `HashSet` order), the zones (each `Zone.save(output, stringMap)` then
//! `action`, `animalType`, `spawnAnimal`, `spawnedAnimals`), then the
//! junctions (point indices and both zones' UUIDs).
//!
//! The zones are written in the order of `animalZoneHandler`'s
//! `HashMap<UUID, AnimalZone>` (reproduced from the UUID hashes and the
//! largest size the map has had). The junctions are a `HashSet` of objects
//! hashed by identity, so their order differs between runs of the
//! original; here they are written zone by zone in list order.

use std::io;
use std::path::Path;

use pz_kahlua::java_io::{JavaReader, JavaUuid, JavaWrite};

use crate::zone_save::{write_string_map, write_zone, UuidSource, MAGIC, WORLD_VERSION};
use crate::zones::{AnimalZoneData, AnimalZoneJunction};
use crate::{IsoMetaGrid, ZoneId, ZoneKind};

/// The `metagrid` folder of a save.
pub const METAGRID_DIR: &str = "metagrid";

/// `String.format("metacell_%d_%d.bin", x, y)`.
pub fn metacell_file_name(x: i32, y: i32) -> String {
    format!("metacell_{x}_{y}.bin")
}

/// `UUID.hashCode()`.
pub fn uuid_hash(u: JavaUuid) -> i32 {
    let hilo = u.msb ^ u.lsb;
    ((hilo >> 32) as i32) ^ (hilo as i32)
}

/// The iteration order of a `java.util.HashMap` whose keys have hashes
/// `hashes` (inserted in that order, distinct keys) and whose table grew
/// for `peak` entries at most: indices into `hashes`. Treeified bins are
/// not modelled.
pub fn java_hash_order_of(hashes: &[i32], peak: usize) -> Vec<usize> {
    let mut cap: usize = 16;
    while peak.max(hashes.len()) as f64 > cap as f64 * 0.75 {
        cap *= 2;
    }
    let mut idx: Vec<(usize, usize)> = hashes
        .iter()
        .enumerate()
        .map(|(i, &h)| {
            let spread = h ^ ((h as u32) >> 16) as i32;
            ((spread as u32 as usize) & (cap - 1), i)
        })
        .collect();
    idx.sort();
    idx.into_iter().map(|(_, i)| i).collect()
}

/// What [`IsoMetaGrid::load_animal_zones`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnimalZoneLoadReport {
    /// The file did not start with `"ZONE"`; nothing was changed.
    pub bad_magic: bool,
    pub world_version: i32,
    pub zones: usize,
    pub junctions: usize,
}

fn bad(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn animal(kind: &ZoneKind) -> Option<&AnimalZoneData> {
    match kind {
        ZoneKind::Animal(a) => Some(a),
        _ => None,
    }
}

impl IsoMetaGrid {
    /// `addCellToSave(cell)` for the cell at absolute `(x, y)`.
    pub fn add_cell_to_save(&self, x: i32, y: i32) {
        let mut cells = self.cells_to_save.borrow_mut();
        if !cells.contains(&(x, y)) {
            cells.push((x, y));
        }
    }

    /// `IsoMetaCell.checkAnimalZonesGenerated(chunkX, chunkY)` on the cell
    /// at absolute `(x, y)`: sets `animalZonesGenerated` and returns true
    /// the first time, when the caller runs
    /// `ZoneGenerator.genAnimalsPath(chunkX, chunkY)` (which queues the cell
    /// with [`IsoMetaGrid::add_cell_to_save`] unless the sandbox's animal
    /// path chance is "none"). False without a cell.
    pub fn check_animal_zones_generated(&self, x: i32, y: i32) -> bool {
        let Some(cell) = self.get_cell_data(x, y) else {
            return false;
        };
        if cell.animal_zones_generated.get() {
            return false;
        }
        cell.animal_zones_generated.set(true);
        true
    }

    /// `saveCellsToSaveBufferMap` / the bytes of `saveCells`: each queued
    /// cell's `metagrid/metacell_<x>_<y>.bin` (`IsoMetaCell.save`, one byte),
    /// leaving the queue empty. A queued position without a cell is skipped.
    pub fn save_cells(&self) -> Vec<(String, Vec<u8>)> {
        let cells = std::mem::take(&mut *self.cells_to_save.borrow_mut());
        cells
            .into_iter()
            .filter_map(|(x, y)| {
                let cell = self.get_cell_data(x, y)?;
                let path = format!("{METAGRID_DIR}/{}", metacell_file_name(x, y));
                Some((path, vec![u8::from(cell.animal_zones_generated.get())]))
            })
            .collect()
    }

    /// `saveCells("metagrid", ...)`: writes each queued cell's file under
    /// `save_dir` (the `metagrid` folder must exist, as
    /// `SavefileNaming.ensureSubdirectoriesExist` makes it at game load).
    /// The first failure ends it; the cells not written stay queued.
    pub fn save_cells_files(&self, save_dir: &Path) -> io::Result<()> {
        let cells: Vec<(i32, i32)> = self.cells_to_save.borrow().clone();
        for (x, y) in cells {
            if let Some(cell) = self.get_cell_data(x, y) {
                let path = save_dir.join(METAGRID_DIR).join(metacell_file_name(x, y));
                std::fs::write(path, [u8::from(cell.animal_zones_generated.get())])?;
            }
            self.cells_to_save.borrow_mut().retain(|&c| c != (x, y));
        }
        Ok(())
    }

    /// `IsoMetaCell.load(grid, input, -1)` for the cell at absolute
    /// `(x, y)` (created when missing, `getCellOrCreate`): the first byte is
    /// `animalZonesGenerated`.
    pub fn load_meta_cell(&mut self, x: i32, y: i32, data: &[u8]) -> io::Result<()> {
        let (gx, gy) = (x - self.min_x, y - self.min_y);
        if gx < 0 || gy < 0 || gx >= self.width || gy >= self.height {
            return Err(bad(format!(
                "java.lang.ArrayIndexOutOfBoundsException: metacell {x},{y} outside the grid"
            )));
        }
        let cell = self.get_cell_or_create(gx, gy);
        let b = *data
            .first()
            .ok_or_else(|| bad("java.nio.BufferUnderflowException".into()))?;
        cell.animal_zones_generated.set(b != 0);
        Ok(())
    }

    /// `loadCells("metagrid", "metacell_(-?[0-9]+)_(-?[0-9]+)\\.bin",
    /// IsoMetaCell::load)`: every matching file of `save_dir/metagrid`. A
    /// file that fails is logged and the next one loaded; a cell outside
    /// the grid or a number that does not fit an `int` ends the load (the
    /// original throws out of `loadCells`). Returns the cells loaded.
    pub fn load_cells_files(&mut self, save_dir: &Path) -> io::Result<Vec<(i32, i32)>> {
        let mut loaded = Vec::new();
        let dir = match std::fs::read_dir(save_dir.join(METAGRID_DIR)) {
            Ok(d) => d,
            Err(_) => return Ok(loaded),
        };
        for entry in dir {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some((x, y)) = parse_metacell_name(name)? else {
                continue;
            };
            let data = match std::fs::read(entry.path()) {
                Ok(d) => d,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => {
                    eprintln!("ERROR: IsoMetaGrid.loadCells: {e}");
                    continue;
                }
            };
            match self.load_meta_cell(x, y, &data) {
                Ok(()) => loaded.push((x, y)),
                Err(e) if e.to_string().contains("ArrayIndexOutOfBounds") => return Err(e),
                Err(e) => eprintln!("ERROR: IsoMetaGrid.loadCells: {e}"),
            }
        }
        Ok(loaded)
    }

    /// `saveAnimalZones(output)`. Zones without an id get a random one
    /// first (the original assigns it at construction).
    pub fn save_animal_zones(&mut self, ids: &mut dyn UuidSource) -> io::Result<Vec<u8>> {
        for &id in &self.animal_zones {
            let zone = &mut self.zone_defs[id.0 as usize];
            if zone.id.is_none() {
                zone.id = Some(ids.random_uuid());
            }
        }
        let hashes: Vec<i32> = self
            .animal_zones
            .iter()
            .map(|&id| uuid_hash(self.zone(id).id.expect("assigned above")))
            .collect();
        let order: Vec<ZoneId> = java_hash_order_of(&hashes, self.animal_zones_peak)
            .into_iter()
            .map(|i| self.animal_zones[i])
            .collect();
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.put_i32(WORLD_VERSION);
        let index = write_string_map(&mut out, order.iter().map(|&id| self.zone(id)))?;
        out.put_i32(order.len() as i32);
        for &id in &order {
            let z = self.zone(id);
            write_zone(&mut out, z, &index);
            let default = AnimalZoneData::default();
            let a = animal(&z.kind).unwrap_or(&default);
            out.write_string_utf(a.action.as_deref());
            out.write_string_utf(a.animal_type.as_deref());
            out.put_bool(a.spawn_animal);
            out.put_bool(a.spawned_animals);
        }
        let mut junctions = Vec::new();
        for &id in &order {
            if let Some(js) = animal(&self.zone(id).kind).and_then(|a| a.junctions.as_ref()) {
                for j in js {
                    junctions.push((id, *j));
                }
            }
        }
        out.put_i32(junctions.len() as i32);
        for (self_id, j) in junctions {
            let other = self.zone(j.zone_other).id.unwrap_or_default();
            out.put_i32(j.point_index_self);
            out.put_i32(j.point_index_other);
            out.write_uuid(self.zone(self_id).id.expect("assigned above"));
            out.write_uuid(other);
        }
        Ok(out)
    }

    /// `loadAnimalZones(input, worldVersion)`; `world_version == -1` reads
    /// the header. The animal zones there were are dropped (from the
    /// handler and every cell) and `cellsToSave` emptied, then the file's
    /// zones are registered without queueing their cells
    /// (`registerAnimalZone(zone, false)`), then the junctions are added
    /// to the zone named first. A junction naming an unknown zone ends the
    /// load (`NullPointerException`), keeping what was read.
    pub fn load_animal_zones(
        &mut self,
        data: &[u8],
        mut world_version: i32,
    ) -> io::Result<AnimalZoneLoadReport> {
        let mut r = JavaReader::new(data);
        let mut rep = AnimalZoneLoadReport::default();
        if world_version == -1 {
            if r.take(4)? != MAGIC {
                pz_core::debug::log("ERROR: expected 'ZONE' at start of map_animals.bin");
                rep.bad_magic = true;
                return Ok(rep);
            }
            world_version = r.get_i32()?;
        }
        rep.world_version = world_version;
        self.animal_zones.clear();
        self.cells_to_save.borrow_mut().clear();
        for cell in self.grid.iter_mut().flatten() {
            cell.animal_zones.clear();
        }
        let n = r.get_i32()?;
        let mut strings = Vec::new();
        for _ in 0..n.max(0) {
            strings.push(r.read_string_utf()?);
        }
        let count = r.get_i32()?;
        pz_core::debug::log(&format!("loading {count} zones from map_animals.bin"));
        for _ in 0..count.max(0) {
            let mut zone = self.read_zone(&mut r, world_version, &strings, &mut NoUuids)?;
            let data = AnimalZoneData {
                action: Some(r.read_string_utf()?),
                animal_type: Some(r.read_string_utf()?),
                spawn_animal: r.get_bool()?,
                spawned_animals: r.get_bool()?,
                junctions: None,
            };
            zone.kind = ZoneKind::Animal(data);
            self.register_animal_zone_save(zone, false);
            rep.zones += 1;
        }
        let n = r.get_i32()?;
        for _ in 0..n.max(0) {
            let point_index_self = r.get_i32()?;
            let point_index_other = r.get_i32()?;
            let self_id = r.read_uuid()?;
            let other_id = r.read_uuid()?;
            let find = |u: JavaUuid| {
                self.animal_zones
                    .iter()
                    .copied()
                    .find(|&id| self.zone(id).id == Some(u))
            };
            let (Some(zs), Some(zo)) = (find(self_id), find(other_id)) else {
                return Err(bad(
                    "java.lang.NullPointerException: AnimalZoneJunction zone not found".into(),
                ));
            };
            if let ZoneKind::Animal(a) = &mut self.zone_defs[zs.0 as usize].kind {
                a.junctions
                    .get_or_insert_with(Vec::new)
                    .push(AnimalZoneJunction {
                        point_index_self,
                        zone_other: zo,
                        point_index_other,
                    });
            }
            rep.junctions += 1;
        }
        Ok(rep)
    }

    /// `load("map_animals.bin", this::loadAnimalZones)`: a missing file
    /// changes nothing.
    pub fn load_animal_zones_file(
        &mut self,
        save_dir: &Path,
    ) -> io::Result<Option<AnimalZoneLoadReport>> {
        match std::fs::read(save_dir.join("map_animals.bin")) {
            Ok(data) => self.load_animal_zones(&data, -1).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes `map_animals.bin` in `save_dir`.
    pub fn save_animal_zones_file(
        &mut self,
        save_dir: &Path,
        ids: &mut dyn UuidSource,
    ) -> io::Result<()> {
        let bytes = self.save_animal_zones(ids)?;
        std::fs::write(save_dir.join("map_animals.bin"), bytes)
    }
}

/// Animal zones are only saved at world version 249, which carries every
/// zone's UUID.
struct NoUuids;

impl UuidSource for NoUuids {
    fn random_uuid(&mut self) -> JavaUuid {
        JavaUuid::default()
    }
}

/// `metacell_(-?[0-9]+)_(-?[0-9]+)\.bin` matched against the whole name:
/// the two numbers (`Integer.parseInt`; one that does not fit is an
/// error, as the original's `NumberFormatException`).
fn parse_metacell_name(name: &str) -> io::Result<Option<(i32, i32)>> {
    let Some(rest) = name
        .strip_prefix("metacell_")
        .and_then(|r| r.strip_suffix(".bin"))
    else {
        return Ok(None);
    };
    let num = |s: &str| {
        let digits = s.strip_prefix('-').unwrap_or(s);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    };
    // The regex's first group is greedy over `-?[0-9]+`, which cannot
    // contain `_`: the split is at the only `_` that leaves two numbers.
    let Some((a, b)) = rest.split_once('_') else {
        return Ok(None);
    };
    if !num(a) || !num(b) {
        return Ok(None);
    }
    let parse = |s: &str| {
        s.parse::<i32>().map_err(|_| {
            bad(format!(
                "java.lang.NumberFormatException: For input string: \"{s}\""
            ))
        })
    };
    Ok(Some((parse(a)?, parse(b)?)))
}

#[cfg(test)]
mod tests;
