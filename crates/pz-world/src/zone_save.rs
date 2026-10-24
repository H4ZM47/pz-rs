//! `map_zone.bin`: `IsoMetaGrid.saveZone` / `loadZone` and
//! `Zone.save`/`load(ByteBuffer, int, Map, SharedStrings)`.
//!
//! Layout: `"ZONE"`, the world version, a string table (count, then
//! `GameWindow` strings), the zones (each naming its name, type and
//! original name by 16-bit table index, then its bounds, geometry, timers
//! and UUID), and `IsoWorld.spawnedZombieZone` (zone name to UUID list).
//!
//! The string table is the original's `HashSet` of every zone's name,
//! original name and type, written in `java.util.HashSet` iteration order;
//! `spawnedZombieZone` is a `HashMap` written in its key order. Both orders
//! are reproduced (see [`java_hash_order`]). World-gen zones are dropped on
//! load, as upstream.

use crate::java::{java_equals_ignore_case, string_hash};
use crate::{IsoMetaGrid, Zone, ZoneGeometryType, ZoneKind};
use pz_kahlua::java_io::{JavaReader, JavaUuid, JavaWrite};
use std::io;
use std::path::Path;

/// `IsoWorld.getWorldVersion()` in this build.
pub const WORLD_VERSION: i32 = 249;
const MAGIC: [u8; 4] = *b"ZONE";

/// `UUID.randomUUID()`.
pub trait UuidSource {
    fn random_uuid(&mut self) -> JavaUuid;
}

/// Version-4 UUIDs from the OS clock and a counter mixed through
/// SplitMix64 (the original uses `SecureRandom`; values are not
/// reproducible either way).
pub struct ClockUuids(u64);

impl Default for ClockUuids {
    fn default() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        ClockUuids(t ^ (std::process::id() as u64) << 32)
    }
}

impl UuidSource for ClockUuids {
    fn random_uuid(&mut self) -> JavaUuid {
        let mut next = || {
            self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^ (z >> 31)
        };
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&next().to_be_bytes());
        b[8..].copy_from_slice(&next().to_be_bytes());
        JavaUuid::from_random_bytes(b)
    }
}

/// `IsoWorld.spawnedZombieZone` (`HashMap<String, ArrayList<UUID>>`), kept
/// in insertion order; saved in the `HashMap`'s order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpawnedZombieZones {
    pub entries: Vec<(String, Vec<JavaUuid>)>,
}

impl SpawnedZombieZones {
    /// `put(name, ids)`: replaces an existing entry in place.
    pub fn put(&mut self, name: String, ids: Vec<JavaUuid>) {
        match self.entries.iter_mut().find(|(k, _)| *k == name) {
            Some(e) => e.1 = ids,
            None => self.entries.push((name, ids)),
        }
    }
}

/// Iteration order of a `java.util.HashSet<String>` / `HashMap` key set
/// (default capacity and load factor, never shrunk) after adding `keys` in
/// order; duplicates keep their first position, `None` is `null` (hash 0).
///
/// Buckets are visited by index and keep insertion order within a bucket
/// (resizes split buckets order-preservingly). Treeified bins (more than 8
/// keys in one bucket of a table of 64 or more) are not modelled.
pub fn java_hash_order<'a>(keys: &[Option<&'a str>]) -> Vec<Option<&'a str>> {
    let mut unique: Vec<Option<&str>> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for k in keys {
        if seen.insert(*k) {
            unique.push(*k);
        }
    }
    let mut cap: usize = 16;
    while unique.len() as f64 > cap as f64 * 0.75 {
        cap *= 2;
    }
    let mut indexed: Vec<(usize, usize, Option<&str>)> = unique
        .into_iter()
        .enumerate()
        .map(|(i, k)| {
            let h = k.map_or(0, string_hash);
            let spread = h ^ ((h as u32) >> 16) as i32;
            ((spread as u32 as usize) & (cap - 1), i, k)
        })
        .collect();
    indexed.sort_by_key(|&(bucket, i, _)| (bucket, i));
    indexed.into_iter().map(|(_, _, k)| k).collect()
}

