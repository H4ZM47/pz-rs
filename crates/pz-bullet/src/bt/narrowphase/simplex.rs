// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2006 Erwin Coumans  http://continuousphysics.com/Bullet/
//
// This software is provided 'as-is', without any express or implied warranty.
// In no event will the authors be held liable for any damages arising from the use of this software.
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it freely,
// subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Elsevier CDROM license agreements grants nonexclusive license to use the software
// for any purpose, commercial or non-commercial as long as the following credit is included
// identifying the original source of the software:
//
// Parts of the source are "from the book Real-Time Collision Detection by
// Christer Ericson, published by Morgan Kaufmann Publishers,
// (c) 2005 Elsevier Inc."
//
// Altered for pz-rs: Rust translation of btVoronoiSimplexSolver (Bullet 2.82,
// double precision, BT_USE_EQUAL_VERTEX_THRESHOLD and
// CATCH_DEGENERATE_TETRAHEDRON as compiled).

//! `btVoronoiSimplexSolver`.

use crate::bt::linear_math::*;

const VORONOI_SIMPLEX_MAX_VERTS: usize = 5;
/// `VORONOI_DEFAULT_EQUAL_VERTEX_THRESHOLD` (`0.0001f`).
pub const VORONOI_DEFAULT_EQUAL_VERTEX_THRESHOLD: f64 = flt(0.0001);

/// `btUsageBitfield` (the four used flags).
#[derive(Clone, Copy, Debug, Default)]
pub struct UsageBitfield {
    pub used_vertex_a: bool,
    pub used_vertex_b: bool,
    pub used_vertex_c: bool,
    pub used_vertex_d: bool,
}

impl UsageBitfield {
    fn reset(&mut self) {
        *self = UsageBitfield::default();
    }
}

/// `btSubSimplexClosestResult`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SubSimplexClosestResult {
    pub closest_point_on_simplex: Vec3,
    pub used_vertices: UsageBitfield,
    pub barycentric_coords: [f64; 4],
    pub degenerate: bool,
}

impl SubSimplexClosestResult {
    fn reset(&mut self) {
        self.degenerate = false;
        self.set_barycentric_coordinates(0.0, 0.0, 0.0, 0.0);
        self.used_vertices.reset();
    }
    fn is_valid(&self) -> bool {
        self.barycentric_coords[0] >= 0.0
            && self.barycentric_coords[1] >= 0.0
            && self.barycentric_coords[2] >= 0.0
            && self.barycentric_coords[3] >= 0.0
    }
    fn set_barycentric_coordinates(&mut self, a: f64, b: f64, c: f64, d: f64) {
        self.barycentric_coords = [a, b, c, d];
    }
}

/// `btVoronoiSimplexSolver`.
#[derive(Clone, Debug)]
pub struct VoronoiSimplexSolver {
    pub num_vertices: usize,
    pub simplex_vector_w: [Vec3; VORONOI_SIMPLEX_MAX_VERTS],
    pub simplex_points_p: [Vec3; VORONOI_SIMPLEX_MAX_VERTS],
    pub simplex_points_q: [Vec3; VORONOI_SIMPLEX_MAX_VERTS],
    pub cached_p1: Vec3,
    pub cached_p2: Vec3,
    pub cached_v: Vec3,
    pub last_w: Vec3,
    pub equal_vertex_threshold: f64,
    pub cached_valid_closest: bool,
    pub cached_bc: SubSimplexClosestResult,
    pub needs_update: bool,
}

impl Default for VoronoiSimplexSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl VoronoiSimplexSolver {
    /// `btVoronoiSimplexSolver()`. Members the constructor leaves
    /// uninitialised start at zero; `reset()` runs before any use.
    pub fn new() -> VoronoiSimplexSolver {
        VoronoiSimplexSolver {
            num_vertices: 0,
            simplex_vector_w: [Vec3::ZERO; VORONOI_SIMPLEX_MAX_VERTS],
            simplex_points_p: [Vec3::ZERO; VORONOI_SIMPLEX_MAX_VERTS],
            simplex_points_q: [Vec3::ZERO; VORONOI_SIMPLEX_MAX_VERTS],
            cached_p1: Vec3::ZERO,
            cached_p2: Vec3::ZERO,
            cached_v: Vec3::ZERO,
            last_w: Vec3::ZERO,
            equal_vertex_threshold: VORONOI_DEFAULT_EQUAL_VERTEX_THRESHOLD,
            cached_valid_closest: false,
            cached_bc: SubSimplexClosestResult::default(),
            needs_update: false,
        }
    }

