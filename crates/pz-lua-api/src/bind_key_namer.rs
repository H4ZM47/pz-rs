//! `zombie.inventory.ItemPickerJava$KeyNamer` (global `KeyNamer`) as
//! `media/lua/server/Items/KeyNamer.lua` uses it: `KeyNamer.clear()` and
//! `KeyNamer.<list>:add(name)` on its six public static `ArrayList`s.
//! [`key_namer`] gives the lists to `pz_inventory::key_namer::KeyNamer`
//! (`getName` / `nameKey`), which take the square as an input.

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;
use pz_inventory::key_namer::KeyNamer;
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
