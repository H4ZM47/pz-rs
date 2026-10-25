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
// Altered for pz-rs: Rust translation of btConvexCast::CastResult,
// btSubsimplexConvexCast, btGjkConvexCast and btContinuousConvexCollision
// (convex-convex form) from Bullet 2.82, double precision.

//! Convex casts used by `btCollisionWorld::rayTestSingle` (GJK cast with a
//! point sphere, subsimplex cast with `kF_UseSubSimplexConvexCastRaytest`)
//! and `objectQuerySingle` (continuous convex collision with EPA).

use super::gjk_pair_detector::{
    ClosestPointInput, DiscreteResult, GjkPairDetector, PenetrationDepthSolver, PointCollector,
};
use super::simplex::VoronoiSimplexSolver;
use crate::bt::linear_math::transform_util::{calculate_velocity, integrate_transform};
use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;

/// `btConvexCast::CastResult` (debug hooks dropped).
#[derive(Clone, Copy, Debug)]
pub struct CastResult {
    pub hit_transform_a: Transform,
    pub hit_transform_b: Transform,
    pub normal: Vec3,
    pub hit_point: Vec3,
    pub fraction: f64,
    pub allowed_penetration: f64,
}

impl Default for CastResult {
    fn default() -> Self {
        CastResult {
            hit_transform_a: Transform::default(),
            hit_transform_b: Transform::default(),
            normal: Vec3::ZERO,
            hit_point: Vec3::ZERO,
            fraction: BT_LARGE_FLOAT,
            allowed_penetration: 0.0,
        }
    }
}

const MAX_ITERATIONS: i32 = 64;

/// `btSubsimplexConvexCast::calcTimeOfImpact`.
pub fn subsimplex_convex_cast(
    convex_a: &Shape,
    convex_b: &Shape,
    simplex_solver: &mut VoronoiSimplexSolver,
    from_a: &Transform,
    to_a: &Transform,
    from_b: &Transform,
    to_b: &Transform,
    result: &mut CastResult,
) -> bool {
    simplex_solver.reset();
    let lin_vel_a = to_a.origin - from_a.origin;
    let lin_vel_b = to_b.origin - from_b.origin;
    let mut lambda = 0.0;
    let mut interpolated_trans_a = *from_a;
    let mut interpolated_trans_b = *from_b;
    let r = lin_vel_a - lin_vel_b;
    let mut sup_vertex_a =
        from_a.apply(&convex_a.local_get_supporting_vertex(&((-r) * from_a.basis)));
    let mut sup_vertex_b = from_b.apply(&convex_b.local_get_supporting_vertex(&(r * from_b.basis)));
    let mut v = sup_vertex_a - sup_vertex_b;
    let mut max_iter = MAX_ITERATIONS;
    let mut n = Vec3::ZERO;
    let mut dist2 = v.length2();
    let epsilon = 0.0001;
    let mut w;
    loop {
        if !(dist2 > epsilon) {
            break;
        }
        let go = max_iter != 0;
        max_iter -= 1;
        if !go {
            break;
        }
        sup_vertex_a = interpolated_trans_a
            .apply(&convex_a.local_get_supporting_vertex(&((-v) * interpolated_trans_a.basis)));
        sup_vertex_b = interpolated_trans_b
            .apply(&convex_b.local_get_supporting_vertex(&(v * interpolated_trans_b.basis)));
        w = sup_vertex_a - sup_vertex_b;
        let vdotw = v.dot(&w);
        if lambda > 1.0 {
            return false;
        }
        if vdotw > 0.0 {
            let vdotr = v.dot(&r);
            if vdotr >= -(SIMD_EPSILON * SIMD_EPSILON) {
                return false;
            } else {
                lambda -= vdotw / vdotr;
                interpolated_trans_a
                    .origin
                    .set_interpolate3(&from_a.origin, &to_a.origin, lambda);
                interpolated_trans_b
                    .origin
                    .set_interpolate3(&from_b.origin, &to_b.origin, lambda);
                w = sup_vertex_a - sup_vertex_b;
                n = v;
            }
        }
        if !simplex_solver.in_simplex(&w) {
            simplex_solver.add_vertex(&w, &sup_vertex_a, &sup_vertex_b);
        }
        if simplex_solver.closest(&mut v) {
            dist2 = v.length2();
        } else {
            dist2 = 0.0;
        }
    }
    result.fraction = lambda;
    if n.length2() >= SIMD_EPSILON * SIMD_EPSILON {
        result.normal = n.normalized();
    } else {
        result.normal = Vec3::ZERO;
    }
    if result.normal.dot(&r) >= -result.allowed_penetration {
        return false;
    }
    let mut hit_a = Vec3::ZERO;
    let mut hit_b = Vec3::ZERO;
    simplex_solver.compute_points(&mut hit_a, &mut hit_b);
    result.hit_point = hit_b;
    true
}