    fn remove_vertex(&mut self, index: usize) {
        self.num_vertices -= 1;
        let n = self.num_vertices;
        self.simplex_vector_w[index] = self.simplex_vector_w[n];
        self.simplex_points_p[index] = self.simplex_points_p[n];
        self.simplex_points_q[index] = self.simplex_points_q[n];
    }

    fn reduce_vertices(&mut self, used: UsageBitfield) {
        if self.num_vertices >= 4 && !used.used_vertex_d {
            self.remove_vertex(3);
        }
        if self.num_vertices >= 3 && !used.used_vertex_c {
            self.remove_vertex(2);
        }
        if self.num_vertices >= 2 && !used.used_vertex_b {
            self.remove_vertex(1);
        }
        if self.num_vertices >= 1 && !used.used_vertex_a {
            self.remove_vertex(0);
        }
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.cached_valid_closest = false;
        self.num_vertices = 0;
        self.needs_update = true;
        self.last_w = Vec3::new(BT_LARGE_FLOAT, BT_LARGE_FLOAT, BT_LARGE_FLOAT);
        self.cached_bc.reset();
    }

    /// `addVertex(w, p, q)`.
    pub fn add_vertex(&mut self, w: &Vec3, p: &Vec3, q: &Vec3) {
        self.last_w = *w;
        self.needs_update = true;
        let n = self.num_vertices;
        self.simplex_vector_w[n] = *w;
        self.simplex_points_p[n] = *p;
        self.simplex_points_q[n] = *q;
        self.num_vertices += 1;
    }