/// What [`IsoMetaGrid::load_zones`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZoneLoadReport {
    /// The file did not start with `"ZONE"`; nothing was changed.
    pub bad_magic: bool,
    pub world_version: i32,
    /// Zones read, world-gen zones skipped, and zones refused as suspicious.
    pub read: usize,
    pub skipped_world_gen: usize,
    pub suspicious: usize,
}

fn bad(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

impl IsoMetaGrid {
    /// `loadZone(input, worldVersion)`; `world_version == -1` reads the
    /// header. Replaces all zones (not vehicle zones); spawned-zombie zone
    /// entries are added to `spawned`.
    pub fn load_zones(
        &mut self,
        data: &[u8],
        mut world_version: i32,
        spawned: &mut SpawnedZombieZones,
        ids: &mut dyn UuidSource,
    ) -> io::Result<ZoneLoadReport> {
        let mut r = JavaReader::new(data);
        let mut rep = ZoneLoadReport::default();
        if world_version == -1 {
            if r.take(4)? != MAGIC {
                eprintln!("ERROR: expected 'ZONE' at start of map_zone.bin");
                rep.bad_magic = true;
                return Ok(rep);
            }
            world_version = r.get_i32()?;
        }
        rep.world_version = world_version;
        self.clear_zones_for_load();
        let n = r.get_i32()?;
        let mut strings = Vec::new();
        for _ in 0..n.max(0) {
            strings.push(r.read_string_utf()?);
        }
        let zone_count = r.get_i32()?;
        for _ in 0..zone_count.max(0) {
            let zone = self.read_zone(&mut r, world_version, &strings, ids)?;
            rep.read += 1;
            if java_equals_ignore_case("WorldGen", &zone.zone_type) {
                rep.skipped_world_gen += 1;
                continue;
            }
            let id = self.register_zone_def(zone);
            if self.zones.last() != Some(&id) {
                rep.suspicious += 1;
            }
        }
        let n = r.get_i32()?;
        for _ in 0..n.max(0) {
            let name = r.read_string_utf()?;
            let count = r.get_i32()?;
            let mut list = Vec::new();
            for _ in 0..count.max(0) {
                list.push(if world_version >= 215 {
                    r.read_uuid()?
                } else {
                    ids.random_uuid()
                });
            }
            spawned.put(name, list);
        }
        Ok(rep)
    }

    /// The clearing at the start of `loadZone`: drop every zone and every
    /// meta chunk left with neither rooms nor zones.
    fn clear_zones_for_load(&mut self) {
        self.zones.clear();
        for cell in self.grid.iter_mut().flatten() {
            for slot in cell.chunk_map.iter_mut() {
                if let Some(chunk) = slot {
                    chunk.clear_zones();
                    if chunk.rooms().is_empty() {
                        *slot = None;
                    }
                }
            }
        }
    }

    fn read_zone(
        &self,
        r: &mut JavaReader<'_>,
        world_version: i32,
        strings: &[String],
        ids: &mut dyn UuidSource,
    ) -> io::Result<Zone> {
        let lookup = |r: &mut JavaReader<'_>| -> io::Result<String> {
            let i = r.get_i16()?;
            usize::try_from(i)
                .ok()
                .and_then(|i| strings.get(i))
                .cloned()
                .ok_or_else(|| bad(format!("map_zone.bin: string index {i} out of range")))
        };
        let name = lookup(r)?;
        let zone_type = lookup(r)?;
        let mut zone = Zone::default();
        zone.name = name;
        zone.zone_type = zone_type;
        zone.x = r.get_i32()?;
        zone.y = r.get_i32()?;
        zone.z = r.get_i8()? as i32;
        zone.w = r.get_i32()?;
        zone.h = r.get_i32()?;
        zone.kind = ZoneKind::Zone;
        zone.geometry_type = match r.get_i8()? {
            1 => ZoneGeometryType::Point,
            2 => ZoneGeometryType::Polyline,
            3 => ZoneGeometryType::Polygon,
            _ => ZoneGeometryType::Invalid,
        };
        if zone.geometry_type != ZoneGeometryType::Invalid {
            if zone.geometry_type == ZoneGeometryType::Polyline {
                zone.polyline_width = (r.get_i8()? as i32).clamp(0, 255);
            }
            let n = r.get_i16()?;
            for _ in 0..n.max(0) {
                zone.points.push(r.get_i32()?);
            }
        }
        zone.is_preferred_zone_for_square = self
            .zone_tables
            .is_preferred_zone_for_square(&zone.zone_type);
        zone.hour_last_seen = r.get_i32()?;
        zone.have_construction = r.get_i8()? != 0;
        zone.last_action_timestamp = r.get_i32()?;
        zone.set_original_name(Some(lookup(r)?));
        zone.id = Some(if world_version >= 215 {
            r.read_uuid()?
        } else {
            r.get_f64()?;
            ids.random_uuid()
        });
        Ok(zone)
    }

    /// `saveZone(output)`. Zones without an id get a random one first (the
    /// original assigns it at construction), so later saves repeat it.
    pub fn save_zones(
        &mut self,
        spawned: &SpawnedZombieZones,
        ids: &mut dyn UuidSource,
    ) -> io::Result<Vec<u8>> {
        for &id in &self.zones {
            let zone = &mut self.zone_defs[id.0 as usize];
            if zone.id.is_none() {
                zone.id = Some(ids.random_uuid());
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.put_i32(WORLD_VERSION);
        let mut keys = Vec::with_capacity(self.zones.len() * 3);
        for &id in &self.zones {
            let z = self.zone(id);
            keys.push(Some(z.name.as_str()));
            keys.push(z.get_original_name());
            keys.push(Some(z.zone_type.as_str()));
        }
        let table = java_hash_order(&keys);
        if table.len() > i16::MAX as usize {
            return Err(io::Error::other(
                "IsoMetaGrid.saveZone() string table is too large",
            ));
        }
        let index: std::collections::HashMap<Option<&str>, i16> = table
            .iter()
            .enumerate()
            .map(|(i, s)| (*s, i as i16))
            .collect();
        out.put_i32(table.len() as i32);
        for s in &table {
            out.write_string_utf(*s);
        }
        out.put_i32(self.zones.len() as i32);
        for &id in &self.zones {
            let z = self.zone(id);
            out.put_i16(index[&Some(z.name.as_str())]);
            out.put_i16(index[&Some(z.zone_type.as_str())]);
            out.put_i32(z.x);
            out.put_i32(z.y);
            out.put_i8(z.z as i8);
            out.put_i32(z.w);
            out.put_i32(z.h);
            out.put_i8(z.geometry_type as i8);
            if !z.is_rectangle() {
                if z.is_polyline() {
                    out.put_i8(z.polyline_width as i8);
                }
                out.put_i16(z.points.len() as i16);
                for p in &z.points {
                    out.put_i32(*p);
                }
            }
            out.put_i32(z.hour_last_seen);
            out.put_bool(z.have_construction);
            out.put_i32(z.last_action_timestamp);
            out.put_i16(index[&z.get_original_name()]);
            out.write_uuid(z.id.expect("assigned above"));
        }
        let names: Vec<Option<&str>> = spawned
            .entries
            .iter()
            .map(|(k, _)| Some(k.as_str()))
            .collect();
        out.put_i32(spawned.entries.len() as i32);
        for name in java_hash_order(&names) {
            let name = name.expect("no null keys");
            let (_, list) = spawned
                .entries
                .iter()
                .find(|(k, _)| k == name)
                .expect("key from entries");
            out.write_string_utf(Some(name));
            out.put_i32(list.len() as i32);
            for u in list {
                out.write_uuid(*u);
            }
        }
        Ok(out)
    }

    /// `load("map_zone.bin", this::loadZone)`: a missing file changes
    /// nothing.
    pub fn load_zones_file(
        &mut self,
        save_dir: &Path,
        spawned: &mut SpawnedZombieZones,
        ids: &mut dyn UuidSource,
    ) -> io::Result<Option<ZoneLoadReport>> {
        match std::fs::read(save_dir.join("map_zone.bin")) {
            Ok(data) => self.load_zones(&data, -1, spawned, ids).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes `map_zone.bin` in `save_dir`.
    pub fn save_zones_file(
        &mut self,
        save_dir: &Path,
        spawned: &SpawnedZombieZones,
        ids: &mut dyn UuidSource,
    ) -> io::Result<()> {
        let bytes = self.save_zones(spawned, ids)?;
        std::fs::write(save_dir.join("map_zone.bin"), bytes)
    }
}

#[cfg(test)]
mod tests;
