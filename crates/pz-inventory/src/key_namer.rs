//! `ItemPickerJava.KeyNamer`: the room and zone lists
//! `media/lua/server/Items/KeyNamer.lua` fills, and `getName(square)` /
//! `nameKey(item, square)`, which name a key after the building it was
//! found in.

/// What `getName` reads from the square, its building and the world.
pub trait KeySquare {
    /// `square.getBuilding() != null`.
    fn has_building(&self) -> bool;
    /// `square.getSquareZombiesType()`.
    fn zombies_type(&self) -> Option<String>;
    /// The `name` of each zone at the square (z 0) whose type is
    /// `BuildingName` (`LuaManager.GlobalObject.getZones`), in order.
    fn building_name_zones(&self) -> Vec<String>;
    /// `building.containsRoom(name)`.
    fn building_contains_room(&self, room: &str) -> bool;
    /// `square.getRoom().getRoomDef().getName()`.
    fn room_def_name(&self) -> Option<String>;
    /// `ItemPickerJava.squareHasZone(square, zone)`: a meta-grid zone at
    /// the square (z 0) with that name or type.
    fn has_zone(&self, zone: &str) -> bool;
    /// `Translator.getTextOrNull(key) != null`.
    fn has_translation(&self, key: &str) -> bool;
}

/// `ItemPickerJava.KeyLootType`.
pub const KEY_LOOT_TYPE: &str = "Key";

/// The `KeyNamer` lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyNamer {
    pub bad_zones: Vec<String>,
    pub big_building_rooms: Vec<String>,
    pub restaurant_substrings: Vec<String>,
    pub restaurants: Vec<String>,
    pub room_substrings: Vec<String>,
    pub rooms: Vec<String>,
}

impl KeyNamer {
    /// `clear()`.
    pub fn clear(&mut self) {
        *self = KeyNamer::default();
    }

    fn bad(&self, zone: Option<&str>) -> bool {
        zone.is_some_and(|z| self.bad_zones.iter().any(|b| b == z))
    }

    /// `ItemPickerJava.getSquareBuildingName(square)`.
    pub fn square_building_name(&self, sq: &dyn KeySquare) -> Option<String> {
        sq.building_name_zones().into_iter().find(|n| !self.bad(Some(n)))
    }

    /// `getName(square)`.
    pub fn get_name(&self, sq: &dyn KeySquare) -> Option<String> {
        if !sq.has_building() {
            return None;
        }
        let mut zone = sq.zombies_type();
        if self.bad(zone.as_deref()) {
            zone = None;
        }
        if let Some(name) = self.square_building_name(sq) {
            return Some(name);
        }
        if sq.building_contains_room("bedroom") && sq.building_contains_room("livingroom") && sq.building_contains_room("kitchen") {
            return Some("Residential".into());
        }
        let mut key_name = match zone.as_deref() {
            Some(z @ ("Prison" | "Police" | "Army")) => Some(z.to_string()),
            _ => None,
        };
        if self.bad(key_name.as_deref()) {
            key_name = None;
        }
        if key_name.is_some() {
            return key_name;
        }
        for name in &self.big_building_rooms {
            if name == "storageunit" && sq.building_contains_room("bedroom") {
                break;
            }
            if sq.building_contains_room(name) {
                return Some(name.clone());
            }
        }
        if let Some(t) = sq.zombies_type() {
            if !self.bad(Some(&t)) && sq.has_translation(&format!("IGUI_{t}{KEY_LOOT_TYPE}")) {
                return Some(t);
            }
        }
        if let Some(room) = sq.room_def_name() {
            if self.rooms.contains(&room) {
                return Some(room);
            }
            if let Some(n) = self.room_substrings.iter().find(|n| room.contains(n.as_str())) {
                return Some(n.clone());
            }
            if self.restaurants.contains(&room) {
                return Some(room);
            }
            if let Some(n) = self.restaurant_substrings.iter().find(|n| room.contains(n.as_str())) {
                return Some(n.clone());
            }
        }
        if sq.has_zone("TrailerPark") && sq.building_contains_room("bedroom") {
            return Some("TrailerPark".into());
        }
        if sq.has_zone("Ranch") && sq.building_contains_room("bedroom") {
            return Some("Ranch".into());
        }
        if sq.has_zone("Forest") {
            return Some("Forest".into());
        }
        if sq.has_zone("DeepForest") {
            return Some("DeepForest".into());
        }
        None
    }