    fn update_closest_vector_and_points(&mut self) -> bool {
        if self.needs_update {
            self.cached_bc.reset();
            self.needs_update = false;
            match self.num_vertices {
                0 => self.cached_valid_closest = false,
                1 => {
                    self.cached_p1 = self.simplex_points_p[0];
                    self.cached_p2 = self.simplex_points_q[0];
                    self.cached_v = self.cached_p1 - self.cached_p2;
                    self.cached_bc.reset();
                    self.cached_bc
                        .set_barycentric_coordinates(1.0, 0.0, 0.0, 0.0);
                    self.cached_valid_closest = self.cached_bc.is_valid();
                }
                2 => {
                    let from = self.simplex_vector_w[0];
                    let to = self.simplex_vector_w[1];
                    let p = Vec3::ZERO;
                    let mut diff = p - from;
                    let v = to - from;
                    let mut t = v.dot(&diff);
                    if t > 0.0 {
                        let dot_vv = v.dot(&v);
                        if t < dot_vv {
                            t /= dot_vv;
                            diff -= t * v;
                            self.cached_bc.used_vertices.used_vertex_a = true;
                            self.cached_bc.used_vertices.used_vertex_b = true;
                        } else {
                            t = 1.0;
                            diff -= v;
                            self.cached_bc.used_vertices.used_vertex_b = true;
                        }
                    } else {
                        t = 0.0;
                        self.cached_bc.used_vertices.used_vertex_a = true;
                    }
                    let _ = diff;
                    self.cached_bc
                        .set_barycentric_coordinates(1.0 - t, t, 0.0, 0.0);
                    self.cached_p1 = self.simplex_points_p[0]
                        + t * (self.simplex_points_p[1] - self.simplex_points_p[0]);
                    self.cached_p2 = self.simplex_points_q[0]
                        + t * (self.simplex_points_q[1] - self.simplex_points_q[0]);
                    self.cached_v = self.cached_p1 - self.cached_p2;
                    let used = self.cached_bc.used_vertices;
                    self.reduce_vertices(used);
                    self.cached_valid_closest = self.cached_bc.is_valid();
                }
                3 => {
                    let p = Vec3::ZERO;
                    let a = self.simplex_vector_w[0];
                    let b = self.simplex_vector_w[1];
                    let c = self.simplex_vector_w[2];
                    let mut bc = self.cached_bc;
                    closest_pt_point_triangle(&p, &a, &b, &c, &mut bc);
                    self.cached_bc = bc;
                    let w = bc.barycentric_coords;
                    self.cached_p1 = self.simplex_points_p[0] * w[0]
                        + self.simplex_points_p[1] * w[1]
                        + self.simplex_points_p[2] * w[2];
                    self.cached_p2 = self.simplex_points_q[0] * w[0]
                        + self.simplex_points_q[1] * w[1]
                        + self.simplex_points_q[2] * w[2];
                    self.cached_v = self.cached_p1 - self.cached_p2;
                    self.reduce_vertices(bc.used_vertices);
                    self.cached_valid_closest = self.cached_bc.is_valid();
                }
                4 => {
                    let p = Vec3::ZERO;
                    let a = self.simplex_vector_w[0];
                    let b = self.simplex_vector_w[1];
                    let c = self.simplex_vector_w[2];
                    let d = self.simplex_vector_w[3];
                    let mut bc = self.cached_bc;
                    let has_seperation = closest_pt_point_tetrahedron(&p, &a, &b, &c, &d, &mut bc);
                    self.cached_bc = bc;
                    if has_seperation {
                        let w = bc.barycentric_coords;
                        self.cached_p1 = self.simplex_points_p[0] * w[0]
                            + self.simplex_points_p[1] * w[1]
                            + self.simplex_points_p[2] * w[2]
                            + self.simplex_points_p[3] * w[3];
                        self.cached_p2 = self.simplex_points_q[0] * w[0]
                            + self.simplex_points_q[1] * w[1]
                            + self.simplex_points_q[2] * w[2]
                            + self.simplex_points_q[3] * w[3];
                        self.cached_v = self.cached_p1 - self.cached_p2;
                        self.reduce_vertices(bc.used_vertices);
                    } else {
                        if self.cached_bc.degenerate {
                            self.cached_valid_closest = false;
                        } else {
                            self.cached_valid_closest = true;
                            self.cached_v.set_value(0.0, 0.0, 0.0);
                        }
                        return self.cached_valid_closest;
                    }
                    self.cached_valid_closest = self.cached_bc.is_valid();
                }
                _ => self.cached_valid_closest = false,
            }
        }
        self.cached_valid_closest
    }

    /// `closest(v)`.
    pub fn closest(&mut self, v: &mut Vec3) -> bool {
        let succes = self.update_closest_vector_and_points();
        *v = self.cached_v;
        succes
    }

    /// `maxVertex()`.
    pub fn max_vertex(&self) -> f64 {
        let mut max_v = 0.0;
        for i in 0..self.num_vertices {
            let cur_len2 = self.simplex_vector_w[i].length2();
            if max_v < cur_len2 {
                max_v = cur_len2;
            }
        }
        max_v
    }

    #[inline(always)]
    pub fn full_simplex(&self) -> bool {
        self.num_vertices == 4
    }

    /// `inSimplex(w)` with `BT_USE_EQUAL_VERTEX_THRESHOLD`.
    pub fn in_simplex(&self, w: &Vec3) -> bool {
        let mut found = false;
        for i in 0..self.num_vertices {
            if self.simplex_vector_w[i].distance2(w) <= self.equal_vertex_threshold {
                found = true;
            }
        }
        if *w == self.last_w {
            return true;
        }
        found
    }

    /// `backup_closest(v)`.
    pub fn backup_closest(&self, v: &mut Vec3) {
        *v = self.cached_v;
    }

    #[inline(always)]
    pub fn empty_simplex(&self) -> bool {
        self.num_vertices == 0
    }

