//! `UI3DScene.Ray` / `Plane` / `Circle` and the geometry helpers the
//! gizmos use, with JOML's float arithmetic.

use pz_joml::vec3::{self, V};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ray {
    pub origin: V,
    pub direction: V,
    pub t: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    pub point: V,
    pub normal: V,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub center: V,
    pub orientation: V,
    pub radius: f32,
}

impl Default for Circle {
    fn default() -> Self {
        Circle {
            center: [0.0; 3],
            orientation: [0.0; 3],
            radius: 1.0,
        }
    }
}

/// `UI3DScene.closest_distance_between_lines(l1, l2)`: sets both `t`.
pub fn closest_distance_between_lines(l1: &mut Ray, l2: &mut Ray) -> f32 {
    let (u, v) = (l1.direction, l2.direction);
    let w = vec3::sub(l1.origin, l2.origin);
    let a = vec3::dot(u, u);
    let b = vec3::dot(u, v);
    let c = vec3::dot(v, v);
    let d = vec3::dot(u, w);
    let e = vec3::dot(v, w);
    let det = a * c - b * b;
    let (sc, tc) = if det < 1.0e-8 {
        (0.0, if b > c { d / b } else { e / c })
    } else {
        ((b * e - c * d) / det, (a * e - b * d) / det)
    };
    let dp = vec3::sub(vec3::add(w, vec3::mul(u, sc)), vec3::mul(v, tc));
    l1.t = sc;
    l2.t = tc;
    vec3::length(dp)
}

/// `UI3DScene.intersect_ray_plane(plane, ray, out)`: 1 with the hit, 0 for
/// none, 2 when the ray lies in the plane.
pub fn intersect_ray_plane(pn: &Plane, s: &Ray) -> (i32, Option<V>) {
    let u = vec3::mul(s.direction, 100.0);
    let w = vec3::sub(s.origin, pn.point);
    let d = vec3::dot(pn.normal, u);
    let n = -vec3::dot(pn.normal, w);
    if d.abs() < 1.0e-8 {
        return (if n == 0.0 { 2 } else { 0 }, None);
    }
    let si = n / d;
    if !(0.0..=1.0).contains(&si) {
        return (0, None);
    }
    (1, Some(vec3::add(s.origin, vec3::mul(u, si))))
}

fn project(a: V, b: V) -> V {
    vec3::mul(b, vec3::dot(a, b) / vec3::dot(b, b))
}

fn reject(a: V, b: V) -> V {
    vec3::sub(a, project(a, b))
}

/// `UI3DScene.distance_between_point_ray(p, ray)`.
pub fn distance_between_point_ray(p: V, l: &Ray) -> f32 {
    let v = vec3::mul(l.direction, 100.0);
    let w = vec3::sub(p, l.origin);
    let b = vec3::dot(w, v) / vec3::dot(v, v);
    let pb = vec3::add(vec3::mul(v, b), l.origin);
    vec3::length(vec3::sub(pb, p))
}

/// `UI3DScene.closest_distance_line_circle(ray, circle, point)`: the
/// distance and the closest point on the circle.
pub fn closest_distance_line_circle(ray: &Ray, c: &Circle) -> (f32, V) {
    let plane = Plane {
        point: c.center,
        normal: c.orientation,
    };
    if let (1, Some(on_plane)) = intersect_ray_plane(&plane, ray) {
        let point = vec3::add(
            vec3::mul(vec3::normalize(vec3::sub(on_plane, c.center)), c.radius),
            c.center,
        );
        (vec3::length(vec3::sub(on_plane, point)), point)
    } else {
        let a = vec3::sub(ray.origin, c.center);
        let rt = reject(a, c.orientation);
        let point = vec3::add(vec3::mul(vec3::normalize(rt), c.radius), c.center);
        (distance_between_point_ray(point, ray), point)
    }
}
