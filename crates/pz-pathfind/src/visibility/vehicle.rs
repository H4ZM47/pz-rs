//! `Vehicle`, `VehiclePoly`, `VehicleRect` and `VehicleCluster`.

use super::{ClusterId, VehicleId};

/// `VehiclePoly`: a vehicle's footprint quad, in world squares, with its
/// level (+32 offset).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehiclePoly {
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

/// `Vehicle`.
#[derive(Clone, Debug, Default)]
pub struct Vehicle {
    pub id: i32,
    pub poly: VehiclePoly,
    pub poly_plus_radius: VehiclePoly,
    pub up_vector_dot: f32,
    pub crawl_offsets: Vec<f32>,
}

/// `VehicleRect`: integer bounding box of a vehicle on one level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VehicleRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub z: i32,
    pub vehicle: Option<VehicleId>,
    pub cluster: Option<ClusterId>,
}

impl VehicleRect {
    pub fn left(&self) -> i32 {
        self.x
    }
    pub fn top(&self) -> i32 {
        self.y
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// `VehicleCluster`: overlapping vehicle rects on one level that share a
/// visibility graph.
#[derive(Clone, Debug, Default)]
pub struct VehicleCluster {
    pub z: i32,
    pub rects: Vec<VehicleRect>,
}

impl VehiclePoly {
    fn corners(&self) -> [(f32, f32); 4] {
        [
            (self.x1, self.y1),
            (self.x2, self.y2),
            (self.x3, self.y3),
            (self.x4, self.y4),
        ]
    }

    /// `VehiclePoly::getAABB`: integer bounds (`floor` of the minimum,
    /// `ceil` of the maximum) on the poly's level.
    pub fn aabb(&self) -> VehicleRect {
        let min = |a: f32, b: f32| if a <= b { a } else { b };
        let max = |a: f32, b: f32| if b < a { a } else { b };
        let min_x = min(self.x1, min(self.x2, min(self.x3, self.x4)));
        let min_y = min(self.y1, min(self.y2, min(self.y3, self.y4)));
        let max_x = max(self.x1, max(self.x2, max(self.x3, self.x4)));
        let max_y = max(self.y1, max(self.y2, max(self.y3, self.y4)));
        let x = crate::util::fastfloor(min_x);
        let y = crate::util::fastfloor(min_y);
        VehicleRect {
            x,
            y,
            w: max_x.ceil() as i32 - x,
            h: max_y.ceil() as i32 - y,
            z: crate::util::fastfloor(self.z),
            vehicle: None,
            cluster: None,
        }
    }

    /// `VehiclePoly::containsPoint` (winding number test).
    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        let c = self.corners();
        let mut wn = 0i32;
        for i in 0..4 {
            let (x1, y1) = c[i];
            let (x2, y2) = c[(i + 1) & 3];
            let is_left = (x2 - x1).mul_add(py - y1, -((y2 - y1) * (px - x1)));
            if y1 <= py {
                if py < y2 && 0.0 < is_left {
                    wn += 1;
                }
            } else if y2 <= py && is_left < 0.0 {
                wn -= 1;
            }
        }
        wn != 0
    }

    /// `LineClearCollide::polyVehicleIntersect`: whether any side of
    /// `quad` crosses any side of this poly.
    pub fn intersects_quad(&self, quad: &[(f32, f32); 4]) -> bool {
        let v = self.corners();
        for i in 1..5 {
            let (ax, ay) = quad[i - 1];
            let (bx, by) = quad[i & 3];
            let dx = bx - ax;
            let dy = by - ay;
            for j in 0..4 {
                let (vx, vy) = v[j];
                let (wx, wy) = v[(j + 1) & 3];
                let ex = wx - vx;
                let ey = wy - vy;
                let denom = (-(ex as f64)).mul_add(dy as f64, (dx * ey) as f64);
                if denom == 0.0 {
                    continue;
                }
                let oy = ay - vy;
                let ox = ax - vx;
                let t1 = dx.mul_add(oy, -(ox * dy)) as f64 / denom;
                if t1 > 1.0 || t1 < 0.0 || t1.is_nan() {
                    continue;
                }
                let t2 = ex.mul_add(oy, -(ox * ey)) as f64 / denom;
                if t2 >= 0.0 && t2 <= 1.0 {
                    return true;
                }
            }
        }
        false
    }
}
