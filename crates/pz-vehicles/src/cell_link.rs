//! `IsoChunk.vehicles` on pz-world's cell: the runtime's vehicles as
//! [`ChunkVehicles`] handles, kept current the way `BaseVehicle` keeps its
//! chunk's list (`addToWorld`, the chunk change in `update`,
//! `permanentlyRemove`).
//!
//! A [`VehicleWorld`](crate::env::VehicleWorld) backed by an [`IsoCell`]
//! forwards its chunk hooks to these functions.

use crate::vehicle::{BaseVehicle, VehicleRef};
use pz_world::chunk_vehicles::{ChunkVehicles, LoadedVehicle, VehicleHandle, VehicleOrigin};
use pz_world::IsoCell;

/// The list handle of a runtime vehicle (same allocation, so identity
/// comparisons hold).
pub fn handle(v: &VehicleRef) -> VehicleHandle {
    v.clone() as VehicleHandle
}

/// The runtime vehicles of chunk `(wx, wy)`, in list order; `None` when the
/// chunk is not in the cell.
pub fn chunk_vehicles(cell: &IsoCell, wx: i32, wy: i32) -> Option<Vec<VehicleRef>> {
    cell.chunk_vehicles(wx, wy)
        .map(|l: &ChunkVehicles| l.of_type::<BaseVehicle>().collect())
}

/// `if (!chunk.vehicles.contains(this)) chunk.vehicles.add(this)`
/// (`addToWorld`).
pub fn add_to_chunk(cell: &mut IsoCell, v: &VehicleRef, chunk: (i32, i32)) -> bool {
    let h = handle(v);
    match cell.chunks.get_mut(&chunk) {
        Some(c) if !c.vehicles.contains(&h) => {
            c.vehicles.add(h);
            true
        }
        _ => false,
    }
}

/// The chunk change in `update`: off the old chunk's list, onto the new.
pub fn update_chunk(cell: &mut IsoCell, v: &VehicleRef, from: (i32, i32), to: (i32, i32)) -> bool {
    cell.update_vehicle_chunk(&handle(v), from, to)
}

/// `chunk.vehicles.remove(this)`.
pub fn remove_from_chunk(cell: &mut IsoCell, v: &VehicleRef, chunk: (i32, i32)) -> bool {
    cell.remove_vehicle_from_chunk(&handle(v), chunk)
}

/// Replaces the vehicles chunk loading put in `(wx, wy)`'s list with
/// runtime vehicles: `make` builds one from each [`LoadedVehicle`] (for
/// example [`crate::save::from_save`]); entries it declines stay as they
/// are. A vehicle from a `vehicles.db` row takes the row's id as its
/// `sqlId` (`WorldStreamerThread.vehicleLoaded`). Returns the vehicles
/// made, in list order.
pub fn adopt_loaded(
    cell: &mut IsoCell,
    wx: i32,
    wy: i32,
    mut make: impl FnMut(&LoadedVehicle) -> Option<VehicleRef>,
) -> Vec<VehicleRef> {
    let mut out = Vec::new();
    let Some(c) = cell.chunks.get_mut(&(wx, wy)) else {
        return out;
    };
    for i in 0..c.vehicles.len() {
        let loaded = c
            .vehicles
            .get(i)
            .and_then(|h| h.clone().downcast::<LoadedVehicle>().ok());
        let Some(lv) = loaded else { continue };
        if let Some(v) = make(&lv) {
            {
                let mut st = v.st_mut();
                st.chunk = Some((wx, wy));
                // `vehicle.setSquare(sq)`, `vehicle.setCurrent(sq)`.
                st.square = lv.square;
                st.current = lv.square;
                if let VehicleOrigin::Database { sql_id } = lv.origin {
                    st.sql_id = sql_id;
                }
            }
            c.vehicles.replace(i, handle(&v));
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::IsoChunk;
    use std::rc::Rc;

    #[test]
    fn chunk_lists_follow_the_vehicle() {
        let mut cell = IsoCell::default();
        cell.chunks.insert((1, 1), IsoChunk::new(1, 1));
        cell.chunks.insert((2, 1), IsoChunk::new(2, 1));
        let v = BaseVehicle::new();
        assert!(add_to_chunk(&mut cell, &v, (1, 1)));
        assert!(!add_to_chunk(&mut cell, &v, (1, 1)));
        assert_eq!(chunk_vehicles(&cell, 1, 1).unwrap().len(), 1);
        assert!(update_chunk(&mut cell, &v, (1, 1), (2, 1)));
        assert!(chunk_vehicles(&cell, 1, 1).unwrap().is_empty());
        let in_2 = chunk_vehicles(&cell, 2, 1).unwrap();
        assert!(Rc::ptr_eq(&in_2[0], &v));
        assert!(remove_from_chunk(&mut cell, &v, (2, 1)));
        assert!(chunk_vehicles(&cell, 3, 3).is_none());
    }
}
