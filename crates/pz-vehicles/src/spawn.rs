//! Random vehicle spawning on chunk creation: `IsoChunk.AddVehicles` and
//! its helpers (`AddVehicles_OnZone`, `AddVehicles_OnZonePolyline`, the
//! traffic jams, `spawnVehicleRandomAngle`, `RandomizeModel`,
//! `isGoodVehiclePos`, `doSpawnedVehiclesInInvalidPosition`).
//!
//! The draws from `Rand` happen in the original order. The world is seen
//! through [`SpawnWorld`]; the spawned vehicles are returned (the chunk's
//! `vehicles` list) and added to the world by the caller.
//!
//! Not ported: `AddVehicles_ForTest` (debug `TestVehicles` zones) and
//! `RandomizedVehicleStoryBase.doRandomStory` (left to
//! [`SpawnWorld::random_car_crash`]).

use crate::jmath::{clamp, fastfloor, jsqrt, Quaternionf, Vector2, Vector3f};
use crate::vehicle::{BaseVehicle, VehicleRef};
use crate::vehicle_type::{self, VehicleType};
use crate::{env, rand, world_sim};
use pz_world::IsoDirections;
use std::cell::RefCell;
use std::rc::Rc;

/// `zombie.iso.zones.VehicleZone` (pz-world's view of a meta-grid zone).
pub use pz_world::VehicleZone;

/// `Zone.getPolylineLength()`.
pub fn polyline_length(zone: &VehicleZone) -> f32 {
    if !zone.is_polyline() || zone.points.is_empty() {
        return 0.0;
    }
    let mut length = 0.0f32;
    let mut i = 0;
    while i + 2 < zone.points.len() {
        let dx = (zone.points[i + 2] - zone.points[i]) as f32;
        let dy = (zone.points[i + 3] - zone.points[i + 1]) as f32;
        length += jsqrt(dx * dx + dy * dy);
        i += 2;
    }
    length
}

/// The sandbox options `AddVehicles` reads.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnOptions {
    /// `CarSpawnRate` (1 = none .. 5 = very high).
    pub car_spawn_rate: i32,
    /// `CarAlarm` (1 = never .. 6 = very often).
    pub car_alarm: i32,
    pub traffic_jam: bool,
    pub enable_vehicles: bool,
    /// `SystemDisabler.doVehiclesEverywhere` or the debug option.
    pub vehicles_everywhere: bool,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        SpawnOptions {
            car_spawn_rate: 4,
            car_alarm: 4,
            traffic_jam: true,
            enable_vehicles: true,
            vehicles_everywhere: false,
        }
    }
}

/// The world as the spawner sees it.
pub trait SpawnWorld {
    /// The square exists (`getGridSquare(x, y, z) != null`, world coords).
    fn square_exists(&self, x: i32, y: i32, z: i32) -> bool;
    /// `IsoGridSquare.isOutside()`.
    fn is_outside(&self, x: i32, y: i32, z: i32) -> bool;
    /// `metaGrid.getVehicleZoneAt(x, y, z) != null`.
    fn vehicle_zone_at(&self, x: i32, y: i32, z: i32) -> bool;
    /// `IsoMetaCell.vehicleZones` of cell (cx, cy), in order.
    fn vehicle_zones(&self, cell_x: i32, cell_y: i32) -> Vec<VehicleZone>;
    /// The `vehicles` of a loaded chunk of the current cell other than the
    /// one being populated (`isGoodVehiclePos`).
    fn chunk_vehicles(&self, _wx: i32, _wy: i32) -> Option<Vec<VehicleRef>> {
        None
    }
    /// Whether the chunk being populated is already in the cell, so its own
    /// list counts for `isGoodVehiclePos`.
    fn own_chunk_in_cell(&self) -> bool {
        true
    }
    /// `addRandomCarCrash(zone, false)` for each zone of the meta chunk,
    /// called with the vehicles spawned so far (it does nothing when that
    /// list is not empty).
    fn random_car_crash(&self, _wx: i32, _wy: i32, _vehicles: &mut Vec<VehicleRef>) {}
}

/// A [`SpawnWorld`] on pz-world's cell, sprite tables and meta grid.
pub struct CellSpawnWorld<'a> {
    pub cell: &'a pz_world::IsoCell,
    pub sprites: &'a pz_world::IsoSpriteManager,
    pub meta: &'a pz_world::IsoMetaGrid,
}

