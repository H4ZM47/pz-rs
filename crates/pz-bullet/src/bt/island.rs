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
// Altered for pz-rs: Rust translation of btSimulationIslandManager
// (STATIC_SIMULATION_ISLAND_OPTIMIZATION, split islands) from Bullet 2.82.

//! `btSimulationIslandManager`.

use super::collision_object::*;
use super::collision_world::CollisionWorld;
use super::dispatch::{obj, CollisionDispatcher};
use super::linear_math::quick_sort;
use super::manifold::ManifoldId;
use super::union_find::UnionFind;

/// `IslandCallback::processIsland(bodies, manifolds, islandId)`.
pub type IslandCallback<'a> = dyn FnMut(&mut CollisionWorld, &[ObjId], &[ManifoldId], i32) + 'a;

/// `btSimulationIslandManager` with `m_splitIslands = true`.
#[derive(Clone, Debug, Default)]
pub struct SimulationIslandManager {
    pub union_find: UnionFind,
    pub island_manifold: Vec<ManifoldId>,
    pub island_bodies: Vec<ObjId>,
}

/// `getIslandId(manifold)`.
fn get_island_id(cw: &CollisionWorld, m: ManifoldId) -> i32 {
    let mf = cw.dispatcher.manifold(m);
    let t0 = cw.obj(mf.body0).island_tag1;
    if t0 >= 0 {
        t0
    } else {
        cw.obj(mf.body1).island_tag1
    }
}

impl SimulationIslandManager {
    pub fn new() -> SimulationIslandManager {
        SimulationIslandManager::default()
    }

    /// `findUnions`.
    fn find_unions(&mut self, cw: &CollisionWorld) {
        for pair in cw.broadphase.paircache.overlapping_pair_array.iter() {
            let o0 = cw.broadphase.proxy(pair.proxy0).base.client_object;
            let o1 = cw.broadphase.proxy(pair.proxy1).base.client_object;
            let c0 = cw.obj(o0);
            let c1 = cw.obj(o1);
            if c0.merges_simulation_islands() && c1.merges_simulation_islands() {
                self.union_find.unite(c0.island_tag1, c1.island_tag1);
            }
        }
    }

    /// `updateActivationState(colWorld, dispatcher)`.
    pub fn update_activation_state(&mut self, cw: &mut CollisionWorld) {
        let mut index = 0;
        for i in 0..cw.collision_objects.len() {
            let id = cw.collision_objects[i];
            let o = cw.obj_mut(id);
            if !o.is_static_or_kinematic_object() {
                o.island_tag1 = index;
                index += 1;
            }
            o.companion_id = -1;
            o.hit_fraction = 1.0;
        }
        self.union_find.reset(index as usize);
        self.find_unions(cw);
    }

    /// `storeIslandActivationState(colWorld)`.
    pub fn store_island_activation_state(&mut self, cw: &mut CollisionWorld) {
        let mut index = 0usize;
        for i in 0..cw.collision_objects.len() {
            let id = cw.collision_objects[i];
            if !cw.obj(id).is_static_or_kinematic_object() {
                let tag = self.union_find.find(index as i32);
                let o = cw.obj_mut(id);
                o.island_tag1 = tag;
                self.union_find.get_element_mut(index).sz = i as i32;
                o.companion_id = -1;
                index += 1;
            } else {
                let o = cw.obj_mut(id);
                o.island_tag1 = -1;
                o.companion_id = -2;
            }
        }
    }