/// `btGjkConvexCast::calcTimeOfImpact`.
pub fn gjk_convex_cast(
    convex_a: &Shape,
    convex_b: &Shape,
    simplex_solver: &mut VoronoiSimplexSolver,
    from_a: &Transform,
    to_a: &Transform,
    from_b: &Transform,
    to_b: &Transform,
    result: &mut CastResult,
) -> bool {
    simplex_solver.reset();
    let lin_vel_a = to_a.origin - from_a.origin;
    let lin_vel_b = to_b.origin - from_b.origin;
    let radius = 0.001;
    let mut lambda = 0.0;
    let max_iter = MAX_ITERATIONS;
    let mut n;
    let r = lin_vel_a - lin_vel_b;
    let mut last_lambda = lambda;
    let mut num_iter = 0;
    let mut point_collector = PointCollector::default();
    let mut gjk = GjkPairDetector::new(convex_a, convex_b, simplex_solver, None);
    let mut input = ClosestPointInput {
        transform_a: *from_a,
        transform_b: *from_b,
        ..Default::default()
    };
    gjk.get_closest_points(&input, &mut point_collector);
    let has_result = point_collector.has_result;
    let mut c = point_collector.point_in_world;
    if has_result {
        let mut dist = point_collector.distance;
        n = point_collector.normal_on_b_in_world;
        while dist > radius {
            num_iter += 1;
            if num_iter > max_iter {
                return false;
            }
            let projected_linear_velocity = r.dot(&n);
            let d_lambda = dist / projected_linear_velocity;
            lambda -= d_lambda;
            if lambda > 1.0 {
                return false;
            }
            if lambda < 0.0 {
                return false;
            }
            if lambda <= last_lambda {
                return false;
            }
            last_lambda = lambda;
            input
                .transform_a
                .origin
                .set_interpolate3(&from_a.origin, &to_a.origin, lambda);
            input
                .transform_b
                .origin
                .set_interpolate3(&from_b.origin, &to_b.origin, lambda);
            gjk.get_closest_points(&input, &mut point_collector);
            if point_collector.has_result {
                if point_collector.distance < 0.0 {
                    result.fraction = last_lambda;
                    n = point_collector.normal_on_b_in_world;
                    result.normal = n;
                    result.hit_point = point_collector.point_in_world;
                    return true;
                }
                c = point_collector.point_in_world;
                n = point_collector.normal_on_b_in_world;
                dist = point_collector.distance;
            } else {
                return false;
            }
        }
        if n.dot(&r) >= -result.allowed_penetration {
            return false;
        }
        result.fraction = lambda;
        result.normal = n;
        result.hit_point = c;
        return true;
    }
    false
}

/// `btContinuousConvexCollision::computeClosestPoints` (convex B).
fn continuous_compute_closest_points(
    convex_a: &Shape,
    convex_b: &Shape,
    simplex_solver: &mut VoronoiSimplexSolver,
    pd: Option<PenetrationDepthSolver>,
    trans_a: &Transform,
    trans_b: &Transform,
    point_collector: &mut dyn DiscreteResult,
) {
    simplex_solver.reset();
    let mut gjk = GjkPairDetector::with_params(
        convex_a,
        convex_b,
        convex_a.shape_type(),
        convex_b.shape_type(),
        convex_a.get_margin(),
        convex_b.get_margin(),
        simplex_solver,
        pd,
    );
    let input = ClosestPointInput {
        transform_a: *trans_a,
        transform_b: *trans_b,
        ..Default::default()
    };
    gjk.get_closest_points(&input, point_collector);
}