    /// `nameKey(item, square)`'s translation key: `IGUI_<name>Key` when it
    /// exists. The item is then named `getText(displayName) + " - " +
    /// getText(key)` (and its origin set to the square).
    pub fn key_name_translation(&self, sq: &dyn KeySquare) -> Option<String> {
        let key = format!("IGUI_{}{KEY_LOOT_TYPE}", self.get_name(sq)?);
        sq.has_translation(&key).then_some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Sq {
        building: bool,
        zombies: Option<String>,
        building_names: Vec<String>,
        rooms: Vec<&'static str>,
        room_def: Option<String>,
        zones: Vec<&'static str>,
        texts: Vec<String>,
    }

    impl KeySquare for Sq {
        fn has_building(&self) -> bool {
            self.building
        }
        fn zombies_type(&self) -> Option<String> {
            self.zombies.clone()
        }
        fn building_name_zones(&self) -> Vec<String> {
            self.building_names.clone()
        }
        fn building_contains_room(&self, room: &str) -> bool {
            self.rooms.contains(&room)
        }
        fn room_def_name(&self) -> Option<String> {
            self.room_def.clone()
        }
        fn has_zone(&self, zone: &str) -> bool {
            self.zones.contains(&zone)
        }
        fn has_translation(&self, key: &str) -> bool {
            self.texts.iter().any(|t| t == key)
        }
    }

    fn namer() -> KeyNamer {
        KeyNamer {
            bad_zones: vec!["Rich".into(), "Rocker".into()],
            big_building_rooms: vec!["motelroom".into(), "storageunit".into(), "church".into()],
            rooms: vec!["bakery".into()],
            room_substrings: vec!["office".into()],
            restaurants: vec!["pizzawhirled".into()],
            restaurant_substrings: vec!["diner".into()],
        }
    }

    #[test]
    fn names() {
        let k = namer();
        assert_eq!(k.get_name(&Sq::default()), None);
        let sq = |f: &dyn Fn(&mut Sq)| {
            let mut s = Sq {
                building: true,
                ..Default::default()
            };
            f(&mut s);
            s
        };
        assert_eq!(k.get_name(&sq(&|s| s.building_names = vec!["Rocker".into(), "Spiffo".into()])).as_deref(), Some("Spiffo"));
        assert_eq!(k.get_name(&sq(&|s| s.rooms = vec!["bedroom", "livingroom", "kitchen"])).as_deref(), Some("Residential"));
        assert_eq!(k.get_name(&sq(&|s| s.zombies = Some("Police".into()))).as_deref(), Some("Police"));
        // `storageunit` with a bedroom stops the big-building search.
        assert_eq!(k.get_name(&sq(&|s| s.rooms = vec!["bedroom", "storageunit", "church"])), None);
        assert_eq!(k.get_name(&sq(&|s| s.rooms = vec!["storageunit", "church"])).as_deref(), Some("storageunit"));
        assert_eq!(
            k.get_name(&sq(&|s| {
                s.zombies = Some("Farm".into());
                s.texts = vec!["IGUI_FarmKey".into()];
            }))
            .as_deref(),
            Some("Farm")
        );
        assert_eq!(k.get_name(&sq(&|s| s.room_def = Some("dentistoffice".into()))).as_deref(), Some("office"));
        assert_eq!(k.get_name(&sq(&|s| s.room_def = Some("bakery".into()))).as_deref(), Some("bakery"));
        assert_eq!(k.get_name(&sq(&|s| s.room_def = Some("dinerkitchen".into()))).as_deref(), Some("diner"));
        assert_eq!(k.get_name(&sq(&|s| s.zones = vec!["Forest", "DeepForest"])).as_deref(), Some("Forest"));
        assert_eq!(k.get_name(&sq(&|s| s.zones = vec!["Ranch"])), None);
        let s = sq(&|s| {
            s.room_def = Some("bakery".into());
            s.texts = vec!["IGUI_bakeryKey".into()];
        });
        assert_eq!(k.key_name_translation(&s).as_deref(), Some("IGUI_bakeryKey"));
    }
}
