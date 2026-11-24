//! Build 42.22 `zombie.inventory.ItemConfigurator`: preprocesses zone names,
//! room names, container items, vehicle containers, vehicle names, `"freezer"`,
//! sprite container types, and tile IDs into integer selector tables used by
//! `ItemConfig` buckets during item spawning.

use std::collections::HashMap;

use pz_scripts::objects::items::Item;
use pz_scripts::objects::vehicles::VehicleScript;
use pz_scripts::{ScriptManager, ScriptType};

/// `ItemConfigurator.vehicle_containers` in Build 42.22.
pub const VEHICLE_CONTAINERS: &[&str] = &[
    "TruckBed",
    "TruckBedOpen",
    "SeatFrontLeft",
    "SeatFrontRight",
    "SeatMiddleLeft",
    "SeatMiddleRight",
    "SeatRearLeft",
    "SeatRearRight",
    "GloveBox",
];

/// `zombie.inventory.ItemConfigurator` string/tile selector ID registry.
#[derive(Clone, Debug, Default)]
pub struct ItemConfigurator {
    next_id: i32,
    string_ids: HashMap<String, i32>,
    tile_ids: HashMap<String, i32>,
}

impl ItemConfigurator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears both selector maps and resets the ID counter.
    pub fn clear(&mut self) {
        self.next_id = 0;
        self.string_ids.clear();
        self.tile_ids.clear();
    }

    /// `ItemConfigurator.registerString(String)`: assigns a sequential integer
    /// ID if `s` is non-empty/non-whitespace and not yet registered.
    pub fn register_string(&mut self, s: &str) -> bool {
        if s.trim().is_empty() || self.string_ids.contains_key(s) {
            return false;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.string_ids.insert(s.to_string(), id);
        true
    }

    /// `ItemConfigurator.registerZone(String)`.
    pub fn register_zone(&mut self, zone: &str) -> bool {
        self.register_string(zone)
    }

    /// Registers a tile sprite name -> sprite ID entry in `TILE_INTEGER_HASH_MAP`.
    pub fn register_sprite(&mut self, name: &str, sprite_id: i32) {
        if !name.trim().is_empty() {
            self.tile_ids.insert(name.to_string(), sprite_id);
        }
    }

    /// `ItemConfigurator.GetIdForString(String)`: returns `-1` when unknown.
    pub fn get_id_for_string(&self, s: &str) -> i32 {
        self.string_ids.get(s).copied().unwrap_or(-1)
    }

    /// `ItemConfigurator.GetIdForSprite(String)`: returns `-1` when unknown.
    pub fn get_id_for_sprite(&self, s: &str) -> i32 {
        self.tile_ids.get(s).copied().unwrap_or(-1)
    }

    pub fn string_count(&self) -> usize {
        self.string_ids.len()
    }

    pub fn tile_count(&self) -> usize {
        self.tile_ids.len()
    }

    /// `ItemConfigurator.Preprocess()` matching Build 42.22 registration order:
    /// 1. Zone names and zone types from `IsoMetaGrid.zones`
    /// 2. Room names from `IsoMetaGrid.buildings`
    /// 3. Container item names from `ScriptManager.getAllItems()`
    /// 4. `VEHICLE_CONTAINERS`
    /// 5. Vehicle script names from `ScriptManager.getAllVehicleScripts()`
    /// 6. `"freezer"`
    /// 7. Sprite `"container"` property values from `IsoSpriteManager.namedMap`
    /// 8. Sprite `(name, id)` pairs from `IsoSpriteManager.intMap`
    pub fn preprocess<'a>(
        &mut self,
        sm: &ScriptManager,
        zones: impl IntoIterator<Item = (&'a str, &'a str)>,
        rooms: impl IntoIterator<Item = &'a str>,
        container_props: impl IntoIterator<Item = &'a str>,
        tile_sprites: impl IntoIterator<Item = (&'a str, i32)>,
    ) {
        self.clear();
        for (name, zone_type) in zones {
            self.register_string(name);
            self.register_string(zone_type);
        }
        for room in rooms {
            self.register_string(room);
        }
        for script in sm.collection(ScriptType::Item).get_all_scripts() {
            if let Some(item) = script.downcast::<Item>() {
                let b = item.borrow();
                if b.is_item_type(sm, "CONTAINER") {
                    self.register_string(b.name());
                }
            }
        }
        for &vc in VEHICLE_CONTAINERS {
            self.register_string(vc);
        }
        for script in sm.collection(ScriptType::Vehicle).get_all_scripts() {
            if let Some(v) = script.downcast::<VehicleScript>() {
                if let Some(name) = v.borrow().name() {
                    self.register_string(name);
                }
            }
        }
        self.register_string("freezer");
        for prop in container_props {
            self.register_string(prop);
        }
        for (name, id) in tile_sprites {
            self.register_sprite(name, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_registers_zones_rooms_vehicles_freezer_and_sprites() {
        let sm = ScriptManager::new();
        let mut cfg = ItemConfigurator::new();
        cfg.preprocess(
            &sm,
            [("Town", "TownZone"), ("", "Forest")],
            ["kitchen", "bedroom", "kitchen"],
            ["fridge", "counter"],
            [("appliances_refrigeration_01_0", 4012)],
        );

        assert_eq!(cfg.get_id_for_string("Town"), 0);
        assert_eq!(cfg.get_id_for_string("TownZone"), 1);
        assert_eq!(cfg.get_id_for_string("Forest"), 2);
        assert_eq!(cfg.get_id_for_string("kitchen"), 3);
        assert_eq!(cfg.get_id_for_string("bedroom"), 4);
        assert!(cfg.get_id_for_string("TruckBed") >= 5);
        assert!(cfg.get_id_for_string("GloveBox") >= 5);
        assert!(cfg.get_id_for_string("freezer") > cfg.get_id_for_string("GloveBox"));
        assert!(cfg.get_id_for_string("fridge") > cfg.get_id_for_string("freezer"));
        assert_eq!(cfg.get_id_for_string("DoesNotExist"), -1);

        assert_eq!(cfg.get_id_for_sprite("appliances_refrigeration_01_0"), 4012);
        assert_eq!(cfg.get_id_for_sprite("missing_tile"), -1);
    }
}
