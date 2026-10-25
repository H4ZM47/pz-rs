//! `VehicleIDMap`, the cell's vehicle lists (`IsoCell.vehicles` /
//! `addVehicles`) and the single-player per-frame vehicle loop (what
//! `IsoWorld.update`, the moving-object scheduler and `IsoPlayer`'s driver
//! update do for vehicles), i.e. the offline part of `VehicleManager`.

use crate::vehicle::{BaseVehicle, VehicleRef};
use std::cell::RefCell;
use std::rc::Rc;

/// `VehicleIDMap`.
pub struct VehicleIdMap {
    id_to_vehicle: Vec<Option<VehicleRef>>,
    free_id: Vec<i16>,
}

impl Default for VehicleIdMap {
    fn default() -> Self {
        let mut m = VehicleIdMap {
            id_to_vehicle: Vec::new(),
            free_id: Vec::new(),
        };
        m.resize(256);
        m
    }
}

impl VehicleIdMap {
    fn resize(&mut self, capacity: usize) {
        let old = self.id_to_vehicle.len();
        let cap = capacity.min(i16::MAX as usize);
        self.id_to_vehicle.resize(cap, None);
        for i in old..cap {
            self.free_id.push(i as i16);
        }
    }

    /// `allocateID()`: the most recently freed id first (ids start at 255
    /// and count down).
    pub fn allocate(&mut self) -> i16 {
        if let Some(id) = self.free_id.pop() {
            return id;
        }
        if self.id_to_vehicle.len() >= i16::MAX as usize {
            return -1;
        }
        let n = self.id_to_vehicle.len() + 256;
        self.resize(n);
        self.allocate()
    }

    /// `put(id, vehicle)`.
    pub fn put(&mut self, id: i16, v: &VehicleRef) {
        assert!(
            id >= 0 && (id as usize) < self.id_to_vehicle.len(),
            "invalid vehicle id {id}"
        );
        assert!(
            self.id_to_vehicle[id as usize].is_none(),
            "duplicate vehicle with id {id}"
        );
        self.id_to_vehicle[id as usize] = Some(v.clone());
    }

    /// `remove(id)`.
    pub fn remove(&mut self, id: i16) {
        if id < 0
            || id as usize >= self.id_to_vehicle.len()
            || self.id_to_vehicle[id as usize].is_none()
        {
            return;
        }
        self.id_to_vehicle[id as usize] = None;
        self.free_id.push(id);
    }

    /// `get(id)`.
    pub fn get(&self, id: i16) -> Option<VehicleRef> {
        if id < 0 {
            return None;
        }
        self.id_to_vehicle.get(id as usize).cloned().flatten()
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        let cap = self.id_to_vehicle.len();
        self.id_to_vehicle = vec![None; cap];
        self.free_id = (0..cap as i16).collect();
    }
}

#[derive(Default)]
struct Cell {
    vehicles: Vec<VehicleRef>,
    add_vehicles: Vec<VehicleRef>,
}

thread_local! {
    static IDS: RefCell<VehicleIdMap> = RefCell::new(VehicleIdMap::default());
    static CELL: RefCell<Cell> = RefCell::new(Cell::default());
}

pub fn id_map_allocate() -> i16 {
    IDS.with(|m| m.borrow_mut().allocate())
}

pub fn id_map_put(id: i16, v: &VehicleRef) {
    IDS.with(|m| m.borrow_mut().put(id, v));
}

pub fn id_map_remove(id: i16) {
    IDS.with(|m| m.borrow_mut().remove(id));
}

/// `VehicleManager.instance.getVehicleByID(id)` (single player).
pub fn vehicle_by_id(id: i16) -> Option<VehicleRef> {
    IDS.with(|m| m.borrow().get(id))
}

/// `IsoCell.getVehicles()`, in order.
pub fn cell_vehicles() -> Vec<VehicleRef> {
    CELL.with(|c| c.borrow().vehicles.clone())
}

/// `getCell().vehicles.contains(v)`.
pub fn cell_contains(v: &BaseVehicle) -> bool {
    CELL.with(|c| {
        c.borrow()
            .vehicles
            .iter()
            .any(|x| std::ptr::eq(x.as_ref(), v))
    })
}

/// Queues a vehicle for the cell (`addVehicles`).
pub fn cell_add(v: &VehicleRef) {
    CELL.with(|c| {
        let mut c = c.borrow_mut();
        if !c.vehicles.iter().any(|x| Rc::ptr_eq(x, v))
            && !c.add_vehicles.iter().any(|x| Rc::ptr_eq(x, v))
        {
            c.add_vehicles.push(v.clone());
        }
    });
}

/// Removes a vehicle from both lists.
pub fn cell_remove(v: &BaseVehicle) {
    CELL.with(|c| {
        let mut c = c.borrow_mut();
        c.vehicles.retain(|x| !std::ptr::eq(x.as_ref(), v));
        c.add_vehicles.retain(|x| !std::ptr::eq(x.as_ref(), v));
    });
}

/// Moves `addVehicles` into `vehicles` (`IsoCell.ProcessAddVehicles`).
pub fn process_add_vehicles() {
    CELL.with(|c| {
        let mut c = c.borrow_mut();
        let add = std::mem::take(&mut c.add_vehicles);
        for v in add {
            if !c.vehicles.iter().any(|x| Rc::ptr_eq(x, &v)) {
                c.vehicles.push(v);
            }
        }
    });
}

/// Forgets every vehicle and id (a new world).
pub fn reset() {
    CELL.with(|c| *c.borrow_mut() = Cell::default());
    IDS.with(|m| *m.borrow_mut() = VehicleIdMap::default());
}

/// Adds a vehicle to the world the way a loaded chunk does
/// (`IsoChunk` vehicle list + `addToWorld`), then makes it part of the
/// cell.
pub fn add_to_world(v: &VehicleRef) {
    v.add_to_world();
    cell_add(v);
    process_add_vehicles();
}

/// One single-player frame for the vehicles: `WorldSimulation.update()`,
/// then for every vehicle the driver's `updatePhysics()` (when it has a
/// driver), `update()` and `postupdate()`.
pub fn update_frame() {
    crate::world_sim::update();
    for v in cell_vehicles() {
        if v.driver().is_some() {
            v.update_physics();
        }
        v.update();
        v.postupdate();
    }
    process_add_vehicles();
}
