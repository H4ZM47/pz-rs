//! Build 42.22 `zombie.LootRespawn`: periodic chunk/container loot respawn
//! gated by `SandboxOptions` (`LootRespawn` / `HoursForLootRespawn`,
//! `SeenHoursPreventLootRespawn`, `MaxItemsForLootRespawn`,
//! `ConstructionPreventsLootRespawn`) and zone types (`TownZone`, `TownZones`,
//! `TrailerPark`).

use crate::chunk::IsoChunk;
use crate::meta_grid::IsoMetaGrid;

/// Zone types eligible for Build 42.22 `LootRespawn`.
pub const RESPAWN_ZONE_TYPES: &[&str] = &["TownZone", "TownZones", "TrailerPark"];

/// Configuration derived from `SandboxOptions` / `ServerOptions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LootRespawnConfig {
    /// Respawn interval in world-age hours (`0` disables loot respawn).
    pub hours_for_respawn: i32,
    /// Minimum hours since a chunk was last seen before respawning (`0` = no limit).
    pub seen_hours_prevent_respawn: i32,
    /// Skip containers that already hold at least this many items (`0` = no limit).
    pub max_items_for_respawn: i32,
    /// Skip chunks with player construction if `true`.
    pub construction_prevents_respawn: bool,
}

impl Default for LootRespawnConfig {
    fn default() -> Self {
        Self {
            hours_for_respawn: 0,
            seen_hours_prevent_respawn: 0,
            max_items_for_respawn: 4,
            construction_prevents_respawn: true,
        }
    }
}

impl LootRespawnConfig {
    /// Convert Build 42.22's `SandboxOptions.LootRespawn` enum index (1..=5)
    /// into world-age hours: 1=None(0), 2=EveryDay(24), 3=EveryWeek(168),
    /// 4=EveryMonth(720), 5=Every2Months(1440).
    pub fn hours_from_sandbox_enum(choice: i32) -> i32 {
        match choice {
            2 => 24,
            3 => 24 * 7,
            4 => 24 * 30,
            5 => 24 * 60,
            _ => 0,
        }
    }
}

/// Build 42.22 `zombie.LootRespawn` runtime tracker.
#[derive(Clone, Debug)]
pub struct LootRespawn {
    pub last_respawn: i32,
    pub config: LootRespawnConfig,
}

impl Default for LootRespawn {
    fn default() -> Self {
        Self {
            last_respawn: -1,
            config: LootRespawnConfig::default(),
        }
    }
}

impl LootRespawn {
    pub fn new() -> Self {
        Self::default()
    }

    /// `LootRespawn.Reset()`.
    pub fn reset(&mut self) {
        self.last_respawn = -1;
    }

    /// Whether a zone type string participates in Build 42.22 loot respawn.
    pub fn is_respawn_zone(zone_type: &str) -> bool {
        RESPAWN_ZONE_TYPES
            .iter()
            .any(|z| z.eq_ignore_ascii_case(zone_type))
    }

    /// Whether `chunk`'s center tile lies inside an eligible respawn zone.
    pub fn chunk_in_respawn_zone(meta: &IsoMetaGrid, wx: i32, wy: i32) -> bool {
        let tx = wx * 8 + 4;
        let ty = wy * 8 + 4;
        meta.get_zones_at(tx, ty, 0)
            .into_iter()
            .filter_map(|id| meta.zone_defs.get(id.0 as usize))
            .any(|z| Self::is_respawn_zone(&z.zone_type))
    }

    /// `LootRespawn.update()` interval gate: returns `true` when the world age
    /// crosses a new `hours_for_respawn` bucket and loaded chunks should be
    /// checked.
    pub fn check_update(&mut self, world_age_hours: i32) -> bool {
        let interval = self.config.hours_for_respawn;
        if interval <= 0 {
            return false;
        }
        if self.last_respawn == -1 {
            self.last_respawn = world_age_hours;
            return false;
        }
        if world_age_hours / interval > self.last_respawn / interval {
            self.last_respawn = world_age_hours;
            true
        } else {
            false
        }
    }

    /// Evaluate and stamp a single chunk for respawn (`chunkLoaded` or
    /// periodic `update`). Returns `true` when the chunk crossed a respawn
    /// boundary and its `loot_respawn_hour` was advanced to `world_age_hours`.
    pub fn process_chunk(&self, chunk: &mut IsoChunk, world_age_hours: i32) -> bool {
        let interval = self.config.hours_for_respawn;
        if interval <= 0 {
            return false;
        }
        if chunk.saved.loot_respawn_hour == -1 {
            chunk.saved.loot_respawn_hour = world_age_hours;
            return false;
        }
        if world_age_hours / interval <= chunk.saved.loot_respawn_hour / interval {
            return false;
        }
        chunk.saved.loot_respawn_hour = world_age_hours;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loot_respawn_interval_and_chunk_tracking() {
        let mut lr = LootRespawn::new();
        lr.config.hours_for_respawn = LootRespawnConfig::hours_from_sandbox_enum(2);
        assert_eq!(lr.config.hours_for_respawn, 24);
        assert!(LootRespawn::is_respawn_zone("TownZone"));
        assert!(LootRespawn::is_respawn_zone("TrailerPark"));
        assert!(!LootRespawn::is_respawn_zone("DeepForest"));

        // First tick initializes last_respawn without firing.
        assert!(!lr.check_update(10));
        assert!(!lr.check_update(23));
        // Crossing hour 24 triggers respawn.
        assert!(lr.check_update(24));
        assert!(!lr.check_update(47));
        assert!(lr.check_update(48));

        let mut chunk = IsoChunk::new(10, 10);
        assert_eq!(chunk.saved.loot_respawn_hour, -1);
        assert!(!lr.process_chunk(&mut chunk, 10));
        assert_eq!(chunk.saved.loot_respawn_hour, 10);
        assert!(!lr.process_chunk(&mut chunk, 23));
        assert!(lr.process_chunk(&mut chunk, 25));
        assert_eq!(chunk.saved.loot_respawn_hour, 25);

        lr.reset();
        assert_eq!(lr.last_respawn, -1);
    }
}
