//! `zombie.pathfind.VehiclePoly`, `PolyPolyIntersect` and
//! `QuadranglesIntersection`.

use crate::jmath::{Transform, Vector2};

/// `VehiclePoly`: the vehicle's footprint, corners in winding order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehiclePoly {
    pub t: Transform,
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub x3: f32,
    pub y3: f32,
    pub x4: f32,
    pub y4: f32,
    pub z: f32,
    pub borders: [Vector2; 4],
}

impl VehiclePoly {
    /// The corners in order.
    pub fn corners(&self) -> [(f32, f32); 4] {
        [
            (self.x1, self.y1),
            (self.x2, self.y2),
            (self.x3, self.y3),
            (self.x4, self.y4),
        ]
    }

    /// `isLeft`.
    fn is_left(x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
        (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
    }

    /// `containsPoint(x, y)` (winding number over `borders`).
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        let mut wn = 0;
        for i in 0..4 {
            let v1 = self.borders[i];
            let v2 = if i == 3 {
                self.borders[0]
            } else {
                self.borders[i + 1]
            };
            if v1.y <= y {
                if v2.y > y && Self::is_left(v1.x, v1.y, v2.x, v2.y, x, y) > 0.0 {
                    wn += 1;
                }
            } else if v2.y <= y && Self::is_left(v1.x, v1.y, v2.x, v2.y, x, y) < 0.0 {
                wn -= 1;
            }
        }
        wn != 0
    }

    /// `isEqual(other)`.
    pub fn is_equal(&self, o: &VehiclePoly) -> bool {
        let eq = |a: f32, b: f32| (b - a).abs() < 0.001;
        eq(self.x1, o.x1)
            && eq(self.y1, o.y1)
            && eq(self.x2, o.x2)
            && eq(self.y2, o.y2)
            && eq(self.x3, o.x3)
            && eq(self.y3, o.y3)
            && eq(self.x4, o.x4)
            && eq(self.y4, o.y4)
    }
}

/// `PolyPolyIntersect.intersects(a, b)` (separating axes).
pub fn poly_poly_intersects(a: &VehiclePoly, b: &VehiclePoly) -> bool {
    let pa = a.corners();
    let pb = b.corners();
    for polygon in [&pa, &pb] {
        for i1 in 0..4 {
            let i2 = (i1 + 1) % 4;
            let p1 = polygon[i1];
            let p2 = polygon[i2];
            let n = (p2.1 - p1.1, p1.0 - p2.0);
            let (mut min_a, mut max_a) = (f64::MAX, f64::NEG_INFINITY);
            for p in &pa {
                let proj = (n.0 * p.0 + n.1 * p.1) as f64;
                if proj < min_a {
                    min_a = proj;
                }
                if proj > max_a {
                    max_a = proj;
                }
            }
            let (mut min_b, mut max_b) = (f64::MAX, f64::NEG_INFINITY);
            for p in &pb {
                let proj = (n.0 * p.0 + n.1 * p.1) as f64;
                if proj < min_b {
                    min_b = proj;
                }
                if proj > max_b {
                    max_b = proj;
                }
            }
            if max_a < min_b || max_b < min_a {
                return false;
            }
        }
    }
    true
}

fn det(a: f32, b: f32, c: f32, d: f32) -> f32 {
    a * d - b * c
}

fn between(a: f32, b: f32, c: f64) -> bool {
    a.min(b) as f64 <= c + 0.001f32 as f64 && c <= (a.max(b) + 0.001) as f64
}

fn intersect_1(a1: f32, b1: f32, c1: f32, d1: f32) -> bool {
    let (a, b) = if a1 > b1 { (b1, a1) } else { (a1, b1) };
    let (c, d) = if c1 > d1 { (d1, c1) } else { (c1, d1) };
    a.max(c) <= b.min(d)
}

/// `QuadranglesIntersection.lineIntersection(start1, end1, start2, end2)`.
pub fn line_intersection(s1: Vector2, e1: Vector2, s2: Vector2, e2: Vector2) -> bool {
    let a1 = s1.y - e1.y;
    let b1 = e1.x - s1.x;
    let c1 = -a1 * s1.x - b1 * s1.y;
    let a2 = s2.y - e2.y;
    let b2 = e2.x - s2.x;
    let c2 = -a2 * s2.x - b2 * s2.y;
    let zn = det(a1, b1, a2, b2);
    if zn != 0.0 {
        let x = -det(c1, b1, c2, b2) as f64 * 1.0 / zn as f64;
        let y = -det(a1, c1, a2, c2) as f64 * 1.0 / zn as f64;
        return between(s1.x, e1.x, x)
            && between(s1.y, e1.y, y)
            && between(s2.x, e2.x, x)
            && between(s2.y, e2.y, y);
    }
    det(a1, c1, a2, c2) == 0.0
        && det(b1, c1, b2, c2) == 0.0
        && intersect_1(s1.x, e1.x, s2.x, e2.x)
        && intersect_1(s1.y, e1.y, s2.y, e2.y)
}

fn point_in_triangle(p: Vector2, t1: Vector2, t2: Vector2, t3: Vector2) -> bool {
    let d1 = (t1.x - p.x) * (t2.y - t1.y) - (t2.x - t1.x) * (t1.y - p.y);
    let d2 = (t2.x - p.x) * (t3.y - t2.y) - (t3.x - t2.x) * (t2.y - p.y);
    let d3 = (t3.x - p.x) * (t1.y - t3.y) - (t1.x - t3.x) * (t3.y - p.y);
    (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
}

/// `QuadranglesIntersection.IsQuadranglesAreIntersected(q1, q2)`.
pub fn quadrangles_intersect(q1: &[Vector2; 4], q2: &[Vector2; 4]) -> bool {
    let edges = [(0, 1), (1, 2), (2, 3), (3, 0)];
    for (a, b) in edges {
        for (c, d) in edges {
            if line_intersection(q1[a], q1[b], q2[c], q2[d]) {
                return true;
            }
        }
    }
    if point_in_triangle(q1[0], q2[0], q2[1], q2[2])
        || point_in_triangle(q1[0], q2[0], q2[2], q2[3])
    {
        return true;
    }
    point_in_triangle(q2[0], q1[0], q1[1], q1[2]) || point_in_triangle(q2[0], q1[0], q1[2], q1[3])
}
