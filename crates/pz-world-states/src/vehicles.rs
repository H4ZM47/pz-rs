//! The vehicle geometry the zombie code reads, from the chunks' loaded
//! `BaseVehicle`s (`IsoChunk.vehicles`): `getIntersectPoint(start, end)` (`IsoZombie
//! .isVehicleBetween`) and `getClosestPointOnPoly` through `getPoly()`
//! (`DistToSquared(vehicle)`).
//!
//! The vehicle's `jniTransform` is the one `BaseVehicle` gives a loaded
//! vehicle before physics moves it: origin `(x, savedPhysicsZ or z, y)`
//! (world offsets cancel out) and the rotation `savedRot`.

use pz_world::save::vehicle::BaseVehicle;
use pz_world::IsoCell;

use crate::StatesEngine;

/// A vehicle script's `getExtents()` and `getCenterOfMassOffset()`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleBox {
    pub extents: [f32; 3],
    pub center_of_mass_offset: [f32; 3],
}

/// `jniTransform` of a loaded vehicle and its script box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleGeometry {
    /// JOML `Matrix3f.set(Quaternionf)`, column-major (`m[col][row]`).
    pub basis: [[f32; 3]; 3],
    /// Physics origin minus the world offset: `(x, height, y)`.
    pub origin: [f32; 3],
    pub script: VehicleBox,
    /// `getQuaternion().x < 0` (which corners `VehiclePoly.init` lifts).
    pub q_x_negative: bool,
}

/// `org.joml.Vector3f.mul(Matrix3fc)`: `m * v` with JOML's `fma`s.
fn mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = v;
    let r = |row: usize| m[0][row].mul_add(x, m[1][row].mul_add(y, m[2][row] * z));
    [r(0), r(1), r(2)]
}

fn transpose(m: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut t = [[0.0; 3]; 3];
    for (c, col) in m.iter().enumerate() {
        for (r, v) in col.iter().enumerate() {
            t[r][c] = *v;
        }
    }
    t
}

impl VehicleGeometry {
    /// The geometry of a saved vehicle with script box `script`.
    pub fn of(v: &BaseVehicle, script: VehicleBox) -> Self {
        Self::from_parts(
            (v.base.x, v.base.y, v.base.z),
            v.saved_physics_z,
            v.saved_rot,
            script,
        )
    }

    /// The geometry of a vehicle at `pos` (world x, y, level z) with
    /// `savedPhysicsZ` and `savedRot` (`x, y, z, w`).
    pub fn from_parts(
        pos: (f32, f32, f32),
        physics_z: f32,
        rot: [f32; 4],
        script: VehicleBox,
    ) -> Self {
        let [x, y, z, w] = rot;
        let (w2, x2, y2, z2) = (w * w, x * x, y * y, z * z);
        let (zw, xy, xz, yw, yz, xw) = (z * w, x * y, x * z, y * w, y * z, x * w);
        let (dzw, dxy, dxz, dyw, dyz, dxw) = (zw + zw, xy + xy, xz + xz, yw + yw, yz + yz, xw + xw);
        let basis = [
            [w2 + x2 - z2 - y2, dxy + dzw, dxz - dyw],
            [-dzw + dxy, y2 - z2 + w2 - x2, dyz + dxw],
            [dyw + dxz, dyz - dxw, z2 - y2 - x2 + w2],
        ];
        let h = if physics_z.is_nan() { pos.2 } else { physics_z };
        // `getQuaternion` reads the basis back (`getUnnormalizedRotation`);
        // its sign of x follows the saved one for a unit quaternion.
        VehicleGeometry {
            basis,
            origin: [pos.0, h, pos.1],
            script,
            q_x_negative: x < 0.0,
        }
    }

    /// The geometry of a live runtime `BaseVehicle` from its current `jni_transform`.
    pub fn from_runtime(v: &pz_vehicles::BaseVehicle, script: VehicleBox) -> Self {
        let t = v.world_transform();
        let (ox, oy) = pz_vehicles::env::offset();
        let b = t.basis;
        let q = t.rotation();
        VehicleGeometry {
            basis: [
                [b.m00, b.m01, b.m02],
                [b.m10, b.m11, b.m12],
                [b.m20, b.m21, b.m22],
            ],
            origin: [t.origin.x + ox, t.origin.y, t.origin.z + oy],
            script,
            q_x_negative: q.x < 0.0,
        }
    }

    /// `getLocalPos(worldX, worldY, worldZ)`: the world z is not used.
    pub fn local_pos(&self, wx: f32, wy: f32) -> [f32; 3] {
        let t = transpose(&self.basis);
        let o = mul(&t, [-self.origin[0], -self.origin[1], -self.origin[2]]);
        let v = mul(&t, [wx, 0.0, wy]);
        [v[0] + o[0], v[1] + o[1], v[2] + o[2]]
    }

    /// `getWorldPos(local)`: `(x, y)` of the world position.
    pub fn world_pos_xy(&self, l: [f32; 3]) -> (f32, f32) {
        let w = mul(&self.basis, l);
        (self.origin[0] + w[0], self.origin[2] + w[2])
    }

