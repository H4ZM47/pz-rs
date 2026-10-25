//! What the Java pathfinding layer reads from `zombie.vehicles.BaseVehicle`
//! (vehicles are not ported yet), and where it finds them.

use crate::visibility::{Vehicle, VehiclePoly};

/// A `BaseVehicle` as the main-thread pathfinding code sees it.
///
/// Polygons are given the way the Java object holds them: `z` is the game
/// level (no +32 offset). [`GameVehicle::native_vehicle`] adds the offset for
/// the native side.
pub trait GameVehicle {
    /// `BaseVehicle.vehicleId` (-1 while not registered).
    fn vehicle_id(&self) -> i16;
    /// `BaseVehicle.getPoly()`.
    fn poly(&self) -> VehiclePoly;
    /// `BaseVehicle.getPolyPlusRadius()`.
    fn poly_plus_radius(&self) -> VehiclePoly;
    /// `BaseVehicle.getUpVectorDot()`.
    fn up_vector_dot(&self) -> f32;
    /// `BaseVehicle.getScript().getCrawlOffsets()`.
    fn crawl_offsets(&self) -> Vec<f32>;
    /// `BaseVehicle.getZ()`.
    fn z(&self) -> f32;
    /// `BaseVehicle.addedToWorld`.
    fn added_to_world(&self) -> bool;
    /// `BaseVehicle.getScript() != null`.
    fn has_script(&self) -> bool;

    /// The record `VehicleAddTask.init` / `VehicleUpdateTask.init` encode
    /// for the native side (`vehicleId`, both polygons with `z + 32`,
    /// `upVectorDot`, crawl offsets).
    fn native_vehicle(&self) -> Vehicle {
        let shift = |mut p: VehiclePoly| {
            p.z += 32.0;
            p
        };
        Vehicle {
            id: self.vehicle_id() as i32,
            poly: shift(self.poly()),
            poly_plus_radius: shift(self.poly_plus_radius()),
            up_vector_dot: self.up_vector_dot(),
            crawl_offsets: self.crawl_offsets(),
        }
    }
}

/// Where the main thread finds vehicles.
pub trait VehicleSource {
    /// `IsoWorld.instance.currentCell.getVehicles()`, in order.
    fn for_each_vehicle(&self, f: &mut dyn FnMut(&dyn GameVehicle));
    /// `chunk.vehicles` of the chunk at `(wx, wy)` (`IsoCell
    /// .getChunkForGridSquare(wx * 8, wy * 8, 0)`, or
    /// `ServerMap.getChunk(wx, wy)` on a server); nothing for a chunk that is
    /// not loaded. Stops early when `f` returns `true`.
    fn for_each_vehicle_in_chunk(
        &self,
        wx: i32,
        wy: i32,
        f: &mut dyn FnMut(&dyn GameVehicle) -> bool,
    );
}

/// A world without vehicles.
pub struct NoVehicles;

impl VehicleSource for NoVehicles {
    fn for_each_vehicle(&self, _f: &mut dyn FnMut(&dyn GameVehicle)) {}
    fn for_each_vehicle_in_chunk(
        &self,
        _wx: i32,
        _wy: i32,
        _f: &mut dyn FnMut(&dyn GameVehicle) -> bool,
    ) {
    }
}

/// `VehiclePoly.isEqual(other)`: every corner within `0.001` (the level is
/// not compared).
pub fn poly_is_equal(a: &VehiclePoly, b: &VehiclePoly) -> bool {
    let eq = |x: f32, y: f32| (y - x).abs() < 0.001f32;
    eq(a.x1, b.x1)
        && eq(a.y1, b.y1)
        && eq(a.x2, b.x2)
        && eq(a.y2, b.y2)
        && eq(a.x3, b.x3)
        && eq(a.y3, b.y3)
        && eq(a.x4, b.x4)
        && eq(a.y4, b.y4)
}

/// `VehiclePoly.containsPoint(x, y)`: winding number over the corners in
/// order.
pub fn poly_contains_point(p: &VehiclePoly, x: f32, y: f32) -> bool {
    let v = [(p.x1, p.y1), (p.x2, p.y2), (p.x3, p.y3), (p.x4, p.y4)];
    let is_left = |x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32| {
        (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
    };
    let mut wn = 0;
    for i in 0..4 {
        let v1 = v[i];
        let v2 = if i == 3 { v[0] } else { v[i + 1] };
        if v1.1 <= y {
            if v2.1 > y && is_left(v1.0, v1.1, v2.0, v2.1, x, y) > 0.0 {
                wn += 1;
            }
        } else if v2.1 <= y && is_left(v1.0, v1.1, v2.0, v2.1, x, y) < 0.0 {
            wn -= 1;
        }
    }
    wn != 0
}
