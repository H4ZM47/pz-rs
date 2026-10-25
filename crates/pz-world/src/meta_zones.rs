//! Zone side of `IsoMetaGrid`, `IsoMetaCell` and `IsoMetaChunk`: the
//! registration entry points map loading calls from Lua (`objects.lua`
//! through `metazoneHandler.lua`), per-chunk zone arrays and queries, and
//! the lot-priority removal (`removeZonesForLotDirectory`).
//!
//! These are plain Rust entry points; the Lua binding layer converts
//! Kahlua tables to [`ZoneProps`] / point slices and calls them.

use crate::java::{fastfloor, java_equals_ignore_case};
use crate::meta_grid::{IsoMetaCell, IsoMetaChunk, IsoMetaGrid};
use crate::zones::{
    calculate_polyline_outline_bounds, prop_string, world_gen_rocks, AnimalZoneData, ClipperOps,
    MannequinZoneData, PropValue, RoomTone, VehicleZoneData, Zone, ZoneError, ZoneGeometryType,
    ZoneId, ZoneKind, ZoneProps, ANY_Z,
};

impl IsoMetaChunk {
    pub fn zones(&self) -> &[ZoneId] {
        &self.zones
    }

    /// `getZonesSize()`.
    pub fn get_zones_size(&self) -> usize {
        self.zones.len()
    }

    pub fn add_zone(&mut self, zone: ZoneId) {
        self.zones.push(zone);
    }

    /// Removes the first occurrence.
    pub fn remove_zone(&mut self, zone: ZoneId) {
        if let Some(i) = self.zones.iter().position(|&z| z == zone) {
            self.zones.remove(i);
        }
    }

    pub fn get_zone(&self, index: i32) -> Option<ZoneId> {
        if index < 0 {
            return None;
        }
        self.zones.get(index as usize).copied()
    }

    /// `compactZoneArray()`.
    pub fn compact_zone_array(&mut self) {
        self.zones.shrink_to_fit();
    }

    pub fn clear_zones(&mut self) {
        self.zones = Vec::new();
    }

    /// `getZoneAt`: newest zone containing the square, except that the
    /// newest preferred-type zone wins.
    pub fn get_zone_at(&self, defs: &[Zone], x: i32, y: i32, z: i32) -> Option<ZoneId> {
        let mut highest = None;
        for &id in self.zones.iter().rev() {
            let zone = &defs[id.0 as usize];
            if !zone.contains(x, y, z) {
                continue;
            }
            if zone.is_preferred_zone_for_square {
                return Some(id);
            }
            if highest.is_none() {
                highest = Some(id);
            }
        }
        highest
    }

    /// `getZonesAt`, oldest first, appended to `result`.
    pub fn get_zones_at(&self, defs: &[Zone], x: i32, y: i32, z: i32, result: &mut Vec<ZoneId>) {
        for &id in &self.zones {
            if defs[id.0 as usize].contains(x, y, z) {
                result.push(id);
            }
        }
    }

    /// `getZoneAt(x, y, z, type)`: first zone at the square with exactly
    /// this type.
    pub fn get_zone_at_with_type(
        &self,
        defs: &[Zone],
        x: i32,
        y: i32,
        z: i32,
        zone_type: &str,
    ) -> Option<ZoneId> {
        let mut v = Vec::new();
        self.get_zones_at(defs, x, y, z, &mut v);
        v.into_iter()
            .find(|id| defs[id.0 as usize].zone_type == zone_type)
    }

    /// `getZonesIntersecting`: appends zones not already in `result`.
    #[allow(clippy::too_many_arguments)]
    pub fn get_zones_intersecting(
        &self,
        defs: &[Zone],
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        result: &mut Vec<ZoneId>,
    ) {
        for &id in &self.zones {
            if result.contains(&id) || !defs[id.0 as usize].intersects(x, y, z, w, h) {
                continue;
            }
            result.push(id);
        }
    }