    /// `getIntersectPoint(start, end, result) != null`.
    pub fn intersects(&self, start: (f32, f32, f32), end: (f32, f32, f32)) -> bool {
        let a = self.local_pos(start.0, start.1);
        let b = self.local_pos(end.0, end.1);
        slab_intersects(a, b, &self.script)
    }

    /// `getPoly()` (`VehiclePoly.init(vehicle, 0)`) corners.
    pub fn poly(&self) -> [(f32, f32); 4] {
        let s = &self.script;
        let (com, ext) = (s.center_of_mass_offset, s.extents);
        let (w, l, h) = (ext[0] / 2.0, ext[2] / 2.0, ext[1] / 2.0);
        let (ha, hb) = if self.q_x_negative {
            (0.0, h)
        } else {
            (h, 0.0)
        };
        let mut c = [
            self.world_pos_xy([com[0] - w, ha, com[2] + l]),
            self.world_pos_xy([com[0] + w, hb, com[2] + l]),
            self.world_pos_xy([com[0] + w, hb, com[2] - l]),
            self.world_pos_xy([com[0] - w, ha, com[2] - l]),
        ];
        let mut sum = 0i32;
        for i in 0..4 {
            let (v1, v2) = (c[i], c[(i + 1) % 4]);
            sum = (sum as f32 + (v2.0 - v1.0) * (v2.1 + v1.1)) as i32;
        }
        if sum < 0 {
            c.swap(1, 3);
        }
        c
    }

    /// `getClosestPointOnPoly(x, y)`: the squared distance.
    pub fn closest_dist_sq(&self, x: f32, y: f32) -> f32 {
        let p = self.poly();
        let mut best = f32::MAX as f64;
        for i in 0..4 {
            let d = closest_point_on_segment(p[i], p[(i + 1) % 4], x, y);
            if d < best {
                best = d;
            }
        }
        best as f32
    }
}

/// `PZMath.closestPointOnLineSegment(x1, y1, x2, y2, px, py, 0)`.
fn closest_point_on_segment(a: (f32, f32), b: (f32, f32), px: f32, py: f32) -> f64 {
    let (dx, dy) = ((b.0 - a.0) as f64, (b.1 - a.1) as f64);
    let len = dx * dx + dy * dy;
    let d2 = |x: f64, y: f64| {
        let (ddx, ddy) = (px as f64 - x, py as f64 - y);
        ddx * ddx + ddy * ddy
    };
    if len < 1.0e-12 {
        return d2(a.0 as f64, a.1 as f64);
    }
    let u = ((px - a.0) as f64 * dx + (py - a.1) as f64 * dy) / len;
    if u <= 0.0 {
        return d2(a.0 as f64, a.1 as f64);
    }
    if u >= 1.0 {
        return d2(b.0 as f64, b.1 as f64);
    }
    d2(a.0 as f64 + u * dx, a.1 as f64 + u * dy)
}

fn slab_intersects(a: [f32; 3], b: [f32; 3], s: &VehicleBox) -> bool {
    let ext = [
        s.extents[0] * 0.5 + s.center_of_mass_offset[0],
        s.extents[1] * 0.5 + s.center_of_mass_offset[1],
        s.extents[2] * 0.5 + s.center_of_mass_offset[2],
    ];
    let (mut tmin, mut tmax) = (0.0f32, 1.0f32);
    for i in 0..3 {
        let d = b[i] - a[i];
        let (lo, hi) = (-ext[i], ext[i]);
        if d.abs() < 1.0e-6 {
            if a[i] < lo || a[i] > hi {
                return false;
            }
            continue;
        }
        let (mut t1, mut t2) = ((lo - a[i]) / d, (hi - a[i]) / d);
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return false;
        }
    }
    true
}

fn runtime_vehicle_box(
    v: &pz_vehicles::BaseVehicle,
    engine: &dyn StatesEngine,
) -> Option<VehicleBox> {
    if let Some(script) = v.script() {
        let s = script.borrow();
        let e = s.extents();
        let com = s.center_of_mass_offset();
        return Some(VehicleBox {
            extents: [e.x, e.y, e.z],
            center_of_mass_offset: [com.x, com.y, com.z],
        });
    }
    let st = v.st();
    engine.vehicle_box(&st.script_name)
}

fn runtime_vehicle_intersects(
    v: &pz_vehicles::BaseVehicle,
    script: &VehicleBox,
    start: (f32, f32, f32),
    end: (f32, f32, f32),
) -> bool {
    let a = v.local_pos(start.0, start.1, start.2);
    let b = v.local_pos(end.0, end.1, end.2);
    slab_intersects([a.x, a.y, a.z], [b.x, b.y, b.z], script)
}

