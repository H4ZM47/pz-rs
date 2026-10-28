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
// Altered for pz-rs: Rust translation of btDiscreteCollisionDetectorInterface,
// btPointCollector and btGjkPairDetector (Bullet 2.82, double precision).

//! `btGjkPairDetector` and the discrete collision result interface.

use super::gjk_epa::gjk_epa_calc_pen_depth;
use super::simplex::VoronoiSimplexSolver;
use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;

const REL_ERROR2: f64 = 1.0e-6;

/// `btDiscreteCollisionDetectorInterface::Result`.
pub trait DiscreteResult {
    fn set_shape_identifiers_a(&mut self, part_id0: i32, index0: i32);
    fn set_shape_identifiers_b(&mut self, part_id1: i32, index1: i32);
    fn add_contact_point(&mut self, normal_on_b_in_world: &Vec3, point_in_world: &Vec3, depth: f64);
}

/// `btDiscreteCollisionDetectorInterface::ClosestPointInput`.
#[derive(Clone, Copy, Debug)]
pub struct ClosestPointInput {
    pub transform_a: Transform,
    pub transform_b: Transform,
    pub maximum_distance_squared: f64,
}

impl Default for ClosestPointInput {
    fn default() -> Self {
        ClosestPointInput {
            transform_a: Transform::default(),
            transform_b: Transform::default(),
            maximum_distance_squared: BT_LARGE_FLOAT,
        }
    }
}

/// `btPointCollector`.
#[derive(Clone, Copy, Debug)]
pub struct PointCollector {
    pub normal_on_b_in_world: Vec3,
    pub point_in_world: Vec3,
    pub distance: f64,
    pub has_result: bool,
}

impl Default for PointCollector {
    fn default() -> Self {
        PointCollector {
            normal_on_b_in_world: Vec3::ZERO,
            point_in_world: Vec3::ZERO,
            distance: BT_LARGE_FLOAT,
            has_result: false,
        }
    }
}

impl DiscreteResult for PointCollector {
    fn set_shape_identifiers_a(&mut self, _: i32, _: i32) {}
    fn set_shape_identifiers_b(&mut self, _: i32, _: i32) {}
    fn add_contact_point(
        &mut self,
        normal_on_b_in_world: &Vec3,
        point_in_world: &Vec3,
        depth: f64,
    ) {
        if depth < self.distance {
            self.has_result = true;
            self.normal_on_b_in_world = *normal_on_b_in_world;
            self.point_in_world = *point_in_world;
            self.distance = depth;
        }
    }
}

/// The penetration depth solver plugged into a GJK pair detector. The game
/// configures `btDefaultCollisionConfiguration` with
/// `m_useEpaPenetrationAlgorithm = 1`, so only EPA exists here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenetrationDepthSolver {
    /// `btGjkEpaPenetrationDepthSolver`.
    GjkEpa,
}

/// `btGjkPairDetector`.
pub struct GjkPairDetector<'a> {
    pub cached_separating_axis: Vec3,
    pub penetration_depth_solver: Option<PenetrationDepthSolver>,
    pub simplex_solver: &'a mut VoronoiSimplexSolver,
    pub minkowski_a: &'a Shape,
    pub minkowski_b: &'a Shape,
    pub shape_type_a: i32,
    pub shape_type_b: i32,
    pub margin_a: f64,
    pub margin_b: f64,
    pub ignore_margin: bool,
    pub cached_separating_distance: f64,
    pub last_used_method: i32,
    pub cur_iter: i32,
    pub degenerate_simplex: i32,
    pub catch_degeneracies: i32,
    pub fix_contact_normal_direction: i32,
}

