// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2009 Erwin Coumans  http://bulletphysics.org
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
// btDbvtBroadphase implementation by Nathanael Presson.
// Altered for pz-rs: Rust translation of btDbvtBroadphase (Bullet 2.82,
// double precision) with its own btHashedOverlappingPairCache.

//! `btDbvtBroadphase` with the default `btHashedOverlappingPairCache`
//! (`DBVT_BP_SORTPAIRS` off, `DBVT_BP_MARGIN` 0.05, no profiling).

use super::dbvt::*;
use super::pair_cache::*;
use crate::bt::collision_object::ProxyId;
use crate::bt::linear_math::*;

pub const DYNAMIC_SET: usize = 0;
pub const FIXED_SET: usize = 1;
pub const STAGECOUNT: usize = 2;
/// `DBVT_BP_MARGIN`.
pub const DBVT_BP_MARGIN: f64 = 0.05;

/// `btDbvtProxy`.
#[derive(Clone, Copy, Debug)]
pub struct DbvtProxy {
    pub base: BroadphaseProxy,
    pub leaf: NodeId,
    /// `links[0]`, `links[1]` (stage list).
    pub links: [Option<ProxyId>; 2],
    pub stage: usize,
}

/// `btBroadphaseRayCallback`.
pub trait BroadphaseRayCallback {
    fn ray_direction_inverse(&self) -> Vec3;
    fn signs(&self) -> [u32; 3];
    /// `m_lambda_max`, read once per tree.
    fn lambda_max(&self) -> f64;
    /// `process(proxy)`.
    fn process(&mut self, proxy_id: ProxyId, proxy: &BroadphaseProxy) -> bool;
}

/// `btDbvtBroadphase`.
#[derive(Debug)]
pub struct DbvtBroadphase {
    pub sets: [Dbvt; 2],
    pub stage_roots: [Option<ProxyId>; STAGECOUNT + 1],
    pub paircache: HashedOverlappingPairCache,
    pub proxies: Vec<Option<DbvtProxy>>,
    free_proxy_slots: Vec<ProxyId>,
    pub prediction: f64,
    pub stage_current: usize,
    pub fupdates: i32,
    pub dupdates: i32,
    pub cupdates: i32,
    pub newpairs: i32,
    pub fixedleft: i32,
    pub updates_call: u32,
    pub updates_done: u32,
    pub updates_ratio: f64,
    pub pid: i32,
    pub cid: i32,
    pub gid: i32,
    pub deferedcollide: bool,
    pub needcleanup: bool,
}

impl Default for DbvtBroadphase {
    fn default() -> Self {
        DbvtBroadphase::new()
    }
}

impl DbvtBroadphase {
    /// `btDbvtBroadphase(0)`.
    pub fn new() -> DbvtBroadphase {
        DbvtBroadphase {
            sets: [Dbvt::new(), Dbvt::new()],
            stage_roots: [None; STAGECOUNT + 1],
            paircache: HashedOverlappingPairCache::new(),
            proxies: Vec::new(),
            free_proxy_slots: Vec::new(),
            prediction: 0.0,
            stage_current: 0,
            fupdates: 1,
            dupdates: 0,
            cupdates: 10,
            newpairs: 1,
            fixedleft: 0,
            updates_call: 0,
            updates_done: 0,
            updates_ratio: 0.0,
            pid: 0,
            cid: 0,
            gid: 0,
            deferedcollide: false,
            needcleanup: true,
        }
    }

    #[inline(always)]
    pub fn proxy(&self, id: ProxyId) -> &DbvtProxy {
        self.proxies[id].as_ref().expect("dead proxy")
    }
    #[inline(always)]
    fn proxy_mut(&mut self, id: ProxyId) -> &mut DbvtProxy {
        self.proxies[id].as_mut().expect("dead proxy")
    }

    fn listappend(&mut self, item: ProxyId, list: usize) {
        let head = self.stage_roots[list];
        {
            let p = self.proxy_mut(item);
            p.links[0] = None;
            p.links[1] = head;
        }
        if let Some(h) = head {
            self.proxy_mut(h).links[0] = Some(item);
        }
        self.stage_roots[list] = Some(item);
    }

    fn listremove(&mut self, item: ProxyId, list: usize) {
        let [l0, l1] = self.proxy(item).links;
        match l0 {
            Some(p) => self.proxy_mut(p).links[1] = l1,
            None => self.stage_roots[list] = l1,
        }
        if let Some(n) = l1 {
            self.proxy_mut(n).links[0] = l0;
        }
    }