    /// `buildIslands(dispatcher, collisionWorld)`.
    pub fn build_islands(&mut self, cw: &mut CollisionWorld) {
        self.island_manifold.clear();
        self.union_find.sort_islands();
        let num_elem = self.union_find.get_num_elements();
        let mut end_island_index;
        let mut start_island_index = 0;
        while start_island_index < num_elem {
            let island_id = self.union_find.get_element(start_island_index).id;
            end_island_index = start_island_index + 1;
            while end_island_index < num_elem
                && self.union_find.get_element(end_island_index).id == island_id
            {
                end_island_index += 1;
            }
            let mut all_sleeping = true;
            for idx in start_island_index..end_island_index {
                let i = self.union_find.get_element(idx).sz as usize;
                let o = cw.obj(cw.collision_objects[i]);
                if o.island_tag1 == island_id {
                    if o.get_activation_state() == ACTIVE_TAG {
                        all_sleeping = false;
                    }
                    if o.get_activation_state() == DISABLE_DEACTIVATION {
                        all_sleeping = false;
                    }
                }
            }
            if all_sleeping {
                for idx in start_island_index..end_island_index {
                    let i = self.union_find.get_element(idx).sz as usize;
                    let id = cw.collision_objects[i];
                    let o = cw.obj_mut(id);
                    if o.island_tag1 == island_id {
                        o.set_activation_state(ISLAND_SLEEPING);
                    }
                }
            } else {
                for idx in start_island_index..end_island_index {
                    let i = self.union_find.get_element(idx).sz as usize;
                    let id = cw.collision_objects[i];
                    let o = cw.obj_mut(id);
                    if o.island_tag1 == island_id && o.get_activation_state() == ISLAND_SLEEPING {
                        o.set_activation_state(WANTS_DEACTIVATION);
                        o.deactivation_time = 0.0;
                    }
                }
            }
            start_island_index = end_island_index;
        }

        let max_num_manifolds = cw.dispatcher.get_num_manifolds();
        for i in 0..max_num_manifolds {
            let m = cw.dispatcher.manifolds_ptr[i];
            let (b0, b1) = {
                let mf = cw.dispatcher.manifold(m);
                (mf.body0, mf.body1)
            };
            let s0 = cw.obj(b0).get_activation_state();
            let s1 = cw.obj(b1).get_activation_state();
            if s0 != ISLAND_SLEEPING || s1 != ISLAND_SLEEPING {
                if cw.obj(b0).is_kinematic_object()
                    && cw.obj(b0).get_activation_state() != ISLAND_SLEEPING
                    && cw.obj(b0).has_contact_response()
                {
                    cw.obj_mut(b1).activate(false);
                }
                if cw.obj(b1).is_kinematic_object()
                    && cw.obj(b1).get_activation_state() != ISLAND_SLEEPING
                    && cw.obj(b1).has_contact_response()
                {
                    cw.obj_mut(b0).activate(false);
                }
                if CollisionDispatcher::needs_response(obj(&cw.objects, b0), obj(&cw.objects, b1)) {
                    self.island_manifold.push(m);
                }
            }
        }
    }

    /// `buildAndProcessIslands(dispatcher, collisionWorld, callback)`;
    /// `callback(bodies, manifolds, islandId)`.
    pub fn build_and_process_islands(
        &mut self,
        cw: &mut CollisionWorld,
        callback: &mut IslandCallback,
    ) {
        self.build_islands(cw);
        let num_elem = self.union_find.get_num_elements();
        let num_manifolds = self.island_manifold.len();
        {
            let cwr: &CollisionWorld = cw;
            quick_sort(&mut self.island_manifold, |a, b| {
                get_island_id(cwr, *a) < get_island_id(cwr, *b)
            });
        }
        let mut start_manifold_index = 0usize;
        let mut end_manifold_index = 1usize;
        let mut start_island_index = 0usize;
        while start_island_index < num_elem {
            let island_id = self.union_find.get_element(start_island_index).id;
            let mut island_sleeping = true;
            let mut end_island_index = start_island_index;
            while end_island_index < num_elem
                && self.union_find.get_element(end_island_index).id == island_id
            {
                let i = self.union_find.get_element(end_island_index).sz as usize;
                let id = cw.collision_objects[i];
                self.island_bodies.push(id);
                if cw.obj(id).is_active() {
                    island_sleeping = false;
                }
                end_island_index += 1;
            }
            let mut num_island_manifolds = 0;
            let mut start_manifold = 0;
            if start_manifold_index < num_manifolds {
                let cur_island_id = get_island_id(cw, self.island_manifold[start_manifold_index]);
                if cur_island_id == island_id {
                    start_manifold = start_manifold_index;
                    end_manifold_index = start_manifold_index + 1;
                    while end_manifold_index < num_manifolds
                        && island_id == get_island_id(cw, self.island_manifold[end_manifold_index])
                    {
                        end_manifold_index += 1;
                    }
                    num_island_manifolds = end_manifold_index - start_manifold_index;
                }
            }
            if !island_sleeping {
                let bodies = std::mem::take(&mut self.island_bodies);
                let manifolds: Vec<ManifoldId> = self.island_manifold
                    [start_manifold..start_manifold + num_island_manifolds]
                    .to_vec();
                callback(cw, &bodies, &manifolds, island_id);
                self.island_bodies = bodies;
            }
            if num_island_manifolds != 0 {
                start_manifold_index = end_manifold_index;
            }
            self.island_bodies.clear();
            start_island_index = end_island_index;
        }
    }
}