impl<'a> GjkPairDetector<'a> {
    /// `btGjkPairDetector(objectA, objectB, simplexSolver, penetrationDepthSolver)`.
    pub fn new(
        object_a: &'a Shape,
        object_b: &'a Shape,
        simplex_solver: &'a mut VoronoiSimplexSolver,
        penetration_depth_solver: Option<PenetrationDepthSolver>,
    ) -> GjkPairDetector<'a> {
        let (ta, tb, ma, mb) = (
            object_a.shape_type(),
            object_b.shape_type(),
            object_a.get_margin(),
            object_b.get_margin(),
        );
        Self::with_params(
            object_a,
            object_b,
            ta,
            tb,
            ma,
            mb,
            simplex_solver,
            penetration_depth_solver,
        )
    }

    /// `btGjkPairDetector(objectA, objectB, shapeTypeA, shapeTypeB, marginA, marginB, ...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn with_params(
        object_a: &'a Shape,
        object_b: &'a Shape,
        shape_type_a: i32,
        shape_type_b: i32,
        margin_a: f64,
        margin_b: f64,
        simplex_solver: &'a mut VoronoiSimplexSolver,
        penetration_depth_solver: Option<PenetrationDepthSolver>,
    ) -> GjkPairDetector<'a> {
        GjkPairDetector {
            cached_separating_axis: Vec3::new(0.0, 1.0, 0.0),
            penetration_depth_solver,
            simplex_solver,
            minkowski_a: object_a,
            minkowski_b: object_b,
            shape_type_a,
            shape_type_b,
            margin_a,
            margin_b,
            ignore_margin: false,
            cached_separating_distance: 0.0,
            last_used_method: -1,
            cur_iter: 0,
            degenerate_simplex: 0,
            catch_degeneracies: 1,
            fix_contact_normal_direction: 1,
        }
    }

    /// `getClosestPoints` (= `getClosestPointsNonVirtual`).
    pub fn get_closest_points(
        &mut self,
        input: &ClosestPointInput,
        output: &mut dyn DiscreteResult,
    ) {
        self.get_closest_points_non_virtual(input, output);
    }

    /// `getClosestPointsNonVirtual`.
    #[allow(unused_assignments)]
    pub fn get_closest_points_non_virtual(
        &mut self,
        input: &ClosestPointInput,
        output: &mut dyn DiscreteResult,
    ) {
        self.cached_separating_distance = 0.0;
        let mut distance = 0.0;
        let mut normal_in_b = Vec3::ZERO;
        let mut point_on_a = Vec3::ZERO;
        let mut point_on_b = Vec3::ZERO;
        let mut local_trans_a = input.transform_a;
        let mut local_trans_b = input.transform_b;
        let position_offset = (local_trans_a.origin + local_trans_b.origin) * 0.5;
        local_trans_a.origin -= position_offset;
        local_trans_b.origin -= position_offset;

        let check2d = self.minkowski_a.is_convex2d() && self.minkowski_b.is_convex2d();
        let mut margin_a = self.margin_a;
        let mut margin_b = self.margin_b;
        if self.ignore_margin {
            margin_a = 0.0;
            margin_b = 0.0;
        }
        self.cur_iter = 0;
        let g_gjk_max_iter = 1000;
        self.cached_separating_axis.set_value(0.0, 1.0, 0.0);

        let mut is_valid = false;
        let mut check_simplex = false;
        let check_penetration = true;
        self.degenerate_simplex = 0;
        self.last_used_method = -1;

        {
            let mut squared_distance = BT_LARGE_FLOAT;
            let margin = margin_a + margin_b;
            self.simplex_solver.reset();
            loop {
                let seperating_axis_in_a = (-self.cached_separating_axis) * input.transform_a.basis;
                let seperating_axis_in_b = self.cached_separating_axis * input.transform_b.basis;
                let p_in_a = self
                    .minkowski_a
                    .local_get_support_vertex_without_margin_non_virtual(&seperating_axis_in_a);
                let q_in_b = self
                    .minkowski_b
                    .local_get_support_vertex_without_margin_non_virtual(&seperating_axis_in_b);
                let mut p_world = local_trans_a.apply(&p_in_a);
                let mut q_world = local_trans_b.apply(&q_in_b);
                if check2d {
                    p_world.z = 0.0;
                    q_world.z = 0.0;
                }
                let w = p_world - q_world;
                let delta = self.cached_separating_axis.dot(&w);

                if delta > 0.0
                    && (delta * delta > squared_distance * input.maximum_distance_squared)
                {
                    self.degenerate_simplex = 10;
                    check_simplex = true;
                    break;
                }
                if self.simplex_solver.in_simplex(&w) {
                    self.degenerate_simplex = 1;
                    check_simplex = true;
                    break;
                }
                let f0 = squared_distance - delta;
                let f1 = squared_distance * REL_ERROR2;
                if f0 <= f1 {
                    if f0 <= 0.0 {
                        self.degenerate_simplex = 2;
                    } else {
                        self.degenerate_simplex = 11;
                    }
                    check_simplex = true;
                    break;
                }
                self.simplex_solver.add_vertex(&w, &p_world, &q_world);
                let mut new_cached_separating_axis = Vec3::ZERO;
                if !self.simplex_solver.closest(&mut new_cached_separating_axis) {
                    self.degenerate_simplex = 3;
                    check_simplex = true;
                    break;
                }
                if new_cached_separating_axis.length2() < REL_ERROR2 {
                    self.cached_separating_axis = new_cached_separating_axis;
                    self.degenerate_simplex = 6;
                    check_simplex = true;
                    break;
                }
                let previous_squared_distance = squared_distance;
                squared_distance = new_cached_separating_axis.length2();
                if previous_squared_distance - squared_distance
                    <= SIMD_EPSILON * previous_squared_distance
                {
                    check_simplex = true;
                    self.degenerate_simplex = 12;
                    break;
                }
                self.cached_separating_axis = new_cached_separating_axis;
                let it = self.cur_iter;
                self.cur_iter += 1;
                if it > g_gjk_max_iter {
                    break;
                }
                let check = !self.simplex_solver.full_simplex();
                if !check {
                    self.degenerate_simplex = 13;
                    break;
                }
            }

            if check_simplex {
                self.simplex_solver
                    .compute_points(&mut point_on_a, &mut point_on_b);
                normal_in_b = self.cached_separating_axis;
                let len_sqr = self.cached_separating_axis.length2();
                if len_sqr < 0.0001 {
                    self.degenerate_simplex = 5;
                }
                if len_sqr > SIMD_EPSILON * SIMD_EPSILON {
                    let rlen = 1.0 / len_sqr.sqrt();
                    normal_in_b *= rlen;
                    let s = squared_distance.sqrt();
                    point_on_a -= self.cached_separating_axis * (margin_a / s);
                    point_on_b += self.cached_separating_axis * (margin_b / s);
                    distance = (1.0 / rlen) - margin;
                    is_valid = true;
                    self.last_used_method = 1;
                } else {
                    self.last_used_method = 2;
                }
            }

            let catch_degenerate_penetration_case = self.catch_degeneracies != 0
                && self.penetration_depth_solver.is_some()
                && self.degenerate_simplex != 0
                && ((distance + margin) < 0.01);

            if check_penetration && (!is_valid || catch_degenerate_penetration_case) {
                if self.penetration_depth_solver.is_some() {
                    let mut tmp_point_on_a = Vec3::ZERO;
                    let mut tmp_point_on_b = Vec3::ZERO;
                    self.cached_separating_axis.set_zero();
                    let is_valid2 = gjk_epa_calc_pen_depth(
                        self.minkowski_a,
                        self.minkowski_b,
                        &local_trans_a,
                        &local_trans_b,
                        &mut self.cached_separating_axis,
                        &mut tmp_point_on_a,
                        &mut tmp_point_on_b,
                    );
                    if is_valid2 {
                        let mut tmp_normal_in_b = tmp_point_on_b - tmp_point_on_a;
                        let mut len_sqr = tmp_normal_in_b.length2();
                        if len_sqr <= SIMD_EPSILON * SIMD_EPSILON {
                            tmp_normal_in_b = self.cached_separating_axis;
                            len_sqr = self.cached_separating_axis.length2();
                        }
                        if len_sqr > SIMD_EPSILON * SIMD_EPSILON {
                            tmp_normal_in_b.div_assign_scalar(len_sqr.sqrt());
                            let distance2 = -(tmp_point_on_a - tmp_point_on_b).length();
                            if !is_valid || (distance2 < distance) {
                                distance = distance2;
                                point_on_a = tmp_point_on_a;
                                point_on_b = tmp_point_on_b;
                                normal_in_b = tmp_normal_in_b;
                                is_valid = true;
                                self.last_used_method = 3;
                            } else {
                                self.last_used_method = 8;
                            }
                        } else {
                            self.last_used_method = 9;
                        }
                    } else if self.cached_separating_axis.length2() > 0.0 {
                        let distance2 = (tmp_point_on_a - tmp_point_on_b).length() - margin;
                        if !is_valid || (distance2 < distance) {
                            distance = distance2;
                            point_on_a = tmp_point_on_a;
                            point_on_b = tmp_point_on_b;
                            point_on_a -= self.cached_separating_axis * margin_a;
                            point_on_b += self.cached_separating_axis * margin_b;
                            normal_in_b = self.cached_separating_axis;
                            normal_in_b.normalize();
                            is_valid = true;
                            self.last_used_method = 6;
                        } else {
                            self.last_used_method = 5;
                        }
                    }
                }
            }
        }

        if is_valid && ((distance < 0.0) || (distance * distance < input.maximum_distance_squared))
        {
            if self.fix_contact_normal_direction != 0 {
                let (aabb_min, aabb_max) = self.minkowski_a.get_aabb(&local_trans_a);
                let pos_a = (aabb_max + aabb_min) * 0.5;
                let (aabb_min, aabb_max) = self.minkowski_b.get_aabb(&local_trans_b);
                let pos_b = (aabb_min + aabb_max) * 0.5;
                let diff = pos_a - pos_b;
                if diff.dot(&normal_in_b) < 0.0 {
                    normal_in_b *= -1.0;
                }
            }
            self.cached_separating_axis = normal_in_b;
            self.cached_separating_distance = distance;
            output.add_contact_point(&normal_in_b, &(point_on_b + position_offset), distance);
        }
    }
}