/// The vehicles chunk loading put in `IsoChunk.vehicles` (chunk files and
/// `vehicles.db` rows); live runtime vehicles in `pz_vehicles::manager`
/// override matching entries when active.
fn for_each_vehicle(cell: &IsoCell, f: &mut dyn FnMut(&BaseVehicle) -> bool) {
    for ch in cell.chunks.values() {
        for v in ch.vehicles.loaded() {
            if f(&v.vehicle) {
                return;
            }
        }
    }
}

/// `IsoZombie.isVehicleBetween`: any live cell vehicle or loaded chunk vehicle
/// with a script crossed by the segment.
pub fn vehicle_between(
    cell: &IsoCell,
    engine: &dyn StatesEngine,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> bool {
    let live = pz_vehicles::manager::cell_vehicles();
    let mut seen_ids = Vec::with_capacity(live.len());
    for v in &live {
        let vid = v.st().vehicle_id;
        if vid >= 0 {
            seen_ids.push(vid);
        }
        if let Some(b) = runtime_vehicle_box(v, engine) {
            if runtime_vehicle_intersects(v, &b, from, to) {
                return true;
            }
        }
    }
    let mut hit = false;
    for_each_vehicle(cell, &mut |v| {
        if v.vehicle_id >= 0 && seen_ids.contains(&v.vehicle_id) {
            return false;
        }
        if let Some(b) = engine.vehicle_box(&v.script_name) {
            hit = VehicleGeometry::of(v, b).intersects(from, to);
        }
        hit
    });
    hit
}

/// `DistToSquared(vehicle)` from `(x, y)`: the closest point of its
/// polygon, or 0 without a script; `f32::MAX` for an unknown vehicle.
pub fn vehicle_dist_squared(
    cell: &IsoCell,
    engine: &dyn StatesEngine,
    id: i16,
    x: f32,
    y: f32,
) -> f32 {
    if let Some(v) = pz_vehicles::manager::vehicle_by_id(id) {
        if v.script().is_some() {
            return v.closest_point_on_poly(x, y).0;
        }
        let script_name = v.st().script_name.clone();
        return match engine.vehicle_box(&script_name) {
            Some(b) => VehicleGeometry::from_runtime(&v, b).closest_dist_sq(x, y),
            None => 0.0,
        };
    }
    let mut d = f32::MAX;
    for_each_vehicle(cell, &mut |v| {
        if v.vehicle_id != id {
            return false;
        }
        d = match engine.vehicle_box(&v.script_name) {
            Some(b) => VehicleGeometry::of(v, b).closest_dist_sq(x, y),
            None => 0.0,
        };
        true
    });
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::{GameRand, IsoCell};

    struct DummyEngine {
        v_box: VehicleBox,
        rand: ZeroRand,
    }
    struct ZeroRand;
    impl GameRand for ZeroRand {
        fn next(&mut self, _max: i32) -> i32 {
            0
        }
    }
    impl StatesEngine for DummyEngine {
        fn rand(&mut self) -> &mut dyn GameRand {
            &mut self.rand
        }
        fn vehicle_box(&self, script_name: &str) -> Option<VehicleBox> {
            (script_name == "Base.CarNormal").then_some(self.v_box)
        }
    }

    #[test]
    fn live_runtime_vehicles_drive_vehicle_between_and_dist_squared() {
        pz_vehicles::manager::reset();
        let v = pz_vehicles::BaseVehicle::new();
        {
            let mut st = v.st_mut();
            st.vehicle_id = 17;
            st.script_name = "Base.CarNormal".into();
            st.x = 50.0;
            st.y = 60.0;
            st.z = 0.0;
            st.saved_physics_z = 0.0;
            st.jni_transform.origin.set(50.0, 0.0, 60.0);
        }
        pz_vehicles::manager::id_map_put(17, &v);
        pz_vehicles::manager::cell_add(&v);
        pz_vehicles::manager::process_add_vehicles();

        let engine = DummyEngine {
            v_box: VehicleBox {
                extents: [2.0, 1.5, 4.0],
                center_of_mass_offset: [0.0, 0.0, 0.0],
            },
            rand: ZeroRand,
        };
        let cell = IsoCell::default();

        // Ray crossing x=50, y=60 hits the live runtime vehicle.
        assert!(vehicle_between(
            &cell,
            &engine,
            (45.0, 60.0, 0.1),
            (55.0, 60.0, 0.1)
        ));
        // Ray far away misses it.
        assert!(!vehicle_between(
            &cell,
            &engine,
            (10.0, 10.0, 0.1),
            (20.0, 10.0, 0.1)
        ));
        // Distance squared from (53.0, 60.0) to 2x4 box centered at (50, 60) => dx=2 => dist^2 = 4.
        let d2 = vehicle_dist_squared(&cell, &engine, 17, 53.0, 60.0);
        assert!((d2 - 4.0).abs() < 1e-3, "expected ~4.0, got {d2}");
        // Unknown vehicle id still returns f32::MAX.
        assert_eq!(vehicle_dist_squared(&cell, &engine, 999, 53.0, 60.0), f32::MAX);

        pz_vehicles::manager::reset();
    }
}
