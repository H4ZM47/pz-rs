//! `IsoChunk.vehicles`: the vehicles standing in a chunk.
//!
//! The list holds shared handles ([`VehicleHandle`]) compared by identity,
//! as the original's `ArrayList<BaseVehicle>`. Chunk loading fills it with
//! [`LoadedVehicle`]s (the chunk file's vehicle list, or `vehicles.db` rows
//! through [`crate::IsoChunk::vehicle_loaded`]); the vehicle runtime swaps
//! in its own objects when it adds them to the world and keeps the list
//! current as vehicles move between chunks ([`crate::IsoCell::update_vehicle_chunk`])
//! or leave the world.

use std::any::Any;
use std::rc::Rc;

use crate::save::vehicle::BaseVehicle;

/// A vehicle in a chunk's list. Two handles are the same vehicle when they
/// point at the same allocation ([`Rc::ptr_eq`]).
pub type VehicleHandle = Rc<dyn Any>;

/// Where a [`LoadedVehicle`] was read from.
#[derive(Clone, Debug, PartialEq)]
pub enum VehicleOrigin {
    /// The chunk file's vehicle list, with the square bytes it was saved
    /// with (chunk-local).
    ChunkFile { x: i8, y: i8, z: i8 },
    /// A `vehicles.db` row (`sqlId`).
    Database { sql_id: i32 },
}

/// A vehicle as chunk loading put it in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedVehicle {
    pub origin: VehicleOrigin,
    /// The square it was given (`square`/`current`), in world coordinates;
    /// `None` when the chunk has no square there.
    pub square: Option<(i32, i32, i32)>,
    pub vehicle: BaseVehicle,
}

/// `IsoChunk.vehicles`.
#[derive(Clone, Default)]
pub struct ChunkVehicles(Vec<VehicleHandle>);

impl std::fmt::Debug for ChunkVehicles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|h| Rc::as_ptr(h) as *const ()))
            .finish()
    }
}

impl PartialEq for ChunkVehicles {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().zip(&other.0).all(|(a, b)| Rc::ptr_eq(a, b))
    }
}

impl ChunkVehicles {
    /// `size()`.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `get(i)`.
    pub fn get(&self, i: usize) -> Option<&VehicleHandle> {
        self.0.get(i)
    }

    /// The handles in list order.
    pub fn iter(&self) -> impl Iterator<Item = &VehicleHandle> {
        self.0.iter()
    }

    /// `add(v)` (appends, even when present, as `ArrayList.add`).
    pub fn add(&mut self, v: VehicleHandle) {
        self.0.push(v);
    }

    /// `contains(v)`.
    pub fn contains(&self, v: &VehicleHandle) -> bool {
        self.0.iter().any(|h| Rc::ptr_eq(h, v))
    }

    /// `remove(v)`: the first occurrence; whether it was there.
    pub fn remove(&mut self, v: &VehicleHandle) -> bool {
        match self.0.iter().position(|h| Rc::ptr_eq(h, v)) {
            Some(i) => {
                self.0.remove(i);
                true
            }
            None => false,
        }
    }

    /// `remove(i)`.
    pub fn remove_at(&mut self, i: usize) -> VehicleHandle {
        self.0.remove(i)
    }

    /// Puts `v` in place of entry `i` (the runtime taking over a loaded
    /// vehicle at the same list position).
    pub fn replace(&mut self, i: usize, v: VehicleHandle) -> VehicleHandle {
        std::mem::replace(&mut self.0[i], v)
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// The entries that are `T`, in list order.
    pub fn of_type<T: 'static>(&self) -> impl Iterator<Item = Rc<T>> + '_ {
        self.0.iter().filter_map(|h| h.clone().downcast::<T>().ok())
    }

    /// The entries still as chunk loading left them.
    pub fn loaded(&self) -> impl Iterator<Item = Rc<LoadedVehicle>> + '_ {
        self.of_type::<LoadedVehicle>()
    }
}