    /// `doesHaveZone(type)` (case-insensitive).
    pub fn does_have_zone(&self, defs: &[Zone], zone_type: &str) -> bool {
        self.zones
            .iter()
            .any(|id| java_equals_ignore_case(&defs[id.0 as usize].zone_type, zone_type))
    }
}

/// Chunk-index span of `[x, x + w)` used by the cell loops: `(min, max)`
/// in chunk coordinates with Java's truncating division.
fn chunk_span(x: i32, w: i32) -> (i32, i32) {
    let end = x.wrapping_add(w);
    let mut max = end / 8;
    if end % 8 == 0 {
        max -= 1;
    }
    (x / 8, max)
}

/// `(int)Math.ceil((float)v / 256.0f)`.
fn ceil_cell(v: i32) -> i32 {
    ((v as f32 / 256.0f32) as f64).ceil() as i32
}

impl IsoMetaCell {
    /// `getZonesIntersecting` over this cell's existing chunks.
    #[allow(clippy::too_many_arguments)]
    pub fn get_zones_intersecting(
        &self,
        defs: &[Zone],
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        result: &mut Vec<ZoneId>,
    ) {
        let (xmin, xmax) = chunk_span(x, w);
        let (ymin, ymax) = chunk_span(y, h);
        let cell_x = self.get_x() * 256;
        let cell_y = self.get_y() * 256;
        for yy in ymin..=ymax {
            for xx in xmin..=xmax {
                if xx < cell_x / 8
                    || xx >= (cell_x + 256) / 8
                    || yy < cell_y / 8
                    || yy >= (cell_y + 256) / 8
                {
                    continue;
                }
                let i = (xx - cell_x / 8 + (yy - cell_y / 8) * 32) as usize;
                if let Some(c) = &self.chunk_map[i] {
                    c.get_zones_intersecting(defs, x, y, z, w, h, result);
                }
            }
        }
    }

    /// `getZonesUnique`: every zone referenced by a chunk, deduplicated in
    /// chunk order.
    pub fn get_zones_unique(&self, result: &mut Vec<ZoneId>) {
        for c in self.chunk_map.iter().flatten() {
            for &id in &c.zones {
                if !result.contains(&id) {
                    result.push(id);
                }
            }
        }
    }

    /// `removeZone(zone)`: unlinks it from this cell's chunks and drops
    /// chunks left without rooms and zones (`clearChunk`).
    fn remove_zone(&mut self, zone: &Zone, id: ZoneId) {
        let (xmin, xmax) = chunk_span(zone.x, zone.w);
        let (ymin, ymax) = chunk_span(zone.y, zone.h);
        let cell_x = self.get_x() * 256;
        let cell_y = self.get_y() * 256;
        for yy in ymin..=ymax {
            for xx in xmin..=xmax {
                if xx < cell_x / 8
                    || xx >= (cell_x + 256) / 8
                    || yy < cell_y / 8
                    || yy >= (cell_y + 256) / 8
                {
                    continue;
                }
                let i = (xx - cell_x / 8 + (yy - cell_y / 8) * 32) as usize;
                let Some(c) = self.chunk_map[i].as_mut() else {
                    continue;
                };
                c.remove_zone(id);
                if c.rooms().is_empty() && c.zones.is_empty() {
                    self.chunk_map[i] = None;
                }
            }
        }
    }
}

impl IsoMetaGrid {
    pub fn zone(&self, id: ZoneId) -> &Zone {
        &self.zone_defs[id.0 as usize]
    }

    pub fn zone_mut(&mut self, id: ZoneId) -> &mut Zone {
        &mut self.zone_defs[id.0 as usize]
    }

    fn push_zone(&mut self, zone: Zone) -> ZoneId {
        let id = ZoneId(self.zone_defs.len() as u32);
        self.zone_defs.push(zone);
        id
    }

    /// Computes the cached polyline outline through the native clipper
    /// (done lazily upstream; the result is the same).
    fn ensure_outline(&mut self, id: ZoneId) {
        let clipper = self
            .clipper
            .as_deref_mut()
            .map(|c| c as &mut dyn ClipperOps);
        self.zone_defs[id.0 as usize].check_polyline_outline(clipper);
    }

