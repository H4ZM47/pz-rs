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
// Altered for pz-rs: Rust translation of btManifoldPoint and
// btPersistentManifold (Bullet 2.82, double precision).

//! `btManifoldPoint` and `btPersistentManifold`.
//!
//! Manifolds live in the dispatcher's arena and are addressed by
//! [`ManifoldId`]. `m_userPersistentData` is never set by the game
//! (no `gContactDestroyedCallback` / `gContactProcessedCallback`), so the
//! user-cache hooks are no-ops here.

use super::collision_object::ObjId;
use super::linear_math::*;

/// Index of a persistent manifold in the dispatcher's arena.
pub type ManifoldId = usize;

/// `gContactBreakingThreshold`.
pub const G_CONTACT_BREAKING_THRESHOLD: f64 = 0.02;
pub const MANIFOLD_CACHE_SIZE: usize = 4;

/// `btManifoldPoint`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ManifoldPoint {
    pub local_point_a: Vec3,
    pub local_point_b: Vec3,
    pub position_world_on_b: Vec3,
    pub position_world_on_a: Vec3,
    pub normal_world_on_b: Vec3,
    pub distance1: f64,
    pub combined_friction: f64,
    pub combined_rolling_friction: f64,
    pub combined_restitution: f64,
    pub part_id0: i32,
    pub part_id1: i32,
    pub index0: i32,
    pub index1: i32,
    pub lateral_friction_initialized: bool,
    pub applied_impulse: f64,
    pub applied_impulse_lateral1: f64,
    pub applied_impulse_lateral2: f64,
    pub contact_motion1: f64,
    pub contact_motion2: f64,
    pub contact_cfm1: f64,
    pub contact_cfm2: f64,
    pub life_time: i32,
    pub lateral_friction_dir1: Vec3,
    pub lateral_friction_dir2: Vec3,
}

impl ManifoldPoint {
    /// `btManifoldPoint(pointA, pointB, normal, distance)`. Fields the C++
    /// constructor leaves uninitialised are zero.
    pub fn new(point_a: Vec3, point_b: Vec3, normal: Vec3, distance: f64) -> ManifoldPoint {
        ManifoldPoint {
            local_point_a: point_a,
            local_point_b: point_b,
            normal_world_on_b: normal,
            distance1: distance,
            ..Default::default()
        }
    }
    #[inline(always)]
    pub fn get_distance(&self) -> f64 {
        self.distance1
    }
}

/// `btPersistentManifold`.
#[derive(Clone, Debug, Default)]
pub struct PersistentManifold {
    pub point_cache: [ManifoldPoint; MANIFOLD_CACHE_SIZE],
    pub body0: ObjId,
    pub body1: ObjId,
    pub cached_points: i32,
    pub contact_breaking_threshold: f64,
    pub contact_processing_threshold: f64,
    pub companion_id_a: i32,
    pub companion_id_b: i32,
    pub index1a: i32,
}

/// `gContactCalcArea3Points`.
const G_CONTACT_CALC_AREA3_POINTS: bool = true;

impl PersistentManifold {
    pub fn new(
        body0: ObjId,
        body1: ObjId,
        contact_breaking_threshold: f64,
        contact_processing_threshold: f64,
    ) -> PersistentManifold {
        PersistentManifold {
            body0,
            body1,
            cached_points: 0,
            contact_breaking_threshold,
            contact_processing_threshold,
            ..Default::default()
        }
    }
    #[inline(always)]
    pub fn get_num_contacts(&self) -> i32 {
        self.cached_points
    }
    #[inline(always)]
    pub fn get_contact_point(&self, i: usize) -> &ManifoldPoint {
        &self.point_cache[i]
    }
    #[inline(always)]
    pub fn get_contact_point_mut(&mut self, i: usize) -> &mut ManifoldPoint {
        &mut self.point_cache[i]
    }
    #[inline(always)]
    pub fn get_contact_breaking_threshold(&self) -> f64 {
        self.contact_breaking_threshold
    }
    #[inline(always)]
    pub fn valid_contact_distance(&self, pt: &ManifoldPoint) -> bool {
        pt.distance1 <= self.get_contact_breaking_threshold()
    }