    /// `compute_points(p1, p2)`.
    pub fn compute_points(&mut self, p1: &mut Vec3, p2: &mut Vec3) {
        self.update_closest_vector_and_points();
        *p1 = self.cached_p1;
        *p2 = self.cached_p2;
    }

    #[inline(always)]
    pub fn num_vertices(&self) -> usize {
        self.num_vertices
    }
}

/// `closestPtPointTriangle`.
fn closest_pt_point_triangle(
    p: &Vec3,
    a: &Vec3,
    b: &Vec3,
    c: &Vec3,
    result: &mut SubSimplexClosestResult,
) -> bool {
    result.used_vertices.reset();
    let ab = *b - *a;
    let ac = *c - *a;
    let ap = *p - *a;
    let d1 = ab.dot(&ap);
    let d2 = ac.dot(&ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        result.closest_point_on_simplex = *a;
        result.used_vertices.used_vertex_a = true;
        result.set_barycentric_coordinates(1.0, 0.0, 0.0, 0.0);
        return true;
    }
    let bp = *p - *b;
    let d3 = ab.dot(&bp);
    let d4 = ac.dot(&bp);
    if d3 >= 0.0 && d4 <= d3 {
        result.closest_point_on_simplex = *b;
        result.used_vertices.used_vertex_b = true;
        result.set_barycentric_coordinates(0.0, 1.0, 0.0, 0.0);
        return true;
    }
    let vc = fma(d1, d4, -(d3 * d2));
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        result.closest_point_on_simplex = *a + v * ab;
        result.used_vertices.used_vertex_a = true;
        result.used_vertices.used_vertex_b = true;
        result.set_barycentric_coordinates(1.0 - v, v, 0.0, 0.0);
        return true;
    }
    let cp = *p - *c;
    let d5 = ab.dot(&cp);
    let d6 = ac.dot(&cp);
    if d6 >= 0.0 && d5 <= d6 {
        result.closest_point_on_simplex = *c;
        result.used_vertices.used_vertex_c = true;
        result.set_barycentric_coordinates(0.0, 0.0, 1.0, 0.0);
        return true;
    }
    let vb = fma(d5, d2, -(d1 * d6));
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        result.closest_point_on_simplex = *a + w * ac;
        result.used_vertices.used_vertex_a = true;
        result.used_vertices.used_vertex_c = true;
        result.set_barycentric_coordinates(1.0 - w, 0.0, w, 0.0);
        return true;
    }
    let va = fma(d3, d6, -(d5 * d4));
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        result.closest_point_on_simplex = *b + w * (*c - *b);
        result.used_vertices.used_vertex_b = true;
        result.used_vertices.used_vertex_c = true;
        result.set_barycentric_coordinates(0.0, 1.0 - w, w, 0.0);
        return true;
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    result.closest_point_on_simplex = *a + ab * v + ac * w;
    result.used_vertices.used_vertex_a = true;
    result.used_vertices.used_vertex_b = true;
    result.used_vertices.used_vertex_c = true;
    result.set_barycentric_coordinates(1.0 - v - w, v, w, 0.0);
    true
}

/// `pointOutsideOfPlane`: 1, 0, or -1 for a degenerate tetrahedron.
fn point_outside_of_plane(p: &Vec3, a: &Vec3, b: &Vec3, c: &Vec3, d: &Vec3) -> i32 {
    let normal = (*b - *a).cross(&(*c - *a));
    let signp = (*p - *a).dot(&normal);
    let signd = (*d - *a).dot(&normal);
    if signd * signd < (1e-8 * 1e-8) {
        return -1;
    }
    (signp * signd < 0.0) as i32
}