    /// `getCellOrCreate(gridX, gridY)`.
    fn get_cell_or_create(&mut self, gx: i32, gy: i32) -> &mut IsoMetaCell {
        let (wx, wy) = (self.min_x + gx, self.min_y + gy);
        self.cell_slot(gx, gy)
            .get_or_insert_with(|| IsoMetaCell::new(wx, wy))
    }

    fn has_cell(&self, gx: i32, gy: i32) -> bool {
        self.get_cell(gx, gy).is_some()
    }

    /// `getZoneAt(x, y, z)`.
    pub fn get_zone_at(&self, x: i32, y: i32, z: i32) -> Option<ZoneId> {
        self.get_chunk_data_from_tile(x, y)?
            .get_zone_at(&self.zone_defs, x, y, z)
    }

    /// `getZonesAt(x, y, z)`, oldest first.
    pub fn get_zones_at(&self, x: i32, y: i32, z: i32) -> Vec<ZoneId> {
        let mut result = Vec::new();
        if let Some(c) = self.get_chunk_data_from_tile(x, y) {
            c.get_zones_at(&self.zone_defs, x, y, z, &mut result);
        }
        result
    }

    /// `getZonesIntersecting(x, y, z, w, h, result)`.
    #[allow(clippy::too_many_arguments)]
    pub fn get_zones_intersecting_into(
        &self,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        result: &mut Vec<ZoneId>,
    ) {
        for yy in y / 256..=y.wrapping_add(h) / 256 {
            for xx in x / 256..=x.wrapping_add(w) / 256 {
                if xx < self.min_x || xx > self.max_x || yy < self.min_y || yy > self.max_y {
                    continue;
                }
                if let Some(cell) = self.get_cell(xx - self.min_x, yy - self.min_y) {
                    cell.get_zones_intersecting(&self.zone_defs, x, y, z, w, h, result);
                }
            }
        }
    }

    /// `getZonesIntersecting(x, y, z, w, h)`.
    pub fn get_zones_intersecting(&self, x: i32, y: i32, z: i32, w: i32, h: i32) -> Vec<ZoneId> {
        let mut result = Vec::new();
        self.get_zones_intersecting_into(x, y, z, w, h, &mut result);
        result
    }

    /// `getZoneWithBoundsAndType`.
    pub fn get_zone_with_bounds_and_type(
        &self,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        zone_type: &str,
    ) -> Option<ZoneId> {
        self.get_zones_intersecting(x, y, z, w, h)
            .into_iter()
            .find(|&id| {
                let zn = self.zone(id);
                zn.x == x
                    && zn.y == y
                    && zn.z == z
                    && zn.w == w
                    && zn.h == h
                    && java_equals_ignore_case(&zn.zone_type, zone_type)
            })
    }

    /// `getMetaGridFromTile(x, y)`.
    pub fn get_meta_grid_from_tile(&self, x: i32, y: i32) -> Option<&IsoMetaCell> {
        self.get_cell_data(x / 256, y / 256)
    }

    /// `getVehicleZoneAt(x, y, z)`.
    pub fn get_vehicle_zone_at(&self, x: i32, y: i32, z: i32) -> Option<ZoneId> {
        let cell = self.get_meta_grid_from_tile(x, y)?;
        cell.vehicle_zones
            .iter()
            .copied()
            .find(|&id| self.zone(id).contains(x, y, z))
    }

    /// `isZoneAbove(zone1, zone2, x, y, z)`.
    pub fn is_zone_above(
        &self,
        zone1: Option<ZoneId>,
        zone2: Option<ZoneId>,
        x: i32,
        y: i32,
        z: i32,
    ) -> bool {
        let Some(z1) = zone1 else { return false };
        if zone2 == Some(z1) {
            return false;
        }
        let zones = self.get_zones_at(x, y, z);
        let index_of = |z: Option<ZoneId>| -> i32 {
            z.and_then(|z| zones.iter().position(|&q| q == z))
                .map_or(-1, |i| i as i32)
        };
        index_of(zone1) > index_of(zone2)
    }

