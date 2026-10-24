//! `zombie.iso.worldgen.zones.WorldGenZone`: rectangles of a map's
//! `objects.lua` with type `WorldGen`, registered per meta cell by
//! `IsoMetaGrid.registerWorldGenZone` (from Lua's `doMapZones`). World
//! generation only reads their `Rocks` property.

use std::collections::HashMap;
use std::path::Path;

use pz_kahlua::Value;

/// `WorldGenZone` (a rectangle zone).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldGenZone {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    pub h: i32,
    /// `properties.Rocks` when it is a boolean, else `true`.
    pub rocks: bool,
}

impl WorldGenZone {
    /// `Zone.contains(x, y, z)` for a rectangle.
    pub fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        z == self.z && x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// `IsoMetaCell.worldGenZones` of every meta cell.
#[derive(Clone, Debug, Default)]
pub struct WorldGenZones {
    /// The meta grid's cell bounds (`IsoMetaGrid.minX..maxX`).
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    zones: Vec<WorldGenZone>,
    cells: HashMap<(i32, i32), Vec<usize>>,
}

impl WorldGenZones {
    pub fn new(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> Self {
        WorldGenZones {
            min_x,
            min_y,
            max_x,
            max_y,
            ..Default::default()
        }
    }

    /// `registerWorldGenZone(...)` for a zone of type `WorldGen`.
    pub fn register(&mut self, zone: WorldGenZone) {
        let i = self.zones.len();
        let cell_max_x = ((zone.x + zone.w) as f32 / 256.0).ceil() as i32;
        let cell_max_y = ((zone.y + zone.h) as f32 / 256.0).ceil() as i32;
        for yy in zone.y / 256..cell_max_y {
            for xx in zone.x / 256..cell_max_x {
                if xx < self.min_x || xx > self.max_x || yy < self.min_y || yy > self.max_y {
                    continue;
                }
                self.cells.entry((xx, yy)).or_default().push(i);
            }
        }
        self.zones.push(zone);
    }

    /// Registers the `WorldGen` rectangles of each map's `objects.lua`, the
    /// way `doMapZones` walks the lot directories (last to first).
    pub fn load_maps(&mut self, fs: &pz_fs::GameFs, lot_dirs: &[&str]) -> Result<(), String> {
        for dir in lot_dirs.iter().rev() {
            let Some(p) = fs.resolve(&format!("media/maps/{dir}/objects.lua")) else {
                continue;
            };
            if p.exists() {
                self.load_objects(&p)?;
            }
        }
        Ok(())
    }

    fn load_objects(&mut self, path: &Path) -> Result<(), String> {
        let Some(objects) = crate::lua::load_data_table(path, "objects")? else {
            return Ok(());
        };
        let mut i = 1;
        loop {
            let v = objects.get_int(i);
            let Some(t) = v.as_table() else { break };
            i += 1;
            if t.get_str("type").as_str() != Some("WorldGen") || !t.get_str("geometry").is_nil() {
                continue;
            }
            let num = |k: &str| t.get_str(k).as_num().unwrap_or(0.0) as i32;
            let rocks = match t
                .get_str("properties")
                .as_table()
                .map(|p| p.get_str("Rocks"))
            {
                Some(Value::Bool(b)) => b,
                _ => true,
            };
            self.register(WorldGenZone {
                x: num("x"),
                y: num("y"),
                z: num("z"),
                w: num("width"),
                h: num("height"),
                rocks,
            });
        }
        Ok(())
    }

    /// `WorldGenChunk.getWorldGenZoneAt(x, y, z)`.
    pub fn zone_at(&self, x: i32, y: i32, z: i32) -> Option<&WorldGenZone> {
        let (cx, cy) = (x / 256, y / 256);
        if cx < self.min_x || cy < self.min_y || cx > self.max_x || cy > self.max_y {
            return None;
        }
        self.cells
            .get(&(cx, cy))?
            .iter()
            .map(|&i| &self.zones[i])
            .find(|z2| z2.contains(x, y, z))
    }

    pub fn len(&self) -> usize {
        self.zones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_containing_zone() {
        let mut z = WorldGenZones::new(0, 0, 10, 10);
        let zone = |x, rocks| WorldGenZone {
            x,
            y: 0,
            z: 0,
            w: 300,
            h: 10,
            rocks,
        };
        z.register(zone(250, false));
        z.register(zone(0, true));
        assert!(!z.zone_at(260, 5, 0).unwrap().rocks);
        assert!(z.zone_at(100, 5, 0).unwrap().rocks);
        assert!(z.zone_at(100, 5, 1).is_none());
        assert!(z.zone_at(100, 50, 0).is_none());
    }
}
