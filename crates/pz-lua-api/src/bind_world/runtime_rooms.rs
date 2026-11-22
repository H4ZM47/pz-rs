//! Runtime IsoRoom and IsoBuilding queries used by procedural loot.
use super::*;

pub(super) fn square_room(x: i32, y: i32, z: i32) -> LuaResult<Option<RoomIdx>> {
    let id = square_room_id(x, y, z)?;
    if id == -1 {
        return Ok(None);
    }
    grid(|g| Ok(g.runtime_room_index(id, pz_core::random::next)))
}

pub(super) fn building(room: RoomIdx) -> LuaResult<Value> {
    grid(|g| Ok(opt(g.room_defs[room.0 as usize].building.map(Key::IsoBuilding))))
}

fn room_arg(c: &NativeCall<'_>) -> LuaResult<RoomIdx> {
    match key_of(&c.arg(0))? {
        Key::IsoRoom(room) => Ok(room),
        _ => Err(wrong(c, 0, "zombie.iso.areas.IsoRoom")),
    }
}

fn building_arg(c: &NativeCall<'_>) -> LuaResult<BuildingIdx> {
    match key_of(&c.arg(0))? {
        Key::IsoBuilding(building) => Ok(building),
        _ => Err(wrong(c, 0, "zombie.iso.areas.IsoBuilding")),
    }
}

pub(super) fn register(thread: &mut Thread) {
    let room = expose_class(thread, "zombie.iso.areas.IsoRoom", Some("java.lang.Object"));
    get(&room, "getBuilding", |c| building(room_arg(c)?));
    get(&room, "getRoomDef", |c| Ok(obj(Key::Room(room_arg(c)?.0))));
    get(&room, "getName", |c| {
        let room = room_arg(c)?;
        grid(|g| Ok(Value::str(&g.room_defs[room.0 as usize].name)))
    });

    let building = expose_class(thread, "zombie.iso.areas.IsoBuilding", Some("java.lang.Object"));
    get(&building, "getDef", |c| Ok(obj(Key::Building(building_arg(c)?.0))));
    get(&building, "getRandomRoom", |c| {
        let building = building_arg(c)?;
        let named = c.nargs() > 1;
        let name = str_arg(c, 1);
        grid(|g| {
            g.with_runtime_building_rooms(building, |rooms| {
                if named && name.is_none() && !rooms.is_empty() {
                    return Err(LuaError::runtime("java.lang.NullPointerException"));
                }
                let matching;
                let candidates = if let Some(name) = name.as_deref() {
                    matching = rooms.iter().copied()
                        .filter(|r| g.room_defs[r.0 as usize].name == name)
                        .collect::<Vec<_>>();
                    matching.as_slice()
                } else {
                    rooms
                };
                if candidates.is_empty() {
                    return Ok(Value::Nil);
                }
                let index = pz_core::random::next(candidates.len() as i32) as usize;
                Ok(obj(Key::IsoRoom(candidates[index])))
            }).ok_or_else(|| LuaError::runtime("IsoBuilding is no longer loaded"))?
        })
    });
}