impl SpawnWorld for CellSpawnWorld<'_> {
    fn square_exists(&self, x: i32, y: i32, z: i32) -> bool {
        self.cell.get_grid_square(x, y, z).is_some()
    }
    fn is_outside(&self, x: i32, y: i32, z: i32) -> bool {
        self.cell
            .get_grid_square(x, y, z)
            .is_some_and(|s| s.is_exterior(self.sprites))
    }
    fn vehicle_zone_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.meta.vehicle_zone_at(x, y, z).is_some()
    }
    fn vehicle_zones(&self, cell_x: i32, cell_y: i32) -> Vec<VehicleZone> {
        self.meta.cell_vehicle_zones(cell_x, cell_y)
    }
    fn chunk_vehicles(&self, wx: i32, wy: i32) -> Option<Vec<VehicleRef>> {
        crate::cell_link::chunk_vehicles(self.cell, wx, wy)
    }
}

thread_local! {
    /// `IsoChunk.BaseVehicleCheckedVehicles`.
    static CHECKED: RefCell<Vec<VehicleRef>> = const { RefCell::new(Vec::new()) };
}

/// `IsoChunk.removeFromCheckedVehicles(v)`.
pub fn remove_from_checked_vehicles(v: &BaseVehicle) {
    CHECKED.with(|c| c.borrow_mut().retain(|o| !std::ptr::eq(&**o, v)));
}

/// `IsoChunk.addFromCheckedVehicles(v)`.
pub fn add_from_checked_vehicles(v: &VehicleRef) {
    CHECKED.with(|c| {
        let mut c = c.borrow_mut();
        if !c.iter().any(|o| Rc::ptr_eq(o, v)) {
            c.push(v.clone());
        }
    });
}

/// `IsoChunk.Reset()`.
pub fn reset_checked_vehicles() {
    CHECKED.with(|c| c.borrow_mut().clear());
}

/// `IsoDirections.toAngle()`.
fn dir_angle(d: IsoDirections) -> f32 {
    d as i32 as f32 * (std::f32::consts::PI * 2.0) / 8.0
}

/// `IsoDirections.fromAngle(dx, dy)`.
fn dir_from_angle(dx: f32, dy: f32) -> IsoDirections {
    let a = (dy as f64).atan2(dx as f64) as f32;
    IsoDirections::ALL[((6 - (8.5f32 + 1.273_239_5f32 * a) as i32) & 7) as usize]
}

/// `while (angle > 2π) angle -= 2π` with the original's double compare.
fn wrap_angle(mut angle: f32) -> f32 {
    let two_pi = std::f64::consts::PI * 2.0;
    while angle as f64 > two_pi {
        angle = (angle as f64 - two_pi) as f32;
    }
    angle
}

const TWO_PI_F: f32 = std::f32::consts::PI * 2.0;

/// The `CarAlarm` chance.
fn alarm_chance(opts: &SpawnOptions) -> i32 {
    match opts.car_alarm {
        1 => -1,
        2 => 3,
        3 => 8,
        5 => 25,
        6 => 50,
        _ => 15,
    }
}

fn set_rotation(v: &BaseVehicle, angle: f32) {
    let q = Quaternionf::angle_axis(angle, 0.0, 1.0, 0.0);
    let mut st = v.st_mut();
    st.saved_rot = q;
    st.jni_transform.set_rotation(&q);
}

fn set_origin(v: &BaseVehicle) {
    let (ox, oy) = env::offset();
    let mut st = v.st_mut();
    st.jni_transform.origin = Vector3f::new(st.x - ox, st.z, st.y - oy);
}

fn set_xy(v: &BaseVehicle, x: f32, y: f32) {
    let mut st = v.st_mut();
    st.x = x;
    st.y = y;
}

fn set_z(v: &BaseVehicle, z: f32) {
    v.st_mut().z = z;
}

fn skin_count(v: &BaseVehicle) -> i32 {
    v.script().map_or(0, |s| s.borrow().skin_count() as i32)
}

/// One chunk being populated.
pub struct ChunkSpawner<'a> {
    pub wx: i32,
    pub wy: i32,
    pub world: &'a dyn SpawnWorld,
    pub options: SpawnOptions,
    /// `IsoChunk.vehicles`.
    pub vehicles: Vec<VehicleRef>,
}

impl<'a> ChunkSpawner<'a> {
    pub fn new(wx: i32, wy: i32, world: &'a dyn SpawnWorld, options: SpawnOptions) -> Self {
        ChunkSpawner {
            wx,
            wy,
            world,
            options,
            vehicles: Vec::new(),
        }
    }

