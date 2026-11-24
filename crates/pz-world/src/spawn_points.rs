//! Build 42.22 `zombie.iso.SpawnPoints`: tracks parsed spawn coordinates and
//! resolves their `BuildingIdx` set so starter buildings are protected from
//! randomized building stories and initial zombie population.

use std::collections::HashSet;

use crate::defs::BuildingIdx;
use crate::meta_grid::IsoMetaGrid;

/// A single spawn coordinate `(x, y, z)` in tile space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpawnPoint {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl SpawnPoint {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Parse a `"x,y,z"` CSV spawn-point string (`SpawnPoint must be x,y,z`).
    pub fn parse_csv(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
        if parts.len() != 3 {
            return Err(format!("SpawnPoint must be x,y,z, got \"{s}\""));
        }
        let x = parts[0]
            .parse::<i32>()
            .map_err(|_| format!("SpawnPoint must be x,y,z, got \"{s}\""))?;
        let y = parts[1]
            .parse::<i32>()
            .map_err(|_| format!("SpawnPoint must be x,y,z, got \"{s}\""))?;
        let z = parts[2]
            .parse::<i32>()
            .map_err(|_| format!("SpawnPoint must be x,y,z, got \"{s}\""))?;
        Ok(Self { x, y, z })
    }

    /// Convert a Build 42.22 spawn-table entry (`worldX`, `worldY`, `posX`,
    /// `posY`, `posZ`) to absolute tile coordinates. When `world_x`/`world_y`
    /// are omitted, `pos_x`/`pos_y` are already absolute tile coordinates
    /// (Build 42 `spawnpoints.lua` format); otherwise `world * 300 + pos`.
    pub fn from_table_fields(
        world_x: Option<i32>,
        world_y: Option<i32>,
        pos_x: i32,
        pos_y: i32,
        pos_z: i32,
    ) -> Self {
        match (world_x, world_y) {
            (Some(wx), Some(wy)) => Self {
                x: wx.saturating_mul(300).saturating_add(pos_x),
                y: wy.saturating_mul(300).saturating_add(pos_y),
                z: pos_z,
            },
            _ => Self {
                x: pos_x,
                y: pos_y,
                z: pos_z,
            },
        }
    }
}

/// Build 42.22 `zombie.iso.SpawnPoints` state.
#[derive(Clone, Debug, Default)]
pub struct SpawnPoints {
    pub points: Vec<SpawnPoint>,
    pub spawn_buildings: HashSet<BuildingIdx>,
    pub spawn_building_ids: HashSet<i64>,
}

impl SpawnPoints {
    pub fn new() -> Self {
        Self::default()
    }

    /// `SpawnPoints.init()`: clears parsed points and resolved buildings.
    pub fn clear(&mut self) {
        self.points.clear();
        self.spawn_buildings.clear();
        self.spawn_building_ids.clear();
    }

    /// Add a unique spawn point.
    pub fn add_point(&mut self, pt: SpawnPoint) {
        if !self.points.contains(&pt) {
            self.points.push(pt);
        }
    }

    /// Add the `"position = center"` fallback point for `meta`.
    pub fn add_center_point(&mut self, meta: &IsoMetaGrid) {
        let cx = ((meta.min_x + meta.max_x) / 2) * 256 + 128;
        let cy = ((meta.min_y + meta.max_y) / 2) * 256 + 128;
        self.add_point(SpawnPoint::new(cx, cy, 0));
    }

    /// `initSpawnBuildings(IsoMetaGrid)`: maps every recorded spawn point to
    /// its containing `BuildingIdx` / `BuildingDef.id`.
    pub fn init_spawn_buildings(&mut self, meta: &IsoMetaGrid) -> usize {
        self.spawn_buildings.clear();
        self.spawn_building_ids.clear();
        for pt in &self.points {
            let b_idx = meta
                .get_room_at(pt.x, pt.y, pt.z)
                .and_then(|r| r.building)
                .or_else(|| meta.get_associated_building_at(pt.x, pt.y).ok().flatten());
            if let Some(bi) = b_idx {
                self.spawn_buildings.insert(bi);
                if let Some(bdef) = meta.building_defs.get(bi.0 as usize) {
                    self.spawn_building_ids.insert(bdef.id);
                }
            }
        }
        self.spawn_buildings.len()
    }

    /// `isSpawnBuilding(BuildingDef)` by index.
    pub fn is_spawn_building(&self, idx: BuildingIdx) -> bool {
        self.spawn_buildings.contains(&idx)
    }

    /// `isSpawnBuilding(BuildingDef)` by `BuildingDef.id`.
    pub fn is_spawn_building_id(&self, id: i64) -> bool {
        self.spawn_building_ids.contains(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{BuildingDef, RoomDef, RoomRect};
    use crate::GameRand;

    struct ZeroRand;
    impl GameRand for ZeroRand {
        fn next(&mut self, _: i32) -> i32 {
            0
        }
    }

    #[test]
    fn spawn_points_parse_and_resolve_buildings() {
        let mut g = IsoMetaGrid::blank(0, 0, 0, 0);
        g.create_cell(0, 0).unwrap();
        let mut r = RoomDef::new(0, "livingroom".to_string());
        r.level = 0;
        r.rects.push(RoomRect::new(20, 30, 8, 8));
        r.CalculateBounds();
        let b = g
            .add_building(0, 0, BuildingDef::new(&mut ZeroRand), vec![r], vec![])
            .unwrap();

        let mut sp = SpawnPoints::new();
        assert_eq!(SpawnPoint::parse_csv("22, 32, 0").unwrap(), SpawnPoint::new(22, 32, 0));
        assert!(SpawnPoint::parse_csv("bad").is_err());
        assert_eq!(
            SpawnPoint::from_table_fields(Some(10), Some(20), 5, 6, 0),
            SpawnPoint::new(3005, 6006, 0)
        );
        assert_eq!(
            SpawnPoint::from_table_fields(None, None, 22, 32, 0),
            SpawnPoint::new(22, 32, 0)
        );

        sp.add_point(SpawnPoint::new(22, 32, 0));
        let resolved = sp.init_spawn_buildings(&g);
        assert_eq!(resolved, 1);
        assert!(sp.is_spawn_building(b));
        assert!(sp.is_spawn_building_id(g.building_defs[b.0 as usize].id));
    }
}