    /// `sortCachedPoints` (with `KEEP_DEEPEST_POINT` and
    /// `gContactCalcArea3Points = true`).
    pub fn sort_cached_points(&self, pt: &ManifoldPoint) -> i32 {
        let pc = &self.point_cache;
        let mut max_penetration_index = -1;
        let mut max_penetration = pt.get_distance();
        for (i, p) in pc.iter().enumerate() {
            if p.get_distance() < max_penetration {
                max_penetration_index = i as i32;
                max_penetration = p.get_distance();
            }
        }
        let (mut res0, mut res1, mut res2, mut res3) = (0.0, 0.0, 0.0, 0.0);
        if G_CONTACT_CALC_AREA3_POINTS {
            if max_penetration_index != 0 {
                let a0 = pt.local_point_a - pc[1].local_point_a;
                let b0 = pc[3].local_point_a - pc[2].local_point_a;
                res0 = a0.cross(&b0).length2();
            }
            if max_penetration_index != 1 {
                let a1 = pt.local_point_a - pc[0].local_point_a;
                let b1 = pc[3].local_point_a - pc[2].local_point_a;
                res1 = a1.cross(&b1).length2();
            }
            if max_penetration_index != 2 {
                let a2 = pt.local_point_a - pc[0].local_point_a;
                let b2 = pc[3].local_point_a - pc[1].local_point_a;
                res2 = a2.cross(&b2).length2();
            }
            if max_penetration_index != 3 {
                let a3 = pt.local_point_a - pc[0].local_point_a;
                let b3 = pc[2].local_point_a - pc[1].local_point_a;
                res3 = a3.cross(&b3).length2();
            }
        }
        Vec3::new4(res0, res1, res2, res3).closest_axis4()
    }

    /// `getCacheEntry`.
    pub fn get_cache_entry(&self, new_point: &ManifoldPoint) -> i32 {
        let mut shortest_dist =
            self.get_contact_breaking_threshold() * self.get_contact_breaking_threshold();
        let mut nearest_point = -1;
        for i in 0..self.cached_points as usize {
            let mp = &self.point_cache[i];
            let diff_a = mp.local_point_a - new_point.local_point_a;
            let d = diff_a.dot(&diff_a);
            if d < shortest_dist {
                shortest_dist = d;
                nearest_point = i as i32;
            }
        }
        nearest_point
    }

    /// `addManifoldPoint` (returns the insert index).
    pub fn add_manifold_point(&mut self, new_point: &ManifoldPoint) -> i32 {
        let mut insert_index = self.cached_points;
        if insert_index == MANIFOLD_CACHE_SIZE as i32 {
            insert_index = self.sort_cached_points(new_point);
        } else {
            self.cached_points += 1;
        }
        if insert_index < 0 {
            insert_index = 0;
        }
        self.point_cache[insert_index as usize] = *new_point;
        insert_index
    }

    /// `removeContactPoint`.
    pub fn remove_contact_point(&mut self, index: usize) {
        let last = (self.cached_points - 1) as usize;
        if index != last {
            self.point_cache[index] = self.point_cache[last];
            let p = &mut self.point_cache[last];
            p.applied_impulse = 0.0;
            p.lateral_friction_initialized = false;
            p.applied_impulse_lateral1 = 0.0;
            p.applied_impulse_lateral2 = 0.0;
            p.life_time = 0;
        }
        self.cached_points -= 1;
    }

    /// `replaceContactPoint` (`MAINTAIN_PERSISTENCY`).
    pub fn replace_contact_point(&mut self, new_point: &ManifoldPoint, insert_index: usize) {
        let old = &self.point_cache[insert_index];
        let life_time = old.life_time;
        let applied_impulse = old.applied_impulse;
        let applied_lateral_impulse1 = old.applied_impulse_lateral1;
        let applied_lateral_impulse2 = old.applied_impulse_lateral2;
        let p = &mut self.point_cache[insert_index];
        *p = *new_point;
        p.applied_impulse = applied_impulse;
        p.applied_impulse_lateral1 = applied_lateral_impulse1;
        p.applied_impulse_lateral2 = applied_lateral_impulse2;
        p.life_time = life_time;
    }

    /// `clearManifold`.
    #[inline(always)]
    pub fn clear_manifold(&mut self) {
        self.cached_points = 0;
    }

    /// `refreshContactPoints(trA, trB)`.
    pub fn refresh_contact_points(&mut self, tr_a: &Transform, tr_b: &Transform) {
        let n = self.cached_points;
        let mut i = n - 1;
        while i >= 0 {
            let mp = &mut self.point_cache[i as usize];
            mp.position_world_on_a = tr_a.apply(&mp.local_point_a);
            mp.position_world_on_b = tr_b.apply(&mp.local_point_b);
            mp.distance1 =
                (mp.position_world_on_a - mp.position_world_on_b).dot(&mp.normal_world_on_b);
            mp.life_time += 1;
            i -= 1;
        }
        let mut i = self.cached_points - 1;
        while i >= 0 {
            let mp = self.point_cache[i as usize];
            if !self.valid_contact_distance(&mp) {
                self.remove_contact_point(i as usize);
            } else {
                let projected_point = mp.position_world_on_a - mp.normal_world_on_b * mp.distance1;
                let projected_difference = mp.position_world_on_b - projected_point;
                let distance2d = projected_difference.dot(&projected_difference);
                let t = self.get_contact_breaking_threshold();
                if distance2d > t * t {
                    self.remove_contact_point(i as usize);
                }
            }
            i -= 1;
        }
    }
}
