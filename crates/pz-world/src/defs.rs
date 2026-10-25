//! `RoomDef`, `BuildingDef`, `MetaObject`, `RoomID`, `BuildingID`.
//!
//! Rooms and buildings reference each other in the original; here they live
//! in arenas owned by [`crate::IsoMetaGrid`] and refer to each other by index.

use crate::java::fastfloor;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RoomIdx(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BuildingIdx(pub u32);

/// `RoomID`.
pub mod room_id {
    pub fn make_id(cell_x: i32, cell_y: i32, room_index: i32) -> i64 {
        let hi = (cell_y << 16) | cell_x;
        ((hi as i64) << 32) | room_index as i64
    }
    pub fn get_cell_x(id: i64) -> i32 {
        ((id >> 32) as i32) & 0xFFFF
    }
    pub fn get_cell_y(id: i64) -> i32 {
        (((id >> 32) as i32) >> 16) & 0xFFFF
    }
    pub fn get_index(id: i64) -> i32 {
        (id & 0xFFFF_FFFF) as i32
    }
    pub fn is_same_cell(id: i64, cell_x: i32, cell_y: i32) -> bool {
        get_cell_x(id) == cell_x && get_cell_y(id) == cell_y
    }
}

/// `BuildingID` (same packing as `RoomID`).
pub mod building_id {
    pub fn make_id(cell_x: i32, cell_y: i32, building_index: i32) -> i64 {
        let hi = cell_x | (cell_y << 16);
        ((hi as i64) << 32) | building_index as i64
    }
    pub use super::room_id::{get_cell_x, get_cell_y, get_index, is_same_cell};
}

/// `RoomDef.RoomRect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl RoomRect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        RoomRect { x, y, w, h }
    }
    pub fn get_x2(&self) -> i32 {
        self.x + self.w
    }
    pub fn get_y2(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32
            && y >= self.y as f32
            && x < self.get_x2() as f32
            && y < self.get_y2() as f32
    }
}

/// `MetaObject`: an object marker stored in a lot header room
/// (coordinates relative to the room's top-left).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaObject {
    pub kind: i32,
    pub x: i32,
    pub y: i32,
    pub used: bool,
}

/// `RoomDef` (map-data fields).
#[derive(Clone, Debug)]
pub struct RoomDef {
    pub explored: bool,
    pub done_spawn: bool,
    pub indoor_zombies: i32,
    pub spawn_count: i32,
    /// `lightsActive`; light switches clear it while the map loads.
    pub lights_active: std::cell::Cell<bool>,
    pub name: String,
    pub level: i32,
    pub building: Option<BuildingIdx>,
    pub id: i64,
    pub rects: Vec<RoomRect>,
    pub objects: Vec<MetaObject>,
    pub x: i32,
    pub y: i32,
    pub x2: i32,
    pub y2: i32,
    pub area: i32,
    pub roof_fixed: bool,
    pub meta_id: i64,
    pub user_defined: bool,
}

impl RoomDef {
    pub fn new(id: i64, name: String) -> Self {
        RoomDef {
            explored: false,
            done_spawn: false,
            indoor_zombies: 0,
            spawn_count: -1,
            lights_active: std::cell::Cell::new(false),
            name,
            level: 0,
            building: None,
            id,
            rects: Vec::with_capacity(1),
            objects: Vec::new(),
            x: 100000,
            y: 100000,
            x2: -10000,
            y2: -10000,
            area: 0,
            roof_fixed: false,
            meta_id: 0,
            user_defined: false,
        }
    }

    #[allow(non_snake_case)]
    pub fn CalculateBounds(&mut self) {
        self.x = 10000000;
        self.y = 10000000;
        self.x2 = -1000000;
        self.y2 = -1000000;
        self.area = 0;
        for rect in &self.rects {
            if rect.x < self.x {
                self.x = rect.x;
            }
            if rect.y < self.y {
                self.y = rect.y;
            }
            if rect.x + rect.w > self.x2 {
                self.x2 = rect.x + rect.w;
            }
            if rect.y + rect.h > self.y2 {
                self.y2 = rect.y + rect.h;
            }
            self.area += rect.w * rect.h;
        }
    }