/// `closestPtPointTetrahedron`.
fn closest_pt_point_tetrahedron(
    p: &Vec3,
    a: &Vec3,
    b: &Vec3,
    c: &Vec3,
    d: &Vec3,
    final_result: &mut SubSimplexClosestResult,
) -> bool {
    let mut temp = SubSimplexClosestResult::default();
    final_result.closest_point_on_simplex = *p;
    final_result.used_vertices.reset();
    final_result.used_vertices.used_vertex_a = true;
    final_result.used_vertices.used_vertex_b = true;
    final_result.used_vertices.used_vertex_c = true;
    final_result.used_vertices.used_vertex_d = true;

    let point_outside_abc = point_outside_of_plane(p, a, b, c, d);
    let point_outside_acd = point_outside_of_plane(p, a, c, d, b);
    let point_outside_adb = point_outside_of_plane(p, a, d, b, c);
    let point_outside_bdc = point_outside_of_plane(p, b, d, c, a);

    if point_outside_abc < 0
        || point_outside_acd < 0
        || point_outside_adb < 0
        || point_outside_bdc < 0
    {
        final_result.degenerate = true;
        return false;
    }
    if point_outside_abc == 0
        && point_outside_acd == 0
        && point_outside_adb == 0
        && point_outside_bdc == 0
    {
        return false;
    }

    let mut best_sq_dist = f32::MAX as f64;
    if point_outside_abc != 0 {
        closest_pt_point_triangle(p, a, b, c, &mut temp);
        let q = temp.closest_point_on_simplex;
        let sq_dist = (q - *p).dot(&(q - *p));
        if sq_dist < best_sq_dist {
            best_sq_dist = sq_dist;
            final_result.closest_point_on_simplex = q;
            final_result.used_vertices.reset();
            final_result.used_vertices.used_vertex_a = temp.used_vertices.used_vertex_a;
            final_result.used_vertices.used_vertex_b = temp.used_vertices.used_vertex_b;
            final_result.used_vertices.used_vertex_c = temp.used_vertices.used_vertex_c;
            let t = temp.barycentric_coords;
            final_result.set_barycentric_coordinates(t[0], t[1], t[2], 0.0);
        }
    }
    if point_outside_acd != 0 {
        closest_pt_point_triangle(p, a, c, d, &mut temp);
        let q = temp.closest_point_on_simplex;
        let sq_dist = (q - *p).dot(&(q - *p));
        if sq_dist < best_sq_dist {
            best_sq_dist = sq_dist;
            final_result.closest_point_on_simplex = q;
            final_result.used_vertices.reset();
            final_result.used_vertices.used_vertex_a = temp.used_vertices.used_vertex_a;
            final_result.used_vertices.used_vertex_c = temp.used_vertices.used_vertex_b;
            final_result.used_vertices.used_vertex_d = temp.used_vertices.used_vertex_c;
            let t = temp.barycentric_coords;
            final_result.set_barycentric_coordinates(t[0], 0.0, t[1], t[2]);
        }
    }
    if point_outside_adb != 0 {
        closest_pt_point_triangle(p, a, d, b, &mut temp);
        let q = temp.closest_point_on_simplex;
        let sq_dist = (q - *p).dot(&(q - *p));
        if sq_dist < best_sq_dist {
            best_sq_dist = sq_dist;
            final_result.closest_point_on_simplex = q;
            final_result.used_vertices.reset();
            final_result.used_vertices.used_vertex_a = temp.used_vertices.used_vertex_a;
            final_result.used_vertices.used_vertex_b = temp.used_vertices.used_vertex_c;
            final_result.used_vertices.used_vertex_d = temp.used_vertices.used_vertex_b;
            let t = temp.barycentric_coords;
            final_result.set_barycentric_coordinates(t[0], t[2], 0.0, t[1]);
        }
    }
    if point_outside_bdc != 0 {
        closest_pt_point_triangle(p, b, d, c, &mut temp);
        let q = temp.closest_point_on_simplex;
        let sq_dist = (q - *p).dot(&(q - *p));
        if sq_dist < best_sq_dist {
            final_result.closest_point_on_simplex = q;
            final_result.used_vertices.reset();
            final_result.used_vertices.used_vertex_b = temp.used_vertices.used_vertex_a;
            final_result.used_vertices.used_vertex_c = temp.used_vertices.used_vertex_c;
            final_result.used_vertices.used_vertex_d = temp.used_vertices.used_vertex_b;
            let t = temp.barycentric_coords;
            final_result.set_barycentric_coordinates(0.0, t[0], t[2], t[1]);
        }
    }
    true
}