    fn is_suspicious(&self, x: i32, y: i32, z: i32, w: i32, h: i32) -> bool {
        x < self.min_x * 256 - 100
            || y < self.min_y * 256 - 100
            || x.wrapping_add(w) > (self.max_x + 1) * 256 + 100
            || y.wrapping_add(h) > (self.max_y + 1) * 256 + 100
            || !(-32..=31).contains(&z)
            || w > 1202
            || h > 1202
    }

    /// `registerZone(name, type, x, y, z, w, h)`. A suspicious zone is
    /// created (and returned) but not added.
    #[allow(clippy::too_many_arguments)]
    pub fn register_zone(
        &mut self,
        name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
    ) -> ZoneId {
        self.register_zone_with_geometry(
            name,
            zone_type,
            x,
            y,
            z,
            w,
            h,
            ZoneGeometryType::Invalid,
            None,
            0,
        )
    }

    /// `registerZone(name, type, x, y, z, w, h, geometryType, points, width)`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_zone_with_geometry(
        &mut self,
        name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        geometry_type: ZoneGeometryType,
        points: Option<&[i32]>,
        polyline_width: i32,
    ) -> ZoneId {
        let zone = Zone::with_geometry(
            name,
            zone_type,
            x,
            y,
            z,
            w,
            h,
            geometry_type,
            points,
            polyline_width,
            &self.zone_tables,
        );
        let suspicious = self.is_suspicious(x, y, z, w, h);
        let id = self.push_zone(zone);
        if suspicious {
            pz_core::debug::log(&format!(
                "111ERROR: not adding suspicious zone \"{name}\" \"{zone_type}\" {x},{y},{z} {w}x{h}"
            ));
            return id;
        }
        self.add_zone(id);
        id
    }

    /// `registerZone(Zone)` (save-file loading). Like the original, the
    /// right/bottom checks add the grid's width/height rather than the
    /// zone's.
    pub fn register_zone_def(&mut self, zone: Zone) -> ZoneId {
        let suspicious = zone.x < self.min_x * 256 - 100
            || zone.y < self.min_y * 256 - 100
            || zone.x.wrapping_add(self.width) > (self.max_x + 1) * 256 + 100
            || zone.y.wrapping_add(self.height) > (self.max_y + 1) * 256 + 100
            || zone.z < -32
            || zone.z > 31
            || zone.w > 1202
            || zone.h > 1202;
        let id = self.push_zone(zone);
        if !suspicious {
            self.ensure_outline(id);
            self.add_zone(id);
        }
        id
    }

    /// `registerZoneNoOverlap` on the meta grid (deprecated; note that
    /// `IsoWorld.registerZoneNoOverlap` calls `registerZone` instead).
    #[allow(clippy::too_many_arguments)]
    pub fn register_zone_no_overlap(
        &mut self,
        name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
    ) -> Option<ZoneId> {
        if x < self.min_x * 256 - 100
            || y < self.min_y * 256 - 100
            || x.wrapping_add(w) > (self.max_x + 1) * 256 + 100
            || y.wrapping_add(h) > (self.max_y + 1) * 256 + 100
            || !(0..8).contains(&z)
            || w > 601
            || h > 601
        {
            return None;
        }
        Some(self.register_zone(name, zone_type, x, y, z, w, h))
    }

    /// `registerGeometryZone(name, type, z, geometry, pointsTable, properties)`
    /// as called from `metazoneHandler.lua`. `points` are the Lua numbers of
    /// the flat points table.
    ///
    /// Only a polyline's `LineWidth` contributes bounds; other geometry is
    /// registered with the `Integer.MAX_VALUE`/`MIN_VALUE` bounds the
    /// original computes, which keeps it out of every cell.
    pub fn register_geometry_zone(
        &mut self,
        name: &str,
        zone_type: &str,
        z: i32,
        geometry: &str,
        points: &[f64],
        properties: Option<&dyn ZoneProps>,
    ) -> Result<Option<ZoneId>, ZoneError> {
        if !points.len().is_multiple_of(2) {
            return Err(ZoneError::OddPointCount);
        }
        let pts: Vec<i32> = points.iter().map(|&d| d as i32).collect();
        let geometry_type = match geometry {
            "point" => ZoneGeometryType::Point,
            "polygon" => ZoneGeometryType::Polygon,
            "polyline" => ZoneGeometryType::Polyline,
            other => return Err(ZoneError::UnknownGeometry(other.to_string())),
        };
        let width = match (geometry_type, properties) {
            (ZoneGeometryType::Polyline, Some(p)) => match p.rawget("LineWidth") {
                PropValue::Number(d) => d as i32,
                _ => 0,
            },
            _ => 0,
        };
        self.register_geometry_zone_typed(
            name,
            zone_type,
            z,
            geometry_type,
            &pts,
            properties,
            width,
        )
    }

    /// `registerGeometryZone(name, type, z, geometryType, points, properties, width)`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_geometry_zone_typed(
        &mut self,
        name: &str,
        zone_type: &str,
        z: i32,
        geometry_type: ZoneGeometryType,
        points: &[i32],
        properties: Option<&dyn ZoneProps>,
        width: i32,
    ) -> Result<Option<ZoneId>, ZoneError> {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        if width > 0 {
            let b = calculate_polyline_outline_bounds(
                self.clipper
                    .as_deref_mut()
                    .map(|c| c as &mut dyn ClipperOps),
                points,
                width,
            );
            (min_x, min_y, max_x, max_y) = (b[0], b[1], b[2], b[3]);
        }
        let (w, h) = (
            max_x.wrapping_sub(min_x).wrapping_add(1),
            max_y.wrapping_sub(min_y).wrapping_add(1),
        );
        let special = match zone_type {
            "Animal" => {
                Some(self.register_animal_zone(name, zone_type, min_x, min_y, z, w, h, properties))
            }
            "Vehicle" | "ParkingStall" => Some(
                self.register_vehicles_zone(name, zone_type, min_x, min_y, z, w, h, properties)?,
            ),
            "WorldGen" => Some(
                self.register_world_gen_zone(name, zone_type, min_x, min_y, z, w, h, properties),
            ),
            _ => None,
        };
        if let Some(zone) = special {
            if let Some(id) = zone {
                let zn = self.zone_mut(id);
                zn.geometry_type = geometry_type;
                zn.points.extend_from_slice(points);
                zn.polyline_width = width;
                self.ensure_outline(id);
            }
            return Ok(zone);
        }
        Ok(Some(self.register_zone_with_geometry(
            name,
            zone_type,
            min_x,
            min_y,
            z,
            w,
            h,
            geometry_type,
            Some(points),
            width,
        )))
    }

    /// `addZone(zone)`: appends to `zones` and links it into every chunk of
    /// the cells it spans whose 8x8 area it intersects.
    pub fn add_zone(&mut self, id: ZoneId) {
        self.ensure_outline(id);
        self.zones.push(id);
        let (zx, zy, zw, zh) = {
            let z = self.zone(id);
            (z.x, z.y, z.w, z.h)
        };
        for yy in zy / 256..=zy.wrapping_add(zh) / 256 {
            for xx in zx / 256..=zx.wrapping_add(zw) / 256 {
                if xx < self.min_x || xx > self.max_x || yy < self.min_y || yy > self.max_y {
                    continue;
                }
                self.get_cell_or_create(xx - self.min_x, yy - self.min_y);
                self.add_zone_to_cell(xx, yy, id);
            }
        }
    }

    /// `IsoMetaCell.addZone(zone, cellX*256, cellY*256)`.
    fn add_zone_to_cell(&mut self, cell_x: i32, cell_y: i32, id: ZoneId) {
        let (ox, oy) = (cell_x * 256, cell_y * 256);
        let zone = self.zone(id);
        let (xmin, xmax) = chunk_span(zone.x, zone.w);
        let (ymin, ymax) = chunk_span(zone.y, zone.h);
        let xmin = xmin.clamp_java(ox / 8, (ox + 256) / 8);
        let ymin = ymin.clamp_java(oy / 8, (oy + 256) / 8);
        let xmax = xmax.clamp_java(ox / 8, (ox + 256) / 8 - 1);
        let ymax = ymax.clamp_java(oy / 8, (oy + 256) / 8 - 1);
        let mut chunks = Vec::new();
        for yy in ymin..=ymax {
            for xx in xmin..=xmax {
                if !zone.intersects(xx * 8, yy * 8, zone.z, 8, 8) {
                    continue;
                }
                chunks.push((xx - ox / 8 + (yy - oy / 8) * 32) as usize);
            }
        }
        for i in chunks {
            if let Some(c) = self.meta_chunk_mut(cell_x, cell_y, i) {
                c.add_zone(id);
            }
        }
    }

    /// `removeZone(zone)`.
    pub fn remove_zone(&mut self, id: ZoneId) {
        if let Some(i) = self.zones.iter().position(|&z| z == id) {
            self.zones.remove(i);
        }
        let zone = self.zone_defs[id.0 as usize].clone();
        for yy in zone.y / 256..=zone.y.wrapping_add(zone.h) / 256 {
            for xx in zone.x / 256..=zone.x.wrapping_add(zone.w) / 256 {
                if xx < self.min_x || xx > self.max_x || yy < self.min_y || yy > self.max_y {
                    continue;
                }
                let (gx, gy) = (xx - self.min_x, yy - self.min_y);
                if let Some(cell) = self.cell_slot(gx, gy).as_mut() {
                    cell.remove_zone(&zone, id);
                }
            }
        }
    }

    /// `removeZonesForCell(cell300X, cell300Y)`: cuts every zone out of
    /// the 300x300 cell (replacing it with its remaining pieces), then drops
    /// overlapping vehicle, mannequin and world-gen zones. As in the
    /// original, that second step looks up the meta cell at the 300-cell
    /// coordinates.
    pub fn remove_zones_for_cell(&mut self, c300x: i32, c300y: i32) {
        let (x, y) = (c300x * 300, c300y * 300);
        let zones = self.get_zones_intersecting(x, y, ANY_Z, 300, 300);
        for cut in zones {
            let diff = {
                let clipper = self
                    .clipper
                    .as_deref_mut()
                    .map(|c| c as &mut dyn ClipperOps);
                self.zone_defs[cut.0 as usize].difference(x, y, ANY_Z, 300, 300, clipper)
            };
            let Some(diff) = diff else { continue };
            self.remove_zone(cut);
            for piece in diff {
                let id = self.push_zone(piece);
                self.add_zone(id);
            }
        }
        let min_x = fastfloor((c300x * 300) as f32 / 256.0);
        let min_y = fastfloor((c300y * 300) as f32 / 256.0);
        let max_x = fastfloor(((c300x + 1) * 300) as f32 / 256.0);
        let max_y = fastfloor(((c300y + 1) * 300) as f32 / 256.0);
        for _yy in min_y..=max_y {
            for _xx in min_x..=max_x {
                let (gx, gy) = (c300x - self.min_x, c300y - self.min_y);
                if gx < 0 || gy < 0 || gx >= self.width || gy >= self.height {
                    continue;
                }
                let Some(cell) = self.grid[(gx * self.height + gy) as usize].as_mut() else {
                    continue;
                };
                let defs = &self.zone_defs;
                let hit = |id: &ZoneId| defs[id.0 as usize].intersects(x, y, ANY_Z, 300, 300);
                remove_overlapping(&mut cell.vehicle_zones, &hit);
                remove_overlapping(&mut cell.mannequin_zones, &hit);
                if let Some(v) = cell.world_gen_zones.as_mut() {
                    remove_overlapping(v, &hit);
                }
            }
        }
    }

    /// `removeZonesForLotDirectory(lotDir)`: before a map's `objects.lua`
    /// is loaded (lowest priority first), clears every 300x300 cell that
    /// map provides so its zones replace those of lower-priority maps.
    pub fn remove_zones_for_lot_directory(&mut self, lot_dir: &str) {
        if self.zones.is_empty() {
            return;
        }
        let Some(mf) = self
            .map_files
            .iter()
            .find(|mf| java_equals_ignore_case(&mf.map_directory_name, lot_dir))
        else {
            return;
        };
        let mut cells = Vec::new();
        for cy in mf.min_cell300_y..=mf.max_cell300_y {
            for cx in mf.min_cell300_x..=mf.max_cell300_x {
                if mf.has_cell300(cx, cy) {
                    cells.push((cx, cy));
                }
            }
        }
        for (cx, cy) in cells {
            self.remove_zones_for_cell(cx, cy);
        }
    }

    /// `processZones()`: compacts chunk arrays; returns the largest number
    /// of zones on one chunk (logged upstream).
    pub fn process_zones(&mut self) -> usize {
        let mut max_zone = 0;
        for cell in self.grid.iter_mut().flatten() {
            for c in cell.chunk_map.iter_mut().flatten() {
                c.compact_zone_array();
                max_zone = max_zone.max(c.get_zones_size());
            }
        }
        max_zone
    }

    /// Grid cells `[x/256, ceil((x+w)/256))` x `[y/256, ceil((y+h)/256))`
    /// inside the grid, as used by the vehicle/mannequin/world-gen/animal
    /// registrations.
    fn ceil_cells(&self, x: i32, y: i32, w: i32, h: i32) -> Vec<(i32, i32)> {
        let cell_max_x = ceil_cell(x.wrapping_add(w));
        let cell_max_y = ceil_cell(y.wrapping_add(h));
        let mut out = Vec::new();
        for yy in y / 256..cell_max_y {
            for xx in x / 256..cell_max_x {
                if xx < self.min_x || xx > self.max_x || yy < self.min_y || yy > self.max_y {
                    continue;
                }
                out.push((xx - self.min_x, yy - self.min_y));
            }
        }
        out
    }

    /// `registerVehiclesZone`: `None` unless the type is `Vehicle` or
    /// `ParkingStall`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_vehicles_zone(
        &mut self,
        name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        properties: Option<&dyn ZoneProps>,
    ) -> Result<Option<ZoneId>, ZoneError> {
        if zone_type != "Vehicle" && zone_type != "ParkingStall" {
            return Ok(None);
        }
        let data = VehicleZoneData::from_props(properties, &self.zone_tables)?;
        let mut zone = Zone::new(name, zone_type, x, y, z, w, h);
        zone.kind = ZoneKind::Vehicle(data);
        let id = self.push_zone(zone);
        self.vehicles_zones.push(id);
        for (gx, gy) in self.ceil_cells(x, y, w, h) {
            self.get_cell_or_create(gx, gy).vehicle_zones.push(id);
        }
        Ok(Some(id))
    }

    /// `registerWorldGenZone`: `None` unless the type is `WorldGen`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_world_gen_zone(
        &mut self,
        name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        properties: Option<&dyn ZoneProps>,
    ) -> Option<ZoneId> {
        if zone_type != "WorldGen" {
            return None;
        }
        let mut zone = Zone::new(name, zone_type, x, y, z, w, h);
        zone.kind = ZoneKind::WorldGen {
            rocks: world_gen_rocks(properties),
        };
        let id = self.push_zone(zone);
        for (gx, gy) in self.ceil_cells(x, y, w, h) {
            self.get_cell_or_create(gx, gy)
                .world_gen_zones
                .get_or_insert_with(Vec::new)
                .push(id);
        }
        Some(id)
    }

    /// `registerMannequinZone`: `None` unless the type is `Mannequin`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_mannequin_zone(
        &mut self,
        name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        properties: Option<&dyn ZoneProps>,
    ) -> Result<Option<ZoneId>, ZoneError> {
        if zone_type != "Mannequin" {
            return Ok(None);
        }
        let data = MannequinZoneData::from_props(properties, &self.zone_tables)?;
        let mut zone = Zone::new(name, zone_type, x, y, z, w, h);
        zone.kind = ZoneKind::Mannequin(data);
        let id = self.push_zone(zone);
        for (gx, gy) in self.ceil_cells(x, y, w, h) {
            self.get_cell_or_create(gx, gy).mannequin_zones.push(id);
        }
        Ok(Some(id))
    }

    /// `registerAnimalZone(name, type, x, y, z, w, h, properties)`: `None`
    /// unless the type is `Animal`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_animal_zone(
        &mut self,
        name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        w: i32,
        h: i32,
        properties: Option<&dyn ZoneProps>,
    ) -> Option<ZoneId> {
        if zone_type != "Animal" {
            return None;
        }
        let mut zone = Zone::new(name, zone_type, x, y, z, w, h);
        zone.kind = ZoneKind::Animal(AnimalZoneData::from_props(properties));
        Some(self.register_animal_zone_def(zone))
    }

    /// `registerAnimalZone(AnimalZone)`. Marking cells for saving
    /// (`addCellToSave`) is not modelled.
    pub fn register_animal_zone_def(&mut self, zone: Zone) -> ZoneId {
        let (x, y, w, h) = (zone.x, zone.y, zone.w, zone.h);
        let id = self.push_zone(zone);
        self.animal_zones.push(id);
        for (gx, gy) in self.ceil_cells(x, y, w, h) {
            self.get_cell_or_create(gx, gy).animal_zones.push(id);
        }
        id
    }

    /// `registerRoomTone`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_room_tone(
        &mut self,
        _name: &str,
        zone_type: &str,
        x: i32,
        y: i32,
        z: i32,
        _w: i32,
        _h: i32,
        properties: Option<&dyn ZoneProps>,
    ) -> Result<(), ZoneError> {
        if zone_type != "RoomTone" {
            return Ok(());
        }
        let props = properties.ok_or(ZoneError::MissingProperties)?;
        let tone = RoomTone {
            x,
            y,
            z,
            enum_value: prop_string(props, "RoomTone")?,
            entire_building: props.rawget("EntireBuilding") == PropValue::Bool(true),
        };
        let (gx, gy) = (x / 256 - self.min_x, y / 256 - self.min_y);
        if gx < 0 || gy < 0 || gx >= self.width || gy >= self.height {
            return Err(ZoneError::NoCell);
        }
        let cell = self.cell_slot(gx, gy).as_mut().ok_or(ZoneError::NoCell)?;
        cell.room_tones.push(tone);
        Ok(())
    }

    /// `checkVehiclesZones()`: drops later vehicle zones with the same
    /// x, y, w and h as an earlier one (from the global list only).
    pub fn check_vehicles_zones(&mut self) {
        let mut i = 0;
        while i < self.vehicles_zones.len() {
            let a = self.zone(self.vehicles_zones[i]);
            let unique = self.vehicles_zones[..i].iter().all(|&k| {
                let b = self.zone(k);
                a.x != b.x || a.y != b.y || a.h != b.h || a.w != b.w
            });
            if unique {
                i += 1;
            } else {
                self.vehicles_zones.remove(i);
            }
        }
    }

    /// Whether the grid cell exists (`hasCell(gridX, gridY)`).
    pub fn has_grid_cell(&self, gx: i32, gy: i32) -> bool {
        self.has_cell(gx, gy)
    }
}

fn remove_overlapping(zones: &mut Vec<ZoneId>, hit: &impl Fn(&ZoneId) -> bool) {
    let mut i = zones.len();
    while i > 0 {
        i -= 1;
        if hit(&zones[i]) {
            zones.remove(i);
        }
    }
}

/// `PZMath.clamp(int, min, max)` (min checked first).
trait ClampJava {
    fn clamp_java(self, min: i32, max: i32) -> i32;
}

impl ClampJava for i32 {
    fn clamp_java(self, min: i32, max: i32) -> i32 {
        if self < min {
            min
        } else if self > max {
            max
        } else {
            self
        }
    }
}