    /// `btDbvtTreeCollider::Process(na, nb)` with `na` in `sets[ta]`,
    /// `nb` in `sets[tb]`.
    fn tree_collide(&mut self, ta: usize, na: NodeId, tb: usize, nb: NodeId) {
        if ta == tb && na == nb {
            return;
        }
        let pa = self.sets[ta].node(na).data;
        let pb = self.sets[tb].node(nb).data;
        let a = self.proxy(pa).base;
        let b = self.proxy(pb).base;
        self.paircache.add_overlapping_pair(pa, &a, pb, &b);
        self.newpairs += 1;
    }

    /// `createProxy(aabbMin, aabbMax, shapeType, userPtr, group, mask, ...)`.
    pub fn create_proxy(
        &mut self,
        aabb_min: &Vec3,
        aabb_max: &Vec3,
        client_object: usize,
        collision_filter_group: i16,
        collision_filter_mask: i16,
    ) -> ProxyId {
        let aabb = DbvtVolume::from_mm(aabb_min, aabb_max);
        self.gid += 1;
        let proxy = DbvtProxy {
            base: BroadphaseProxy {
                client_object,
                collision_filter_group,
                collision_filter_mask,
                unique_id: self.gid,
                aabb_min: *aabb_min,
                aabb_max: *aabb_max,
            },
            leaf: NIL,
            links: [None, None],
            stage: self.stage_current,
        };
        let id = if let Some(s) = self.free_proxy_slots.pop() {
            self.proxies[s] = Some(proxy);
            s
        } else {
            self.proxies.push(Some(proxy));
            self.proxies.len() - 1
        };
        let leaf = self.sets[0].insert(&aabb, id);
        self.proxy_mut(id).leaf = leaf;
        let sc = self.stage_current;
        self.listappend(id, sc);
        if !self.deferedcollide {
            for set in 0..2 {
                let mut hits = Vec::new();
                let root = self.sets[set].root;
                self.sets[set].collide_tv(root, &aabb, &mut |n| hits.push(n));
                for n in hits {
                    self.tree_collide(set, n, 0, leaf);
                }
            }
        }
        id
    }

    /// `destroyProxy(proxy, dispatcher)`.
    pub fn destroy_proxy(&mut self, id: ProxyId, dispatcher: Option<&mut dyn PairDispatcher>) {
        let p = *self.proxy(id);
        if p.stage == STAGECOUNT {
            self.sets[1].remove(p.leaf);
        } else {
            self.sets[0].remove(p.leaf);
        }
        self.listremove(id, p.stage);
        self.paircache
            .remove_overlapping_pairs_containing_proxy(id, dispatcher);
        self.proxies[id] = None;
        self.free_proxy_slots.push(id);
        self.needcleanup = true;
    }

    /// `getAabb(proxy)`.
    pub fn get_aabb(&self, id: ProxyId) -> (Vec3, Vec3) {
        let p = self.proxy(id);
        (p.base.aabb_min, p.base.aabb_max)
    }

    /// Runs `btDbvtTreeCollider` over `collideTTpersistentStack(sets[t].root, leaf)`.
    fn collide_tt_with_leaf(&mut self, t: usize, leaf: NodeId) {
        let mut hits: Vec<(NodeId, NodeId)> = Vec::new();
        let root = self.sets[t].root;
        if t == 0 {
            self.sets[0]
                .collide_tt_persistent_stack(root, None, leaf, &mut |a, b| hits.push((a, b)));
        } else {
            let (s0, s1) = self.sets.split_at_mut(1);
            s1[0].collide_tt_persistent_stack(root, Some(&s0[0]), leaf, &mut |a, b| {
                hits.push((a, b))
            });
        }
        for (a, b) in hits {
            self.tree_collide(t, a, 0, b);
        }
    }

    /// `setAabb(proxy, aabbMin, aabbMax, dispatcher)`.
    pub fn set_aabb(&mut self, id: ProxyId, aabb_min: &Vec3, aabb_max: &Vec3) {
        let mut aabb = DbvtVolume::from_mm(aabb_min, aabb_max);
        let mut docollide = false;
        let p = *self.proxy(id);
        if p.stage == STAGECOUNT {
            self.sets[1].remove(p.leaf);
            let leaf = self.sets[0].insert(&aabb, id);
            self.proxy_mut(id).leaf = leaf;
            docollide = true;
        } else {
            self.updates_call = self.updates_call.wrapping_add(1);
            if intersect(&self.sets[0].node(p.leaf).volume, &aabb) {
                let delta = *aabb_min - p.base.aabb_min;
                let mut velocity = ((p.base.aabb_max - p.base.aabb_min) / 2.0) * self.prediction;
                if delta.x < 0.0 {
                    velocity.x = -velocity.x;
                }
                if delta.y < 0.0 {
                    velocity.y = -velocity.y;
                }
                if delta.z < 0.0 {
                    velocity.z = -velocity.z;
                }
                if self.sets[0].update_velocity_margin(p.leaf, &mut aabb, &velocity, DBVT_BP_MARGIN)
                {
                    self.updates_done = self.updates_done.wrapping_add(1);
                    docollide = true;
                }
            } else {
                self.sets[0].update(p.leaf, &aabb);
                self.updates_done = self.updates_done.wrapping_add(1);
                docollide = true;
            }
        }
        self.finish_set_aabb(id, aabb_min, aabb_max, docollide);
    }