    pub fn calculate_meta_id(&self, cell_x: i32, cell_y: i32) -> i64 {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        for rect in &self.rects {
            if rect.x > min_x || rect.y >= min_y {
                continue;
            }
            min_x = rect.x;
            min_y = rect.y;
        }
        let min_y = min_y.wrapping_sub(cell_y.wrapping_mul(256));
        let min_x = min_x.wrapping_sub(cell_x.wrapping_mul(256));
        ((self.level as i64) << 32) | ((min_y as i64) << 16) | min_x as i64
    }

    pub fn is_empty_outside(&self) -> bool {
        self.name.eq_ignore_ascii_case("emptyoutside")
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.rects.iter().any(|r| r.contains(x as f32, y as f32))
    }

    pub fn intersects(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.rects
            .iter()
            .any(|r| !(x + w <= r.x || x >= r.get_x2() || y + h <= r.y || y >= r.get_y2()))
    }

    /// `isInside(x, y, z)` (the building origin is read but unused upstream).
    pub fn is_inside(&self, x: i32, y: i32, z: i32) -> bool {
        self.rects
            .iter()
            .any(|r| !(x < r.x || y < r.y || x >= r.get_x2() || y >= r.get_y2() || z != self.level))
    }

    pub fn get_area_overlapping(&self, x: i32, y: i32, w: i32, h: i32) -> f32 {
        let mut room_area = 0;
        let mut overlap_area = 0;
        for r in &self.rects {
            room_area += r.w * r.h;
            let x1 = x.max(r.x);
            let y1 = y.max(r.y);
            let x2 = (x + w).min(r.x + r.w);
            let y2 = (y + h).min(r.y + r.h);
            if x2 < x1 || y2 < y1 {
                continue;
            }
            overlap_area += (x2 - x1) * (y2 - y1);
        }
        if overlap_area <= 0 {
            return 0.0;
        }
        overlap_area as f32 / room_area as f32
    }

    /// `isAdjacent(RoomDef other)`: some rect of `other`, grown by one tile
    /// on every side, intersects this room.
    pub fn is_adjacent(&self, other: &RoomDef) -> bool {
        other
            .rects
            .iter()
            .any(|r| self.intersects(r.x - 1, r.y - 1, r.w + 2, r.h + 2))
    }

    /// `isAdjacent(x, y, w, h)`.
    pub fn is_adjacent_rect(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.intersects(x - 1, y - 1, w + 2, h + 2)
    }

    /// `overlaps(RoomDef other)`.
    pub fn overlaps(&self, other: &RoomDef) -> bool {
        other
            .rects
            .iter()
            .any(|r| self.get_area_overlapping(r.x, r.y, r.w, r.h) > 0.0)
    }

    /// `getRoomRect(x, y, z)`: inclusive of the far edges, like the original.
    pub fn get_room_rect(&self, x: i32, y: i32) -> Option<&RoomRect> {
        self.rects
            .iter()
            .find(|r| !(r.x > x || r.y > y || r.get_x2() < x || r.get_y2() < y))
    }
}

/// `BuildingDef` (map-data fields).
#[derive(Clone, Debug)]
pub struct BuildingDef {
    pub emptyoutside: Vec<RoomIdx>,
    pub seen: bool,
    pub has_been_visited: bool,
    pub loot_respawn_hour: i32,
    /// Pairs of `(wx, wy)` stored as shorts.
    pub overlapped_chunks: Option<Vec<i16>>,
    pub alarmed: bool,
    pub alarm_decay: i32,
    pub x: i32,
    pub y: i32,
    pub x2: i32,
    pub y2: i32,
    pub rooms: Vec<RoomIdx>,
    pub food: i32,
    pub id: i64,
    pub key_spawned: i32,
    pub key_id: i32,
    pub meta_id: i64,
    min_level: i32,
    max_level: i32,
    pub user_defined: bool,
}

impl BuildingDef {
    /// `new BuildingDef()`: the constructor draws the key id from `Rand`.
    pub fn new(rand: &mut dyn crate::GameRand) -> Self {
        BuildingDef {
            emptyoutside: Vec::new(),
            seen: false,
            has_been_visited: false,
            loot_respawn_hour: -1,
            overlapped_chunks: None,
            alarmed: false,
            alarm_decay: 10000000,
            x: 10000000,
            y: 10000000,
            x2: -10000000,
            y2: -10000000,
            rooms: Vec::new(),
            food: 0,
            id: 0,
            key_spawned: 0,
            key_id: rand.next(100000000),
            meta_id: 0,
            min_level: 100,
            max_level: -100,
            user_defined: false,
        }
    }

