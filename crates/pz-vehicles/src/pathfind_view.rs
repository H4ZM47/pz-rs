//! What pz-pathfind reads from vehicles (`GameVehicle`, `VehicleSource`,
//! `VehicleScripts`), as plain functions over [`BaseVehicle`] and the
//! cell's vehicle list, plus [`pathfind_adapter!`] which implements the
//! pz-pathfind traits on top of them in the crate that depends on both.
//!
//! The adapter is a macro so this crate does not depend on pz-pathfind:
//! `pz_vehicles::pathfind_adapter!(pz_pathfind);` defines
//! `PathfindVehicle` (a `GameVehicle`) and `PathfindVehicles` (a
//! `VehicleSource` and `VehicleScripts`) in the calling module.

use crate::manager;
use crate::vehicle::{BaseVehicle, VehicleRef};

/// A vehicle polygon as pz-pathfind's `VehiclePoly` holds it (corners and
/// level, no transform).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PolyCorners {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub x3: f32,
    pub y3: f32,
    pub x4: f32,
    pub y4: f32,
    pub z: f32,
}

impl From<crate::poly::VehiclePoly> for PolyCorners {
    fn from(p: crate::poly::VehiclePoly) -> Self {
        PolyCorners {
            x1: p.x1,
            y1: p.y1,
            x2: p.x2,
            y2: p.y2,
            x3: p.x3,
            y3: p.y3,
            x4: p.x4,
            y4: p.y4,
            z: p.z,
        }
    }
}

impl BaseVehicle {
    /// `getScript().getCrawlOffsets()` (empty without a script).
    pub fn script_crawl_offsets(&self) -> Vec<f32> {
        self.script()
            .map(|s| s.borrow().crawl_offsets().to_vec())
            .unwrap_or_default()
    }

    /// `getScript().getExtents()`.
    pub fn script_extents_xyz(&self) -> (f32, f32, f32) {
        self.script().map_or((0.0, 0.0, 0.0), |s| {
            let e = s.borrow().extents();
            (e.x, e.y, e.z)
        })
    }

    /// `getScript().getCenterOfMassOffset()`.
    pub fn script_center_of_mass_offset(&self) -> (f32, f32, f32) {
        self.script().map_or((0.0, 0.0, 0.0), |s| {
            let c = s.borrow().center_of_mass_offset();
            (c.x, c.y, c.z)
        })
    }

    /// `jniTransform.origin.y`.
    pub fn transform_origin_y(&self) -> f32 {
        self.st().jni_transform.origin.y
    }

    /// `getPassengerDoor(seat).getArea()`.
    pub fn passenger_door_area(&self, seat: i32) -> Option<String> {
        self.passenger_door(seat).and_then(|p| p.area())
    }

    /// `areaPositionWorld4PlayerInteract(getScript().getAreaById(id))`;
    /// `None` without that area.
    pub fn area_position_world_4_player_interact_by_id(&self, id: &str) -> Option<(f32, f32)> {
        let a = self.area(id)?;
        let v = self.area_position_world_4_player_interact(&a);
        Some((v.x, v.y))
    }

    /// `getPassengerPositionWorldPos(getPassengerPosition(seat, id))`.
    pub fn passenger_position_world_pos_by_id(
        &self,
        seat: i32,
        id: &str,
    ) -> Option<(f32, f32, f32)> {
        let p = self.passenger_position(seat, id)?;
        let w = self.passenger_position_world_pos(&p);
        Some((w.x, w.y, w.z))
    }
}

/// `IsoWorld.instance.currentCell.getVehicles()`, in order.
pub fn cell_vehicles() -> Vec<VehicleRef> {
    manager::cell_vehicles()
}

/// `chunk.vehicles` of chunk `(wx, wy)` (through
/// [`VehicleWorld::chunk_vehicles`](crate::env::VehicleWorld::chunk_vehicles));
/// empty when the chunk is not loaded.
pub fn chunk_vehicles(wx: i32, wy: i32) -> Vec<VehicleRef> {
    crate::env::world()
        .chunk_vehicles(wx, wy)
        .unwrap_or_default()
}

/// A registered vehicle by `vehicleId`.
pub fn vehicle(id: i16) -> Option<VehicleRef> {
    manager::vehicle_by_id(id)
}