    fn finish_set_aabb(&mut self, id: ProxyId, aabb_min: &Vec3, aabb_max: &Vec3, docollide: bool) {
        let stage = self.proxy(id).stage;
        self.listremove(id, stage);
        let sc = self.stage_current;
        {
            let p = self.proxy_mut(id);
            p.base.aabb_min = *aabb_min;
            p.base.aabb_max = *aabb_max;
            p.stage = sc;
        }
        self.listappend(id, sc);
        if docollide {
            self.needcleanup = true;
            if !self.deferedcollide {
                let leaf = self.proxy(id).leaf;
                self.collide_tt_with_leaf(1, leaf);
                self.collide_tt_with_leaf(0, leaf);
            }
        }
    }

    /// `setAabbForceUpdate(proxy, aabbMin, aabbMax, dispatcher)`.
    pub fn set_aabb_force_update(&mut self, id: ProxyId, aabb_min: &Vec3, aabb_max: &Vec3) {
        let aabb = DbvtVolume::from_mm(aabb_min, aabb_max);
        let p = *self.proxy(id);
        if p.stage == STAGECOUNT {
            self.sets[1].remove(p.leaf);
            let leaf = self.sets[0].insert(&aabb, id);
            self.proxy_mut(id).leaf = leaf;
        } else {
            self.updates_call = self.updates_call.wrapping_add(1);
            self.sets[0].update(p.leaf, &aabb);
            self.updates_done = self.updates_done.wrapping_add(1);
        }
        self.finish_set_aabb(id, aabb_min, aabb_max, true);
    }

    /// `rayTest(rayFrom, rayTo, rayCallback, aabbMin, aabbMax)`.
    pub fn ray_test(
        &self,
        ray_from: &Vec3,
        _ray_to: &Vec3,
        cb: &mut dyn BroadphaseRayCallback,
        aabb_min: &Vec3,
        aabb_max: &Vec3,
    ) {
        for set in 0..2 {
            let inv = cb.ray_direction_inverse();
            let signs = cb.signs();
            let lambda_max = cb.lambda_max();
            let t = &self.sets[set];
            t.ray_test_internal(
                t.root,
                ray_from,
                &inv,
                &signs,
                lambda_max,
                aabb_min,
                aabb_max,
                &mut |n| {
                    let pid = t.node(n).data;
                    cb.process(pid, &self.proxy(pid).base);
                },
            );
        }
    }

    /// `aabbTest(aabbMin, aabbMax, callback)`.
    pub fn aabb_test(
        &self,
        aabb_min: &Vec3,
        aabb_max: &Vec3,
        cb: &mut dyn FnMut(ProxyId, &BroadphaseProxy) -> bool,
    ) {
        let bounds = DbvtVolume::from_mm(aabb_min, aabb_max);
        for set in 0..2 {
            let t = &self.sets[set];
            t.collide_tv(t.root, &bounds, &mut |n| {
                let pid = t.node(n).data;
                cb(pid, &self.proxy(pid).base);
            });
        }
    }

    /// `calculateOverlappingPairs(dispatcher)`: `collide` (the hashed pair
    /// cache has no deferred removal, so `performDeferredRemoval` is a no-op).
    pub fn calculate_overlapping_pairs(&mut self, dispatcher: Option<&mut dyn PairDispatcher>) {
        self.collide(dispatcher);
    }