    pub fn get_min_level(&mut self, rooms: &[RoomDef]) -> i32 {
        if self.min_level != 100 {
            return self.min_level;
        }
        for r in &self.rooms {
            self.min_level = rooms[r.0 as usize].level.min(self.min_level);
        }
        self.min_level
    }

    /// `getMaxLevel()` without caching the result.
    pub fn max_level(&self, rooms: &[RoomDef]) -> i32 {
        if self.max_level != -100 {
            return self.max_level;
        }
        self.rooms
            .iter()
            .fold(self.max_level, |m, r| rooms[r.0 as usize].level.max(m))
    }

    pub fn get_max_level(&mut self, rooms: &[RoomDef]) -> i32 {
        if self.max_level != -100 {
            return self.max_level;
        }
        for r in &self.rooms {
            self.max_level = rooms[r.0 as usize].level.max(self.max_level);
        }
        self.max_level
    }

    /// `resetMinMaxLevel()`.
    pub fn reset_min_max_level(&mut self) {
        self.min_level = 100;
        self.max_level = -100;
    }

    /// `getW()`.
    pub fn get_w(&self) -> i32 {
        self.x2 - self.x
    }

    /// `getH()`.
    pub fn get_h(&self) -> i32 {
        self.y2 - self.y
    }

    /// `intersects(x, y, w, h, z)`: a room (not empty-outside) on level `z`
    /// intersects the rectangle.
    pub fn intersects(&self, rooms: &[RoomDef], x: i32, y: i32, w: i32, h: i32, z: i32) -> bool {
        self.rooms.iter().any(|r| {
            let room = &rooms[r.0 as usize];
            room.level == z && room.intersects(x, y, w, h)
        })
    }

    /// `isAdjacent(x, y, w, h, z)`.
    #[allow(clippy::too_many_arguments)]
    pub fn is_adjacent_rect(
        &self,
        rooms: &[RoomDef],
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        z: i32,
    ) -> bool {
        self.rooms.iter().any(|r| {
            let room = &rooms[r.0 as usize];
            room.level == z && room.is_adjacent_rect(x, y, w, h)
        })
    }

    /// `isAdjacent(BuildingDef other)`: `isAdjacent(other, false)`.
    pub fn is_adjacent(&self, rooms: &[RoomDef], other: &BuildingDef) -> bool {
        self.is_adjacent_ignore_z(rooms, other, false)
    }

    /// `isAdjacent(BuildingDef other, boolean bIgnoreZ)`: rooms only, not
    /// empty-outside rooms.
    pub fn is_adjacent_ignore_z(
        &self,
        rooms: &[RoomDef],
        other: &BuildingDef,
        ignore_z: bool,
    ) -> bool {
        self.rooms.iter().any(|a| {
            let a = &rooms[a.0 as usize];
            other.rooms.iter().any(|b| {
                let b = &rooms[b.0 as usize];
                (ignore_z || a.level == b.level) && a.is_adjacent(b)
            })
        })
    }

    /// `overlaps(BuildingDef other, boolean bIgnoreZ)`.
    pub fn overlaps(&self, rooms: &[RoomDef], other: &BuildingDef, ignore_z: bool) -> bool {
        self.rooms.iter().any(|a| {
            let a = &rooms[a.0 as usize];
            other.rooms.iter().any(|b| {
                let b = &rooms[b.0 as usize];
                (ignore_z || a.level == b.level) && a.overlaps(b)
            })
        })
    }

    /// `addRoomsOf(sourceDef, tempRooms)`: moves nothing out of `source`
    /// (its lists are left as they are, like the original); its rooms and
    /// empty-outside rooms are appended here and re-pointed at `this`, then
    /// the levels are reset and the bounds recalculated. `this` is the
    /// arena index of `self`.
    pub fn add_rooms_of(
        &mut self,
        this: BuildingIdx,
        source: &mut BuildingDef,
        rooms: &mut [RoomDef],
    ) {
        for &r in &source.rooms {
            rooms[r.0 as usize].building = Some(this);
            self.rooms.push(r);
        }
        for &r in &source.emptyoutside {
            rooms[r.0 as usize].building = Some(this);
            self.emptyoutside.push(r);
        }
        self.reset_min_max_level();
        self.CalculateBounds(rooms);
        source.reset_min_max_level();
    }

    /// `isEntirelyEmptyOutside()`.
    pub fn is_entirely_empty_outside(&self) -> bool {
        self.rooms.is_empty() && !self.emptyoutside.is_empty()
    }