    /// `getGridSquare(x, y, z)` (chunk-local): the world coordinates when it
    /// exists.
    fn square(&self, x: i32, y: i32, z: i32) -> Option<(i32, i32, i32)> {
        if !(0..8).contains(&x) || !(0..8).contains(&y) {
            return None;
        }
        let (ax, ay) = (self.wx * 8 + x, self.wy * 8 + y);
        self.world.square_exists(ax, ay, z).then_some((ax, ay, z))
    }

    /// `AddVehicles()`.
    pub fn add_vehicles(&mut self) {
        if self.options.car_spawn_rate == 1 {
            return;
        }
        if !self.options.enable_vehicles {
            return;
        }
        world_sim::create();
        let zones = self.world.vehicle_zones(self.wx / 32, self.wy / 32);
        let (wx, wy) = (self.wx, self.wy);
        for zone in &zones {
            if zone.x + zone.w < wx * 8
                || zone.y + zone.h < wy * 8
                || zone.x >= (wx + 1) * 8
                || zone.y >= (wy + 1) * 8
            {
                continue;
            }
            let name = if zone.name.is_empty() {
                zone.zone_type.clone()
            } else {
                zone.name.clone()
            };
            let rname = || name.replacen("rtraffic", "traffic", 1);
            if self.options.traffic_jam {
                if zone.is_polyline() {
                    if name.eq_ignore_ascii_case("TrafficJam") {
                        self.traffic_jam_polyline(zone, &name);
                        continue;
                    }
                    if name.eq_ignore_ascii_case("RTrafficJam") && rand::next(100) < 10 {
                        self.traffic_jam_polyline(zone, &rname());
                        continue;
                    }
                }
                for (d, n) in [
                    (IsoDirections::W, "TrafficJamW"),
                    (IsoDirections::E, "TrafficJamE"),
                    (IsoDirections::S, "TrafficJamS"),
                    (IsoDirections::N, "TrafficJamN"),
                ] {
                    if name.eq_ignore_ascii_case(n) {
                        self.traffic_jam(zone, &name, d);
                    }
                }
                for (d, n) in [
                    (IsoDirections::W, "RTrafficJamW"),
                    (IsoDirections::E, "RTrafficJamE"),
                    (IsoDirections::S, "RTrafficJamS"),
                    (IsoDirections::N, "RTrafficJamN"),
                ] {
                    if name.eq_ignore_ascii_case(n) && rand::next(100) < 10 {
                        self.traffic_jam(zone, &rname(), d);
                    }
                }
            }
            if name.to_lowercase().contains("trafficjam") {
                continue;
            }
            if name == "TestVehicles" {
                continue;
            }
            if !vehicle_type::has_type_for_zone(&name) {
                continue;
            }
            if zone.is_polyline() {
                if !self.on_zone_polyline(zone, &name) {
                    return;
                }
            } else {
                self.on_zone(zone, &name);
            }
        }
        let mut v = std::mem::take(&mut self.vehicles);
        self.world.random_car_crash(wx, wy, &mut v);
        self.vehicles = v;
    }

    /// `RandomizeModel(v, zone, name, type)`.
    pub fn randomize_model(&self, v: &BaseVehicle, t: &VehicleType) -> bool {
        if t.vehicles_definition.is_empty() {
            return false;
        }
        let r = rand::next_float(0.0, 100.0);
        let mut current = 0.0f32;
        let mut def = &t.vehicles_definition[0];
        for d in &t.vehicles_definition {
            def = d;
            current += d.spawn_chance;
            if r < current {
                break;
            }
        }
        if crate::vehicle::find_vehicle_script(&def.vehicle_type).is_none() {
            return false;
        }
        v.set_script_name(&def.vehicle_type);
        v.set_script(&def.vehicle_type);
        if def.index > -1 {
            v.set_skin_index(def.index);
        } else {
            v.set_skin_index(rand::next(skin_count(v)));
        }
        true
    }

    /// `doSpawnedVehiclesInInvalidPosition(v)`.
    pub fn valid_position(&self, v: &VehicleRef) -> bool {
        let (x, y, z) = {
            let st = v.st();
            (fastfloor(st.x), fastfloor(st.y), fastfloor(st.z))
        };
        if !self.world.square_exists(x, y, z) {
            return false;
        }
        if !self.world.vehicle_zone_at(x, y, z) && !self.world.is_outside(x, y, z) {
            return false;
        }
        let checked = CHECKED.with(|c| c.borrow().clone());
        if checked.iter().any(|o| o.test_collision_with_vehicle(v)) {
            return false;
        }
        add_from_checked_vehicles(v);
        true
    }