    /// `collide(dispatcher)`.
    pub fn collide(&mut self, mut dispatcher: Option<&mut dyn PairDispatcher>) {
        // optimize
        let n0 = 1 + (self.sets[0].leaves * self.dupdates) / 100;
        self.sets[0].optimize_incremental(n0);
        if self.fixedleft != 0 {
            let count = 1 + (self.sets[1].leaves * self.fupdates) / 100;
            self.sets[1].optimize_incremental(1 + (self.sets[1].leaves * self.fupdates) / 100);
            self.fixedleft = bt_max(0, self.fixedleft - count);
        }
        // dynamic -> fixed set
        self.stage_current = (self.stage_current + 1) % STAGECOUNT;
        let mut current = self.stage_roots[self.stage_current];
        if current.is_some() {
            while let Some(cur) = current {
                let next = self.proxy(cur).links[1];
                let st = self.proxy(cur).stage;
                self.listremove(cur, st);
                self.listappend(cur, STAGECOUNT);
                let leaf = self.proxy(cur).leaf;
                self.sets[0].remove(leaf);
                let b = self.proxy(cur).base;
                let cur_aabb = DbvtVolume::from_mm(&b.aabb_min, &b.aabb_max);
                let nl = self.sets[1].insert(&cur_aabb, cur);
                let p = self.proxy_mut(cur);
                p.leaf = nl;
                p.stage = STAGECOUNT;
                current = next;
            }
            self.fixedleft = self.sets[1].leaves;
            self.needcleanup = true;
        }
        // collide dynamics
        if self.deferedcollide {
            let mut hits: Vec<(NodeId, NodeId)> = Vec::new();
            let (s0, s1) = self.sets.split_at_mut(1);
            let r0 = s0[0].root;
            let r1 = s1[0].root;
            s0[0].collide_tt_persistent_stack(r0, Some(&s1[0]), r1, &mut |a, b| hits.push((a, b)));
            for (a, b) in hits {
                self.tree_collide(0, a, 1, b);
            }
            let mut hits: Vec<(NodeId, NodeId)> = Vec::new();
            let r0 = self.sets[0].root;
            self.sets[0].collide_tt_persistent_stack(r0, None, r0, &mut |a, b| hits.push((a, b)));
            for (a, b) in hits {
                self.tree_collide(0, a, 0, b);
            }
        }
        // clean up
        if self.needcleanup {
            let size = self.paircache.overlapping_pair_array.len() as i32;
            if size > 0 {
                let mut ni = bt_min(size, bt_max(self.newpairs, (size * self.cupdates) / 100));
                let mut i = 0;
                while i < ni {
                    let len = self.paircache.overlapping_pair_array.len() as i32;
                    let p = self.paircache.overlapping_pair_array[((self.cid + i) % len) as usize];
                    let pa = self.proxy(p.proxy0);
                    let pb = self.proxy(p.proxy1);
                    if !intersect(&self.sets_volume(pa), &self.sets_volume(pb)) {
                        let (a, b) = (pa.base, pb.base);
                        self.paircache
                            .remove_overlapping_pair(&a, &b, rb(&mut dispatcher));
                        ni -= 1;
                        i -= 1;
                    }
                    i += 1;
                }
                let len = self.paircache.overlapping_pair_array.len() as i32;
                if len > 0 {
                    self.cid = (self.cid + ni) % len;
                } else {
                    self.cid = 0;
                }
            }
        }
        self.pid += 1;
        self.newpairs = 1;
        self.needcleanup = false;
        if self.updates_call > 0 {
            self.updates_ratio = self.updates_done as f64 / self.updates_call as f64;
        } else {
            self.updates_ratio = 0.0;
        }
        self.updates_done /= 2;
        self.updates_call /= 2;
    }

    /// The volume of a proxy's leaf (`proxy->leaf->volume`).
    #[inline(always)]
    pub fn sets_volume(&self, p: &DbvtProxy) -> DbvtVolume {
        let set = if p.stage == STAGECOUNT { 1 } else { 0 };
        self.sets[set].node(p.leaf).volume
    }

    /// `optimize()`.
    pub fn optimize(&mut self) {
        self.sets[0].optimize_top_down(128);
        self.sets[1].optimize_top_down(128);
    }

    /// `getBroadphaseAabb`.
    pub fn get_broadphase_aabb(&self) -> (Vec3, Vec3) {
        let bounds = if !self.sets[0].empty() {
            if !self.sets[1].empty() {
                let mut b = DbvtVolume::default();
                merge_into(
                    &self.sets[0].node(self.sets[0].root).volume,
                    &self.sets[1].node(self.sets[1].root).volume,
                    &mut b,
                );
                b
            } else {
                self.sets[0].node(self.sets[0].root).volume
            }
        } else if !self.sets[1].empty() {
            self.sets[1].node(self.sets[1].root).volume
        } else {
            DbvtVolume::from_cr(&Vec3::ZERO, 0.0)
        };
        (bounds.mi, bounds.mx)
    }

    /// `resetPool(dispatcher)`.
    pub fn reset_pool(&mut self) {
        let total = self.sets[0].leaves + self.sets[1].leaves;
        if total == 0 {
            self.sets[0].clear();
            self.sets[1].clear();
            self.deferedcollide = false;
            self.needcleanup = true;
            self.stage_current = 0;
            self.fixedleft = 0;
            self.fupdates = 1;
            self.dupdates = 0;
            self.cupdates = 10;
            self.newpairs = 1;
            self.updates_call = 0;
            self.updates_done = 0;
            self.updates_ratio = 0.0;
            self.gid = 0;
            self.pid = 0;
            self.cid = 0;
            self.stage_roots = [None; STAGECOUNT + 1];
        }
    }
}