/// Implements pz-pathfind's vehicle traits over this crate in the calling
/// module. The argument is the path of the pz-pathfind crate.
///
/// `chooseBestAttackPosition` goes through the target vehicle's
/// `SurroundVehicle`, with the attacker's floor and death state from
/// `CharacterView`; `VirtualZombieManager.isReused` is not on the mover and
/// counts as false (the vehicle's own `update` reads zombies through
/// `VehicleWorld::surround_zombie`).
#[macro_export]
macro_rules! pathfind_adapter {
    ($pf:path) => {
        use $pf as __pz_pf;

        fn __pz_pf_poly(p: $crate::pathfind_view::PolyCorners) -> __pz_pf::visibility::VehiclePoly {
            __pz_pf::visibility::VehiclePoly {
                x1: p.x1,
                y1: p.y1,
                x2: p.x2,
                y2: p.y2,
                x3: p.x3,
                y3: p.y3,
                x4: p.x4,
                y4: p.y4,
                z: p.z,
            }
        }

        /// A `BaseVehicle` as pz-pathfind sees it.
        pub struct PathfindVehicle(pub $crate::VehicleRef);

        impl __pz_pf::game_vehicle::GameVehicle for PathfindVehicle {
            fn vehicle_id(&self) -> i16 {
                self.0.st().vehicle_id
            }
            fn poly(&self) -> __pz_pf::visibility::VehiclePoly {
                __pz_pf_poly(self.0.poly().into())
            }
            fn poly_plus_radius(&self) -> __pz_pf::visibility::VehiclePoly {
                __pz_pf_poly(self.0.poly_plus_radius().into())
            }
            fn up_vector_dot(&self) -> f32 {
                self.0.up_vector_dot()
            }
            fn crawl_offsets(&self) -> Vec<f32> {
                self.0.script_crawl_offsets()
            }
            fn z(&self) -> f32 {
                self.0.z()
            }
            fn added_to_world(&self) -> bool {
                self.0.st().added_to_world
            }
            fn has_script(&self) -> bool {
                self.0.script().is_some()
            }
        }

        /// The cell's vehicles, for pz-pathfind.
        pub struct PathfindVehicles;

        impl __pz_pf::game_vehicle::VehicleSource for PathfindVehicles {
            fn for_each_vehicle(&self, f: &mut dyn FnMut(&dyn __pz_pf::game_vehicle::GameVehicle)) {
                for v in $crate::pathfind_view::cell_vehicles() {
                    f(&PathfindVehicle(v));
                }
            }
            fn for_each_vehicle_in_chunk(
                &self,
                wx: i32,
                wy: i32,
                f: &mut dyn FnMut(&dyn __pz_pf::game_vehicle::GameVehicle) -> bool,
            ) {
                for v in $crate::pathfind_view::chunk_vehicles(wx, wy) {
                    if f(&PathfindVehicle(v)) {
                        break;
                    }
                }
            }
        }

        impl __pz_pf::behavior::hooks::VehicleScripts for PathfindVehicles {
            fn position(&self, id: i16) -> (f32, f32, f32) {
                $crate::pathfind_view::vehicle(id)
                    .map_or((0.0, 0.0, 0.0), |v| (v.x(), v.y(), v.z()))
            }
            fn current_speed_km_hour(&self, id: i16) -> f32 {
                $crate::pathfind_view::vehicle(id).map_or(0.0, |v| v.current_speed_kmh())
            }
            fn closest_point_on_poly_dist_sq(&self, id: i16, x: f32, y: f32) -> f32 {
                $crate::pathfind_view::vehicle(id)
                    .map_or(f32::MAX, |v| v.closest_point_on_poly(x, y).0)
            }
            fn choose_best_attack_position(
                &self,
                id: i16,
                target: &dyn __pz_pf::mover::CharacterView,
                attacker: &dyn __pz_pf::mover::PathMover,
            ) -> Option<(f32, f32, f32)> {
                let v = $crate::pathfind_view::vehicle(id)?;
                let tv = $crate::pathfind_view::vehicle(target.vehicle()?)?;
                let z = $crate::surround::SurroundZombie {
                    key: attacker.mover_id().0,
                    x: attacker.x(),
                    y: attacker.y(),
                    z: attacker.z(),
                    on_floor: attacker.is_on_floor(),
                    can_walk: attacker.as_zombie().map_or(true, |z| z.is_can_walk()),
                    dead: attacker.is_dead(),
                    // `VirtualZombieManager.isReused` is not on the mover.
                    reused: false,
                    target: Some($crate::surround::SurroundTarget {
                        x: target.x(),
                        y: target.y(),
                        seat: 0,
                    }),
                };
                v.choose_best_attack_position(&tv, &z, attacker.is_animal())
            }
            fn extents(&self, id: i16) -> (f32, f32, f32) {
                $crate::pathfind_view::vehicle(id)
                    .map_or((0.0, 0.0, 0.0), |v| v.script_extents_xyz())
            }
            fn center_of_mass_offset(&self, id: i16) -> (f32, f32, f32) {
                $crate::pathfind_view::vehicle(id)
                    .map_or((0.0, 0.0, 0.0), |v| v.script_center_of_mass_offset())
            }
            fn world_pos(&self, id: i16, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
                $crate::pathfind_view::vehicle(id).map_or((x, y, z), |v| {
                    let w = v.world_pos(x, y, z);
                    (w.x, w.y, w.z)
                })
            }
            fn area_center(&self, id: i16, area: &str) -> Option<(f32, f32)> {
                let v = $crate::pathfind_view::vehicle(id)?;
                v.area_center(area).map(|c| (c.x, c.y))
            }
            fn passenger_position(
                &self,
                id: i16,
                seat: i32,
                pid: &str,
            ) -> Option<__pz_pf::behavior::hooks::PassengerPosition> {
                let v = $crate::pathfind_view::vehicle(id)?;
                v.passenger_position(seat, pid)
                    .map(|p| __pz_pf::behavior::hooks::PassengerPosition { area: p.area })
            }
            fn passenger_position_world_pos(
                &self,
                id: i16,
                seat: i32,
                pid: &str,
            ) -> (f32, f32, f32) {
                $crate::pathfind_view::vehicle(id)
                    .and_then(|v| v.passenger_position_world_pos_by_id(seat, pid))
                    .unwrap_or((0.0, 0.0, 0.0))
            }
            fn area_position_world_4_player_interact(&self, id: i16, area: &str) -> (f32, f32) {
                $crate::pathfind_view::vehicle(id)
                    .and_then(|v| v.area_position_world_4_player_interact_by_id(area))
                    .unwrap_or((0.0, 0.0))
            }
            fn transform_origin_y(&self, id: i16) -> f32 {
                $crate::pathfind_view::vehicle(id).map_or(0.0, |v| v.transform_origin_y())
            }
            fn passenger_door_area(&self, id: i16, seat: i32) -> Option<String> {
                $crate::pathfind_view::vehicle(id)?.passenger_door_area(seat)
            }
        }
    };
}