/// `btContinuousConvexCollision::calcTimeOfImpact` (convex A against convex B).
#[allow(clippy::too_many_arguments)]
pub fn continuous_convex_collision(
    convex_a: &Shape,
    convex_b: &Shape,
    simplex_solver: &mut VoronoiSimplexSolver,
    penetration_depth_solver: Option<PenetrationDepthSolver>,
    from_a: &Transform,
    to_a: &Transform,
    from_b: &Transform,
    to_b: &Transform,
    result: &mut CastResult,
) -> bool {
    let (lin_vel_a, ang_vel_a) = calculate_velocity(from_a, to_a, 1.0);
    let (lin_vel_b, ang_vel_b) = calculate_velocity(from_b, to_b, 1.0);
    let bounding_radius_a = convex_a.get_angular_motion_disc();
    let bounding_radius_b = convex_b.get_angular_motion_disc();
    let max_angular_projected_velocity = fma(
        ang_vel_a.length(),
        bounding_radius_a,
        ang_vel_b.length() * bounding_radius_b,
    );
    let rel_lin_vel = lin_vel_b - lin_vel_a;
    let rel_lin_veloc_length = (lin_vel_b - lin_vel_a).length();
    if (rel_lin_veloc_length + max_angular_projected_velocity) == 0.0 {
        return false;
    }
    let mut lambda = 0.0;
    let max_iter = MAX_ITERATIONS;
    let mut n;
    let mut c;
    let mut last_lambda = lambda;
    let mut num_iter = 0;
    let radius = flt(0.001);
    let mut point_collector1 = PointCollector::default();
    continuous_compute_closest_points(
        convex_a,
        convex_b,
        simplex_solver,
        penetration_depth_solver,
        from_a,
        from_b,
        &mut point_collector1,
    );
    let has_result = point_collector1.has_result;
    c = point_collector1.point_in_world;
    if has_result {
        let mut dist = point_collector1.distance + result.allowed_penetration;
        n = point_collector1.normal_on_b_in_world;
        let mut projected_linear_velocity = rel_lin_vel.dot(&n);
        if (projected_linear_velocity + max_angular_projected_velocity) <= SIMD_EPSILON {
            return false;
        }
        while dist > radius {
            projected_linear_velocity = rel_lin_vel.dot(&n);
            if (projected_linear_velocity + max_angular_projected_velocity) <= SIMD_EPSILON {
                return false;
            }
            let d_lambda = dist / (projected_linear_velocity + max_angular_projected_velocity);
            lambda += d_lambda;
            if lambda > 1.0 {
                return false;
            }
            if lambda < 0.0 {
                return false;
            }
            if lambda <= last_lambda {
                return false;
            }
            last_lambda = lambda;
            let interpolated_trans_a = integrate_transform(from_a, &lin_vel_a, &ang_vel_a, lambda);
            let interpolated_trans_b = integrate_transform(from_b, &lin_vel_b, &ang_vel_b, lambda);
            let mut point_collector = PointCollector::default();
            continuous_compute_closest_points(
                convex_a,
                convex_b,
                simplex_solver,
                penetration_depth_solver,
                &interpolated_trans_a,
                &interpolated_trans_b,
                &mut point_collector,
            );
            if point_collector.has_result {
                dist = point_collector.distance + result.allowed_penetration;
                c = point_collector.point_in_world;
                n = point_collector.normal_on_b_in_world;
            } else {
                return false;
            }
            num_iter += 1;
            if num_iter > max_iter {
                return false;
            }
        }
        result.fraction = lambda;
        result.normal = n;
        result.hit_point = c;
        return true;
    }
    false
}