    #[allow(non_snake_case)]
    pub fn CalculateBounds(&mut self, rooms: &[RoomDef]) {
        self.x = i32::MAX;
        self.y = i32::MAX;
        self.x2 = i32::MIN;
        self.y2 = i32::MIN;
        for list in [&self.rooms, &self.emptyoutside] {
            for r in list.iter() {
                for rect in &rooms[r.0 as usize].rects {
                    if rect.x < self.x {
                        self.x = rect.x;
                    }
                    if rect.y < self.y {
                        self.y = rect.y;
                    }
                    if rect.x + rect.w > self.x2 {
                        self.x2 = rect.x + rect.w;
                    }
                    if rect.y + rect.h > self.y2 {
                        self.y2 = rect.y + rect.h;
                    }
                }
            }
        }
        let min_x = fastfloor(self.x as f32 / 8.0);
        let min_y = fastfloor(self.y as f32 / 8.0);
        let max_x = fastfloor(self.x2 as f32 / 8.0);
        let max_y = fastfloor(self.y2 as f32 / 8.0);
        let mut overlapped = self.overlapped_chunks.take().unwrap_or_else(|| {
            let cap = (max_x - min_x + 1)
                .saturating_mul(max_y - min_y + 1)
                .saturating_mul(2);
            Vec::with_capacity(cap.clamp(0, 4096) as usize)
        });
        overlapped.clear();
        for r in self.rooms.iter().chain(self.emptyoutside.iter()) {
            for rect in &rooms[r.0 as usize].rects {
                // Integer division truncates toward zero, as in Java.
                let min_x = (rect.x - 1) / 8;
                let min_y = (rect.y - 1) / 8;
                let max_x = (rect.x + rect.w) / 8;
                let max_y = (rect.y + rect.h) / 8;
                for wy in min_y..=max_y {
                    for wx in min_x..=max_x {
                        if overlaps(&overlapped, wx, wy) {
                            continue;
                        }
                        overlapped.push(wx as i16);
                        overlapped.push(wy as i16);
                    }
                }
            }
        }
        overlapped.shrink_to_fit();
        self.overlapped_chunks = Some(overlapped);
    }

    pub fn overlaps_chunk(&self, wx: i32, wy: i32) -> bool {
        self.overlapped_chunks
            .as_ref()
            .is_some_and(|v| overlaps(v, wx, wy))
    }

    pub fn calculate_meta_id(&self, rooms: &[RoomDef], cell_x: i32, cell_y: i32) -> i64 {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut min_z = i32::MAX;
        let list = if self.rooms.is_empty() {
            &self.emptyoutside
        } else {
            &self.rooms
        };
        for r in list {
            let room = &rooms[r.0 as usize];
            if room.level > min_z {
                continue;
            }
            if room.level < min_z {
                min_x = i32::MAX;
                min_y = i32::MAX;
            }
            min_z = room.level;
            for rect in &room.rects {
                if rect.x > min_x || rect.y >= min_y {
                    continue;
                }
                min_x = rect.x;
                min_y = rect.y;
            }
        }
        let min_y = min_y.wrapping_sub(cell_y.wrapping_mul(256));
        let min_x = min_x.wrapping_sub(cell_x.wrapping_mul(256));
        ((min_z as i64) << 32) | ((min_y as i64) << 16) | min_x as i64
    }
}

fn overlaps(v: &[i16], wx: i32, wy: i32) -> bool {
    v.as_chunks::<2>()
        .0
        .iter()
        .any(|p| wx == p[0] as i32 && wy == p[1] as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let id = room_id::make_id(45, 33, 7);
        assert_eq!(room_id::get_cell_x(id), 45);
        assert_eq!(room_id::get_cell_y(id), 33);
        assert_eq!(room_id::get_index(id), 7);
        assert_eq!(building_id::make_id(45, 33, 7), id);
    }

    #[test]
    fn room_meta_id_sign_extends() {
        let mut r = RoomDef::new(0, "kitchen".into());
        r.level = -1;
        r.rects.push(RoomRect::new(10 * 256 + 5, 2 * 256 + 3, 2, 2));
        let id = r.calculate_meta_id(10, 2);
        // (long)-1 << 32 | 3 << 16 | 5
        assert_eq!(id, (-1i64 << 32) | (3 << 16) | 5);
    }
}