    /// `isGoodVehiclePos(vehicle, chunk)`.
    fn is_good_vehicle_pos(&self, v: &BaseVehicle) -> bool {
        let (x, y, z) = {
            let st = v.st();
            (st.x, st.y, st.z)
        };
        let min_x = (fastfloor(x) - 4) / 8 - 1;
        let min_y = (fastfloor(y) - 4) / 8 - 1;
        let max_x = ((x + 4.0) / 8.0).ceil() as i32 + 1;
        let max_y = ((y + 4.0) / 8.0).ceil() as i32 + 1;
        for cy in min_y..max_y {
            for cx in min_x..max_x {
                let list = if cx == self.wx && cy == self.wy && self.world.own_chunk_in_cell() {
                    Some(self.vehicles.clone())
                } else {
                    self.world.chunk_vehicles(cx, cy)
                };
                let Some(list) = list else { continue };
                for o in &list {
                    if fastfloor(o.st().z) == fastfloor(z) && v.test_collision_with_vehicle(o) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// The `CarSpawnRate`-scaled chance of a zone type.
    fn spawn_chance(&self, t: &VehicleType) -> i32 {
        let mut chance = t.spawn_rate;
        match self.options.car_spawn_rate {
            2 => chance = (chance as f32 / 10.0).ceil() as i32,
            3 => chance = (chance as f32 / 1.5).ceil() as i32,
            5 => chance *= 2,
            _ => {}
        }
        if self.options.vehicles_everywhere || t.force_spawn {
            chance = 100;
        }
        chance
    }

    /// `AddVehicles_OnZone(zone, zoneName)`.
    pub fn on_zone(&mut self, zone: &VehicleZone, zone_name: &str) {
        let mut dir = IsoDirections::N;
        let mut stall_wid = 3;
        let mut stall_len = 4;
        if (zone.w == stall_len || zone.w == stall_len + 1 || zone.w == stall_len + 2)
            && !(zone.h > stall_wid && zone.h < stall_len + 2)
        {
            dir = IsoDirections::W;
        }
        stall_len = 5;
        if let Some(d) = zone.dir {
            dir = d;
        }
        if dir != IsoDirections::N && dir != IsoDirections::S {
            stall_len = 3;
            stall_wid = 5;
        }
        let mut y_offset = (zone.y - self.wy * 8) as f32 + stall_len as f32 / 2.0;
        while y_offset < 0.0 {
            y_offset += stall_len as f32;
        }
        let mut x_offset = (zone.x - self.wx * 8) as f32 + stall_wid as f32 / 2.0;
        while x_offset < 0.0 {
            x_offset += stall_wid as f32;
        }
        let mut y = y_offset;
        'rows: while y < 8.0 && (self.wy * 8) as f32 + y < (zone.y + zone.h) as f32 {
            let mut x = x_offset;
            while x < 8.0 && (self.wx * 8) as f32 + x < (zone.x + zone.w) as f32 {
                let (fx, fy) = (fastfloor(x), fastfloor(y));
                let cur_x = x;
                x += stall_wid as f32;
                let Some(mut sq) = self.square(fx, fy, 0) else {
                    continue;
                };
                let Some(t) = vehicle_type::random_vehicle_type(zone_name, true) else {
                    y += stall_len as f32;
                    continue 'rows;
                };
                if rand::next(100) > self.spawn_chance(&t) {
                    continue;
                }
                let v = BaseVehicle::new();
                v.st_mut().respawn_zone = zone_name.to_string();
                v.set_vehicle_type(&t.name);
                if t.is_special_car {
                    v.st_mut().do_color = false;
                }
                if !self.randomize_model(&v, &t) {
                    return;
                }
                let alarm = alarm_chance(&self.options);
                let sn = v.script_name().to_lowercase();
                let wrecked = sn.contains("burnt") || sn.contains("smashed");
                if rand::next(100) < alarm && !wrecked {
                    v.set_alarmed(true);
                }
                let vdir = if zone.face_direction {
                    dir
                } else if dir == IsoDirections::N || dir == IsoDirections::S {
                    if rand::next(2) == 0 {
                        IsoDirections::N
                    } else {
                        IsoDirections::S
                    }
                } else if rand::next(2) == 0 {
                    IsoDirections::W
                } else {
                    IsoDirections::E
                };
                let mut angle = wrap_angle(dir_angle(vdir) + std::f32::consts::PI);
                if t.random_angle {
                    angle = rand::next_float(0.0, TWO_PI_F);
                }
                set_rotation(&v, angle);
                let vehicle_length = v.script().map_or(0.0, |s| s.borrow().extents().z);
                let mut vx = sq.0 as f32 + 0.5;
                let mut vy = sq.1 as f32 + 0.5;
                let half_w = stall_wid as f32 / 2.0;
                let half_l = stall_len as f32 / 2.0;
                match dir {
                    IsoDirections::N => {
                        vx = sq.0 as f32 + half_w - (half_w as i32) as f32;
                        vy = zone.y as f32 + vehicle_length / 2.0 + 0.5;
                        if vy >= (sq.1 + 1) as f32 && fy < 7 {
                            if let Some(s) = self.square(fx, fy + 1, 0) {
                                sq = s;
                            }
                        }
                    }
                    IsoDirections::S => {
                        vx = sq.0 as f32 + half_w - (half_w as i32) as f32;
                        vy = (zone.y + zone.h) as f32 - vehicle_length / 2.0 - 0.5;
                        if vy < sq.1 as f32 && fy > 0 {
                            if let Some(s) = self.square(fx, fy - 1, 0) {
                                sq = s;
                            }
                        }
                    }
                    IsoDirections::W => {
                        vx = zone.x as f32 + vehicle_length / 2.0 + 0.5;
                        vy = sq.1 as f32 + half_l - (half_l as i32) as f32;
                        if vx >= (sq.0 + 1) as f32 && fx < 7 {
                            if let Some(s) = self.square(fx + 1, fy, 0) {
                                sq = s;
                            }
                        }
                    }
                    IsoDirections::E => {
                        vx = (zone.x + zone.w) as f32 - vehicle_length / 2.0 - 0.5;
                        vy = sq.1 as f32 + half_l - (half_l as i32) as f32;
                        if vx < sq.0 as f32 && fx > 0 {
                            if let Some(s) = self.square(fx - 1, fy, 0) {
                                sq = s;
                            }
                        }
                    }
                    _ => {}
                }
                let (vx, vy) = clamp_to_square(vx, vy, sq);
                set_xy(&v, vx, vy);
                set_z(&v, sq.2 as f32);
                set_origin(&v);
                let rust_chance = 100.0 - (t.base_vehicle_quality * 120.0).min(100.0);
                v.st_mut().rust = if (rand::next(100) as f32) < rust_chance {
                    1.0
                } else {
                    0.0
                };
                if self.valid_position(&v) {
                    self.vehicles.push(v);
                }
                if t.chance_of_over_car <= 0 || rand::next(100) > t.chance_of_over_car {
                    continue;
                }
                let _ = cur_x;
                self.spawn_vehicle_random_angle(sq, zone, zone_name);
            }
            y += stall_len as f32;
        }
    }

    /// `AddVehicles_OnZonePolyline(zone, zoneName)`. `false` when the
    /// original would throw (a spawn point on a missing square), which
    /// ends `AddVehicles`.
    pub fn on_zone_polyline(&mut self, zone: &VehicleZone, zone_name: &str) -> bool {
        let n = zone.points.len();
        let mut i = 0;
        while i + 2 < n {
            let x1 = zone.points[i];
            let y1 = zone.points[i + 1];
            let x2 = zone.points[(i + 2) % n];
            let y2 = zone.points[(i + 3) % n];
            i += 2;
            let mut v2 = Vector2::new((x2 - x1) as f32, (y2 - y1) as f32);
            let mut d = 2.5f32;
            while d < v2.length() {
                let len = v2.length();
                let mut vx = x1 as f32 + v2.x / len * d;
                let mut vy = y1 as f32 + v2.y / len * d;
                d += 5.0;
                if !(vx >= (self.wx * 8) as f32)
                    || !(vy >= (self.wy * 8) as f32)
                    || !(vx < ((self.wx + 1) * 8) as f32)
                    || !(vy < ((self.wy + 1) * 8) as f32)
                {
                    continue;
                }
                let Some(t) = vehicle_type::random_vehicle_type(zone_name, true) else {
                    return true;
                };
                let v = BaseVehicle::new();
                v.st_mut().respawn_zone = zone_name.to_string();
                v.set_vehicle_type(&t.name);
                if t.is_special_car {
                    v.st_mut().do_color = false;
                }
                if !self.randomize_model(&v, &t) {
                    return true;
                }
                if rand::next(100) < alarm_chance(&self.options) {
                    v.set_alarmed(true);
                }
                let (ox, oy) = (v2.x, v2.y);
                v2.normalize();
                let _dir = dir_from_angle(v2.x, v2.y);
                let mut angle = wrap_angle((v2.x as f64).atan2(v2.y as f64) as f32 + 0.0);
                v2 = Vector2::new(ox, oy);
                if t.random_angle {
                    angle = rand::next_float(0.0, TWO_PI_F);
                }
                set_rotation(&v, angle);
                // The original dereferences the square (an exception ends
                // the zone).
                let Some(sq) =
                    self.square(fastfloor(vx) - self.wx * 8, fastfloor(vy) - self.wy * 8, 0)
                else {
                    return false;
                };
                (vx, vy) = clamp_to_square(vx, vy, sq);
                set_xy(&v, vx, vy);
                set_z(&v, sq.2 as f32);
                set_origin(&v);
                let rust_chance = 100.0 - (t.base_vehicle_quality * 120.0).min(100.0);
                v.st_mut().rust = if (rand::next(100) as f32) < rust_chance {
                    1.0
                } else {
                    0.0
                };
                if self.valid_position(&v) {
                    self.vehicles.push(v);
                }
            }
        }
        true
    }

    /// `spawnVehicleRandomAngle(sq, zone, zoneName)`.
    fn spawn_vehicle_random_angle(
        &mut self,
        sq: (i32, i32, i32),
        zone: &VehicleZone,
        zone_name: &str,
    ) {
        let mut north = true;
        let mut stall_wid = 3;
        let mut stall_len = 4;
        if (zone.w == stall_len || zone.w == stall_len + 1 || zone.w == stall_len + 2)
            && !(zone.h > stall_wid && zone.h < stall_len + 2)
        {
            north = false;
        }
        stall_len = 5;
        if !north {
            stall_len = 3;
            stall_wid = 5;
        }
        let Some(t) = vehicle_type::random_vehicle_type(zone_name, true) else {
            return;
        };
        let v = BaseVehicle::new();
        v.st_mut().respawn_zone = zone_name.to_string();
        if !self.randomize_model(&v, &t) {
            return;
        }
        // setDir: N/S or W/E (the direction is not used further).
        rand::next(2);
        let angle = rand::next_float(0.0, TWO_PI_F);
        set_rotation(&v, angle);
        if north {
            let h = stall_wid as f32 / 2.0;
            set_xy(&v, sq.0 as f32 + h - (h as i32) as f32, sq.1 as f32);
        } else {
            let h = stall_len as f32 / 2.0;
            set_xy(&v, sq.0 as f32, sq.1 as f32 + h - (h as i32) as f32);
        }
        set_z(&v, sq.2 as f32);
        set_origin(&v);
        if self.valid_position(&v) {
            self.vehicles.push(v);
        }
    }

    /// `AddVehicles_TrafficJam_W/E/S/N(zone, zoneName)`.
    pub fn traffic_jam(&mut self, zone: &VehicleZone, zone_name: &str, dir: IsoDirections) {
        let horizontal = matches!(dir, IsoDirections::W | IsoDirections::E);
        let (y_start, y_step, x_start, x_step) = if horizontal {
            (1, 3, 3, 6)
        } else {
            (3, 6, 1, 3)
        };
        let mut y_offset = zone.y - self.wy * 8 + y_start;
        while y_offset < 0 {
            y_offset += y_step;
        }
        let mut x_offset = zone.x - self.wx * 8 + x_start;
        while x_offset < 0 {
            x_offset += x_step;
        }
        let row_step = |_: ()| {
            if horizontal {
                3 + rand::next(1)
            } else {
                6 + rand::next_range(-1, 1)
            }
        };
        let col_step = |_: ()| {
            if horizontal {
                6 + rand::next(1)
            } else {
                3 + rand::next(1)
            }
        };
        let mut y = y_offset;
        'rows: while y < 8 && self.wy * 8 + y < zone.y + zone.h {
            let mut x = x_offset;
            while x < 8 && self.wx * 8 + x < zone.x + zone.w {
                let (lx, ly) = (x, y);
                macro_rules! next_col {
                    () => {{
                        x += col_step(());
                        continue;
                    }};
                }
                let Some(sq) = self.square(lx, ly, 0) else {
                    next_col!()
                };
                let Some(t) = vehicle_type::random_vehicle_type(zone_name, true) else {
                    y += row_step(());
                    continue 'rows;
                };
                let chance = if self.options.vehicles_everywhere {
                    100
                } else {
                    80
                };
                if rand::next(100) > chance {
                    next_col!()
                }
                let v = BaseVehicle::new();
                v.st_mut().respawn_zone = "TrafficJam".to_string();
                v.set_vehicle_type(&t.name);
                if !self.randomize_model(&v, &t) {
                    return;
                }
                let sname = v.script_name();
                v.set_script(&sname);
                let px = sq.0 as f32 + rand::next_float(0.0, 1.0);
                let py = sq.1 as f32 + rand::next_float(0.0, 1.0);
                set_xy(&v, px, py);
                set_z(&v, sq.2 as f32);
                set_origin(&v);
                if !self.is_good_vehicle_pos(&v) {
                    next_col!()
                }
                v.set_skin_index(rand::next(skin_count(&v) - 1));
                let dist = match dir {
                    IsoDirections::W => (zone.x + zone.w - sq.0).abs(),
                    IsoDirections::E => (zone.x + zone.w - sq.0 - zone.w).abs(),
                    IsoDirections::S => (zone.y + zone.h - sq.1 - zone.h).abs(),
                    _ => (zone.y + zone.h - sq.1).abs(),
                } as f32;
                let dist = (dist / 20.0).min(2.0);
                let angle =
                    dir_angle(dir) + std::f32::consts::PI - 0.25 + rand::next_float(0.0, dist);
                set_rotation(&v, wrap_angle(angle));
                if self.valid_position(&v) {
                    self.vehicles.push(v);
                }
                next_col!()
            }
            y += row_step(());
        }
    }

    /// `AddVehicles_TrafficJam_Polyline(zone, zoneName)`.
    pub fn traffic_jam_polyline(&mut self, zone: &VehicleZone, zone_name: &str) {
        let zone_length = polyline_length(zone);
        let mut to_segment_start = 0.0f32;
        let half_width = zone.polyline_width as f32 / 2.0;
        let mut i = 0;
        while i + 2 < zone.points.len() {
            let x1 = zone.points[i];
            let y1 = zone.points[i + 1];
            let x2 = zone.points[i + 2];
            let y2 = zone.points[i + 3];
            i += 2;
            let v2 = Vector2::new((x2 - x1) as f32, (y2 - y1) as f32);
            let len = v2.length();
            let mut perp = tangent(v2);
            perp.normalize();
            let to_spawn = to_segment_start;
            to_segment_start += len;
            let mut d = 3.0f32;
            while d <= len - 3.0 {
                let mut offset = clamp(d + rand::next_float(-1.0, 1.0), 3.0, len - 3.0);
                let mut perp_off = rand::next_float(-1.0, 1.0);
                let sx = x1 as f32 + v2.x / len * offset + perp.x * perp_off;
                let sy = y1 as f32 + v2.y / len * offset + perp.y * perp_off;
                self.try_add_traffic_jam(zone_name, sx, sy, v2, to_spawn + offset, zone_length);
                let mut dp = 2.0f32;
                while dp + 1.5 <= half_width {
                    perp_off = dp + rand::next_float(-1.0, 1.0);
                    if perp_off + 1.5 <= half_width {
                        offset = clamp(d + rand::next_float(-2.0, 2.0), 3.0, len - 3.0);
                        let sx = x1 as f32 + v2.x / len * offset + perp.x * perp_off;
                        let sy = y1 as f32 + v2.y / len * offset + perp.y * perp_off;
                        self.try_add_traffic_jam(
                            zone_name,
                            sx,
                            sy,
                            v2,
                            to_spawn + offset,
                            zone_length,
                        );
                    }
                    perp_off = dp + rand::next_float(-1.0, 1.0);
                    if perp_off + 1.5 <= half_width {
                        offset = clamp(d + rand::next_float(-2.0, 2.0), 3.0, len - 3.0);
                        let sx = x1 as f32 + v2.x / len * offset - perp.x * perp_off;
                        let sy = y1 as f32 + v2.y / len * offset - perp.y * perp_off;
                        self.try_add_traffic_jam(
                            zone_name,
                            sx,
                            sy,
                            v2,
                            to_spawn + offset,
                            zone_length,
                        );
                    }
                    dp += 2.0;
                }
                d += (6 + rand::next_range(-1, 1)) as f32;
            }
        }
    }

    /// `TryAddVehicle_TrafficJam(...)`.
    fn try_add_traffic_jam(
        &mut self,
        zone_name: &str,
        sx: f32,
        sy: f32,
        v2: Vector2,
        dist_to_spawn: f32,
        zone_length: f32,
    ) {
        if sx < (self.wx * 8) as f32
            || sx >= ((self.wx + 1) * 8) as f32
            || sy < (self.wy * 8) as f32
            || sy >= ((self.wy + 1) * 8) as f32
        {
            return;
        }
        let Some(sq) = self.square(fastfloor(sx) - self.wx * 8, fastfloor(sy) - self.wy * 8, 0)
        else {
            return;
        };
        let Some(t) = vehicle_type::random_vehicle_type(&format!("{zone_name}W"), true) else {
            return;
        };
        let chance = if self.options.vehicles_everywhere {
            100
        } else {
            80
        };
        if rand::next(100) > chance {
            return;
        }
        let v = BaseVehicle::new();
        v.st_mut().respawn_zone = "TrafficJam".to_string();
        v.set_vehicle_type(&t.name);
        if !self.randomize_model(&v, &t) {
            return;
        }
        let sname = v.script_name();
        v.set_script(&sname);
        set_xy(&v, sx, sy);
        set_z(&v, sq.2 as f32);
        let mut n = v2;
        n.normalize();
        let mut angle = (n.x as f64).atan2(n.y as f64) as f32;
        let r = 90.0f32 * (dist_to_spawn / zone_length);
        angle += rand::next_float(-r, r) * (std::f32::consts::PI / 180.0);
        set_rotation(&v, wrap_angle(angle));
        set_origin(&v);
        if self.is_good_vehicle_pos(&v) {
            v.set_skin_index(rand::next(skin_count(&v) - 1));
            if self.valid_position(&v) {
                self.vehicles.push(v);
            }
        }
    }
}

/// Keeps (vx, vy) 0.005 inside the square.
fn clamp_to_square(mut vx: f32, mut vy: f32, sq: (i32, i32, i32)) -> (f32, f32) {
    if vx < sq.0 as f32 + 0.005 {
        vx = sq.0 as f32 + 0.005;
    }
    if vx > (sq.0 + 1) as f32 - 0.005 {
        vx = (sq.0 + 1) as f32 - 0.005;
    }
    if vy < sq.1 as f32 + 0.005 {
        vy = sq.1 as f32 + 0.005;
    }
    if vy > (sq.1 + 1) as f32 - 0.005 {
        vy = (sq.1 + 1) as f32 - 0.005;
    }
    (vx, vy)
}

/// `Vector2.tangent()` (rotation by 90 degrees in double precision).
fn tangent(v: Vector2) -> Vector2 {
    let r = 90.0f64.to_radians();
    let (c, s) = (r.cos(), r.sin());
    Vector2::new(
        (v.x as f64 * c - v.y as f64 * s) as f32,
        (v.x as f64 * s + v.y as f64 * c) as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions() {
        assert_eq!(dir_from_angle(1.0, 0.0), IsoDirections::E);
        assert_eq!(dir_from_angle(0.0, -1.0), IsoDirections::N);
        assert_eq!(dir_from_angle(0.0, 1.0), IsoDirections::S);
        assert_eq!(dir_from_angle(-1.0, 0.0), IsoDirections::W);
        assert_eq!(dir_angle(IsoDirections::W), std::f32::consts::PI / 2.0);
    }

    #[test]
    fn tangent_rotates() {
        let t = tangent(Vector2::new(1.0, 0.0));
        assert_eq!(t.y, 1.0);
        assert!(t.x.abs() < 1e-6);
    }

    #[test]
    fn polyline_length() {
        let z = VehicleZone {
            name: String::new(),
            zone_type: String::new(),
            x: 0,
            y: 0,
            z: 0,
            w: 0,
            h: 0,
            dir: None,
            face_direction: false,
            geometry_type: pz_world::ZoneGeometryType::Polyline,
            points: vec![0, 0, 3, 4, 3, 10],
            polyline_width: 0,
        };
        assert_eq!(super::polyline_length(&z), 11.0);
    }
}
