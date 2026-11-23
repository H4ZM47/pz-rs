//! `zombie.inventory.ItemPickerJava$KeyNamer` (global `KeyNamer`) as
//! `media/lua/server/Items/KeyNamer.lua` uses it: `KeyNamer.clear()` and
//! `KeyNamer.<list>:add(name)` on its six public static `ArrayList`s.
//! [`key_namer`] gives the lists to `pz_inventory::key_namer::KeyNamer`
//! (`getName` / `nameKey`), which take the square as an input.

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;
use pz_inventory::key_namer::{KeyNamer, KeySquare};
use pz_kahlua::{Thread, Value};
use std::rc::Rc;

const CLASS: &str = "zombie.inventory.ItemPickerJava$KeyNamer";

/// The static fields, in declaration order.
const FIELDS: [&str; 6] = [
    "badZones",
    "bigBuildingRooms",
    "restaurantSubstrings",
    "restaurants",
    "roomSubstrings",
    "rooms",
];

thread_local! {
    static LISTS: [Rc<JavaList>; 6] = Default::default();
}

impl Default for JavaList {
    fn default() -> Self {
        JavaList::new(Vec::new())
    }
}

/// The lists' strings (`ArrayList.contains` of a `String` only matches
/// strings).
pub fn key_namer() -> KeyNamer {
    LISTS.with(|l| {
        let s = |i: usize| -> Vec<String> {
            l[i].snapshot()
                .into_iter()
                .filter_map(|v| match v {
                    Value::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect()
        };
        KeyNamer {
            bad_zones: s(0),
            big_building_rooms: s(1),
            restaurant_substrings: s(2),
            restaurants: s(3),
            room_substrings: s(4),
            rooms: s(5),
        }
    })
}

/// [`KeySquare`] over the loaded world's `IsoCell`, `IsoMetaGrid`, and
/// `Translator` at `(x, y, z)`.
pub struct WorldKeySquare {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl WorldKeySquare {
    fn building_idx(&self) -> Option<usize> {
        crate::bind_world::with_world(|w| {
            let meta = w.meta_grid.as_ref()?;
            let cell = w.cell.as_ref()?;
            let sq = cell.get_grid_square(self.x, self.y, self.z)?;
            let sq = sq.try_borrow().ok()?;
            let room = sq.room_def(meta)?;
            room.building.map(|b| b.0 as usize)
        })
    }
}

impl KeySquare for WorldKeySquare {
    fn has_building(&self) -> bool {
        self.building_idx().is_some()
    }

    fn zombies_type(&self) -> Option<String> {
        crate::bind_world::with_world(|w| {
            let meta = w.meta_grid.as_ref()?;
            for id in meta.get_zones_at(self.x, self.y, 0) {
                let Some(z) = meta.zone_defs.get(id.0 as usize) else {
                    continue;
                };
                if z.zone_type == "ZombiesType" && !z.name.is_empty() {
                    return Some(z.name.clone());
                }
            }
            None
        })
    }

    fn building_name_zones(&self) -> Vec<String> {
        crate::bind_world::with_world(|w| {
            let Some(meta) = w.meta_grid.as_ref() else {
                return Vec::new();
            };
            meta.get_zones_at(self.x, self.y, 0)
                .into_iter()
                .filter_map(|id| meta.zone_defs.get(id.0 as usize))
                .filter(|z| z.zone_type == "BuildingName" && !z.name.is_empty())
                .map(|z| z.name.clone())
                .collect()
        })
    }

    fn building_contains_room(&self, room: &str) -> bool {
        let Some(b_idx) = self.building_idx() else {
            return false;
        };
        crate::bind_world::with_world(|w| {
            let Some(meta) = w.meta_grid.as_ref() else {
                return false;
            };
            let Some(b) = meta.building_defs.get(b_idx) else {
                return false;
            };
            b.rooms.iter().any(|r_idx| {
                meta.room_defs
                    .get(r_idx.0 as usize)
                    .is_some_and(|r| r.name == room)
            })
        })
    }

    fn room_def_name(&self) -> Option<String> {
        crate::bind_world::with_world(|w| {
            let meta = w.meta_grid.as_ref()?;
            let cell = w.cell.as_ref()?;
            let sq = cell.get_grid_square(self.x, self.y, self.z)?;
            let sq = sq.try_borrow().ok()?;
            sq.room_def(meta).map(|r| r.name.clone())
        })
    }

    fn has_zone(&self, zone: &str) -> bool {
        crate::bind_world::with_world(|w| {
            let Some(meta) = w.meta_grid.as_ref() else {
                return false;
            };
            meta.get_zones_at(self.x, self.y, 0)
                .into_iter()
                .filter_map(|id| meta.zone_defs.get(id.0 as usize))
                .any(|z| z.name == zone || z.zone_type == zone)
        })
    }

    fn has_translation(&self, key: &str) -> bool {
        crate::bind_text::with_translator(|t| t.get_text_or_null(key, &[]).is_some())
            .unwrap_or(false)
    }
}

/// `ItemPickerJava.KeyNamer.nameKey(item, square)` over `(x, y, z)`:
/// names the key `<displayName> - <translated IGUI_<name>Key>` and marks
/// `custom_name = true` when a matching translation exists.
pub fn name_key_at(item: &mut pz_inventory::InventoryItem, x: i32, y: i32, z: i32) -> bool {
    let sq = WorldKeySquare { x, y, z };
    let Some(key) = key_namer().key_name_translation(&sq) else {
        return false;
    };
    let suffix = crate::bind_text::translate(&key);
    let base = item.name.clone();
    item.set_name(&format!("{base} - {suffix}"));
    item.custom_name = true;
    true
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    LISTS.with(|l| {
        for (name, list) in FIELDS.iter().zip(l.iter()) {
            b.field(name, Value::Object(list.clone()));
        }
    });
    b.static_fn("clear", |_| {
        LISTS.with(|l| {
            for list in l {
                list.items.borrow_mut().clear();
            }
        });
        Ok(0)
    });
    b.static_fn("getName", |c| {
        let sq = c.arg(0);
        if sq.is_nil() {
            return Ok(c.push(Value::Nil));
        }
        let Some((x, y, z)) = crate::bind_world::square_coords(&sq) else {
            return Ok(c.push(Value::Nil));
        };
        let wsq = WorldKeySquare { x, y, z };
        let name = key_namer()
            .get_name(&wsq)
            .map(Value::from)
            .unwrap_or(Value::Nil);
        Ok(c.push(name))
    });
    b.static_fn("nameKey", |c| {
        let item_val = c.arg(0);
        let sq = c.arg(1);
        if item_val.is_nil() || sq.is_nil() {
            return Ok(0);
        }
        let (Some(item), Some((x, y, z))) = (
            crate::bind_items::item_arg_value(&item_val),
            crate::bind_world::square_coords(&sq),
        ) else {
            return Ok(0);
        };
        if let Ok(mut body) = item.try_borrow_mut() {
            name_key_at(&mut body, x, y, z);
        }
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn lists_from_lua() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);
        let src = r#"
            -- assert normally comes from the installed stdlib.lua.
            local function assert(c) if not c then error("assertion failed") end end
            KeyNamer.badZones:add("Stale")
            KeyNamer.clear()
            KeyNamer.badZones:add("Rich")
            KeyNamer.bigBuildingRooms:add("motelroom")
            KeyNamer.rooms:add("bakery")
            KeyNamer.roomSubstrings:add("office")
            KeyNamer.restaurants:add("diner")
            KeyNamer.restaurantSubstrings:add("pizza")
            assert(KeyNamer.badZones:size() == 1)
        "#;
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).unwrap();
        let k = key_namer();
        assert_eq!(k.bad_zones, ["Rich"]);
        assert_eq!(k.big_building_rooms, ["motelroom"]);
        assert_eq!(k.rooms, ["bakery"]);
        assert_eq!(k.room_substrings, ["office"]);
        assert_eq!(k.restaurants, ["diner"]);
        assert_eq!(k.restaurant_substrings, ["pizza"]);
    }
}
