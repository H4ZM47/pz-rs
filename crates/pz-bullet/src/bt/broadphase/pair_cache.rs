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
// Altered for pz-rs: Rust translation of btBroadphaseProxy, btBroadphasePair
// and btHashedOverlappingPairCache (Bullet 2.82).

//! `btBroadphaseProxy`, `btBroadphasePair` and `btHashedOverlappingPairCache`.
//!
//! Pairs refer to proxies by [`ProxyId`] and cache the proxies' unique ids
//! (which never change), so the cache needs no access to the proxies after
//! a pair was added. A pair's collision algorithm is an opaque index
//! (`m_algorithm`) owned by the dispatcher; freeing it goes through
//! [`PairDispatcher`].
//!
//! `btAlignedObjectArray` capacities matter here (the hash mask is
//! `capacity - 1` and only grows), so the pair array tracks its capacity
//! the way `btAlignedObjectArray` does: it starts at 2 and doubles when
//! full, and it never shrinks.

use crate::bt::collision_object::ProxyId;
use crate::bt::linear_math::*;

/// `BT_NULL_PAIR`.
pub const BT_NULL_PAIR: i32 = -1;

/// `btBroadphaseProxy::CollisionFilterGroups`.
pub const DEFAULT_FILTER: i16 = 1;
pub const STATIC_FILTER: i16 = 2;
pub const KINEMATIC_FILTER: i16 = 4;
pub const DEBRIS_FILTER: i16 = 8;
pub const SENSOR_TRIGGER: i16 = 16;
pub const CHARACTER_FILTER: i16 = 32;
pub const ALL_FILTER: i16 = -1;

/// The `btBroadphaseProxy` members.
#[derive(Clone, Copy, Debug, Default)]
pub struct BroadphaseProxy {
    /// `m_clientObject` (the collision object id).
    pub client_object: usize,
    pub collision_filter_group: i16,
    pub collision_filter_mask: i16,
    /// `m_uniqueId`.
    pub unique_id: i32,
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
}

/// The dispatcher side of pair removal: `cleanOverlappingPair` destroys a
/// pair's algorithm with `freeCollisionAlgorithm` (which releases its
/// manifolds).
pub trait PairDispatcher {
    /// Destroys the algorithm (`~btCollisionAlgorithm()` +
    /// `freeCollisionAlgorithm`).
    fn free_collision_algorithm(&mut self, algorithm: usize);
}

/// Reborrows an optional dispatcher (`btDispatcher*`, possibly null).
#[inline(always)]
pub fn rb<'s>(d: &'s mut Option<&mut dyn PairDispatcher>) -> Option<&'s mut dyn PairDispatcher> {
    match d {
        Some(x) => Some(&mut **x),
        None => None,
    }
}

/// `btBroadphasePair`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BroadphasePair {
    /// `m_pProxy0` (lower unique id).
    pub proxy0: ProxyId,
    pub proxy1: ProxyId,
    /// Unique ids of `proxy0` and `proxy1`.
    pub uid0: i32,
    pub uid1: i32,
    /// `m_algorithm`: dispatcher-owned algorithm handle.
    pub algorithm: Option<usize>,
    /// `m_internalInfo1` / `m_internalTmpValue`.
    pub internal_info1: i64,
}

impl BroadphasePair {
    /// `btBroadphasePair(proxy0, proxy1)`: sorted by unique id.
    pub fn new(
        id0: ProxyId,
        p0: &BroadphaseProxy,
        id1: ProxyId,
        p1: &BroadphaseProxy,
    ) -> BroadphasePair {
        if p0.unique_id < p1.unique_id {
            BroadphasePair {
                proxy0: id0,
                proxy1: id1,
                uid0: p0.unique_id,
                uid1: p1.unique_id,
                algorithm: None,
                internal_info1: 0,
            }
        } else {
            BroadphasePair {
                proxy0: id1,
                proxy1: id0,
                uid0: p1.unique_id,
                uid1: p0.unique_id,
                algorithm: None,
                internal_info1: 0,
            }
        }
    }
}

/// `btBroadphasePairSortPredicate` on (`uid0`, `uid1`); equal pairs (which
/// the hashed cache never holds) would fall back to algorithm pointers.
pub fn broadphase_pair_sort_predicate(a: &BroadphasePair, b: &BroadphasePair) -> bool {
    a.uid0 > b.uid0
        || (a.proxy0 == b.proxy0 && a.uid1 > b.uid1)
        || (a.proxy0 == b.proxy0
            && a.proxy1 == b.proxy1
            && a.algorithm.map(|x| x as i64).unwrap_or(-1)
                > b.algorithm.map(|x| x as i64).unwrap_or(-1))
}

/// `btHashedOverlappingPairCache::getHash` (also used by
/// `btHashedSimplePairCache`).
#[inline(always)]
pub fn get_hash(id1: u32, id2: u32) -> u32 {
    let mut key = (id1 | (id2 << 16)) as i32;
    key = key.wrapping_add(!(key.wrapping_shl(15)));
    key ^= key >> 10;
    key = key.wrapping_add(key.wrapping_shl(3));
    key ^= key >> 6;
    key = key.wrapping_add(!(key.wrapping_shl(11)));
    key ^= key >> 16;
    key as u32
}

/// `btAlignedObjectArray::allocSize`.
#[inline(always)]
pub fn alloc_size(size: usize) -> usize {
    if size != 0 {
        size * 2
    } else {
        1
    }
}

/// `btHashedOverlappingPairCache` (no overlap filter callback and no ghost
/// pair callback: the game sets neither).
#[derive(Clone, Debug)]
pub struct HashedOverlappingPairCache {
    pub overlapping_pair_array: Vec<BroadphasePair>,
    /// `m_overlappingPairArray.capacity()`.
    pub capacity: usize,
    pub blocked_for_changes: bool,
    pub hash_table: Vec<i32>,
    pub next: Vec<i32>,
}

impl Default for HashedOverlappingPairCache {
    fn default() -> Self {
        HashedOverlappingPairCache::new()
    }
}

impl HashedOverlappingPairCache {
    /// `btHashedOverlappingPairCache()`: `reserve(2)` then `growTables()`.
    pub fn new() -> HashedOverlappingPairCache {
        let mut c = HashedOverlappingPairCache {
            overlapping_pair_array: Vec::new(),
            capacity: 0,
            blocked_for_changes: false,
            hash_table: Vec::new(),
            next: Vec::new(),
        };
        c.capacity = 2;
        c.grow_tables();
        c
    }

    #[inline(always)]
    pub fn get_num_overlapping_pairs(&self) -> usize {
        self.overlapping_pair_array.len()
    }

    #[inline(always)]
    fn mask(&self) -> u32 {
        (self.capacity as u32).wrapping_sub(1)
    }

    #[inline(always)]
    fn equals_pair(pair: &BroadphasePair, id1: i32, id2: i32) -> bool {
        pair.uid0 == id1 && pair.uid1 == id2
    }

    /// `needsBroadphaseCollision`.
    #[inline(always)]
    pub fn needs_broadphase_collision(p0: &BroadphaseProxy, p1: &BroadphaseProxy) -> bool {
        let collides = (p0.collision_filter_group & p1.collision_filter_mask) != 0;
        collides && (p1.collision_filter_group & p0.collision_filter_mask) != 0
    }

    /// `growTables()`.
    fn grow_tables(&mut self) {
        let new_capacity = self.capacity;
        if self.hash_table.len() < new_capacity {
            let cur = self.hash_table.len();
            self.hash_table.resize(new_capacity, 0);
            self.next.resize(new_capacity, 0);
            for h in self.hash_table.iter_mut() {
                *h = BT_NULL_PAIR;
            }
            for n in self.next.iter_mut() {
                *n = BT_NULL_PAIR;
            }
            let mask = self.mask();
            for i in 0..cur {
                let pair = &self.overlapping_pair_array[i];
                let hv = (get_hash(pair.uid0 as u32, pair.uid1 as u32) & mask) as usize;
                self.next[i] = self.hash_table[hv];
                self.hash_table[hv] = i as i32;
            }
        }
    }

    fn internal_find_pair(&self, id1: i32, id2: i32, hash: usize) -> i32 {
        let mut index = self.hash_table[hash];
        while index != BT_NULL_PAIR
            && !Self::equals_pair(&self.overlapping_pair_array[index as usize], id1, id2)
        {
            index = self.next[index as usize];
        }
        index
    }

    /// `addOverlappingPair(proxy0, proxy1)`: returns the pair index, or
    /// `None` when the filter rejects it.
    pub fn add_overlapping_pair(
        &mut self,
        id0: ProxyId,
        p0: &BroadphaseProxy,
        id1: ProxyId,
        p1: &BroadphaseProxy,
    ) -> Option<usize> {
        if !Self::needs_broadphase_collision(p0, p1) {
            return None;
        }
        Some(self.internal_add_pair(id0, p0, id1, p1))
    }

    /// `internalAddPair`.
    fn internal_add_pair<'a>(
        &mut self,
        mut id0: ProxyId,
        mut p0: &'a BroadphaseProxy,
        mut id1: ProxyId,
        mut p1: &'a BroadphaseProxy,
    ) -> usize {
        if p0.unique_id > p1.unique_id {
            std::mem::swap(&mut p0, &mut p1);
            std::mem::swap(&mut id0, &mut id1);
        }
        let uid1 = p0.unique_id;
        let uid2 = p1.unique_id;
        let mut hash = (get_hash(uid1 as u32, uid2 as u32) & self.mask()) as usize;
        let found = self.internal_find_pair(uid1, uid2, hash);
        if found != BT_NULL_PAIR {
            return found as usize;
        }
        let count = self.overlapping_pair_array.len();
        let old_capacity = self.capacity;
        // expandNonInitializing
        if count == self.capacity {
            self.capacity = alloc_size(count);
        }
        self.overlapping_pair_array.push(BroadphasePair::default());
        let new_capacity = self.capacity;
        if old_capacity < new_capacity {
            self.grow_tables();
            hash = (get_hash(uid1 as u32, uid2 as u32) & self.mask()) as usize;
        }
        self.overlapping_pair_array[count] = BroadphasePair::new(id0, p0, id1, p1);
        self.next[count] = self.hash_table[hash];
        self.hash_table[hash] = count as i32;
        count
    }

    /// `findPair(proxy0, proxy1)`: pair index.
    pub fn find_pair(&self, p0: &BroadphaseProxy, p1: &BroadphaseProxy) -> Option<usize> {
        let (a, b) = if p0.unique_id > p1.unique_id {
            (p1, p0)
        } else {
            (p0, p1)
        };
        let id1 = a.unique_id;
        let id2 = b.unique_id;
        let hash = (get_hash(id1 as u32, id2 as u32) & self.mask()) as usize;
        if hash >= self.hash_table.len() {
            return None;
        }
        let index = self.internal_find_pair(id1, id2, hash);
        if index == BT_NULL_PAIR {
            None
        } else {
            Some(index as usize)
        }
    }

    /// `cleanOverlappingPair(pair, dispatcher)`.
    pub fn clean_overlapping_pair(
        pair: &mut BroadphasePair,
        dispatcher: Option<&mut dyn PairDispatcher>,
    ) {
        if let (Some(alg), Some(d)) = (pair.algorithm, dispatcher) {
            d.free_collision_algorithm(alg);
            pair.algorithm = None;
        }
    }

    /// `removeOverlappingPair(proxy0, proxy1, dispatcher)`: returns
    /// `m_internalInfo1` of the removed pair.
    pub fn remove_overlapping_pair(
        &mut self,
        p0: &BroadphaseProxy,
        p1: &BroadphaseProxy,
        dispatcher: Option<&mut dyn PairDispatcher>,
    ) -> Option<i64> {
        let (a, b) = if p0.unique_id > p1.unique_id {
            (p1, p0)
        } else {
            (p0, p1)
        };
        self.remove_overlapping_pair_uids(a.unique_id, b.unique_id, dispatcher)
    }

    /// `removeOverlappingPair` on already sorted unique ids.
    pub fn remove_overlapping_pair_uids(
        &mut self,
        id1: i32,
        id2: i32,
        dispatcher: Option<&mut dyn PairDispatcher>,
    ) -> Option<i64> {
        let mask = self.mask();
        let hash = (get_hash(id1 as u32, id2 as u32) & mask) as usize;
        let pair_index = self.internal_find_pair(id1, id2, hash);
        if pair_index == BT_NULL_PAIR {
            return None;
        }
        let pair_index = pair_index as usize;
        Self::clean_overlapping_pair(&mut self.overlapping_pair_array[pair_index], dispatcher);
        let user_data = self.overlapping_pair_array[pair_index].internal_info1;

        let mut index = self.hash_table[hash];
        let mut previous = BT_NULL_PAIR;
        while index != pair_index as i32 {
            previous = index;
            index = self.next[index as usize];
        }
        if previous != BT_NULL_PAIR {
            self.next[previous as usize] = self.next[pair_index];
        } else {
            self.hash_table[hash] = self.next[pair_index];
        }
        let last_pair_index = self.overlapping_pair_array.len() - 1;
        if last_pair_index == pair_index {
            self.overlapping_pair_array.pop();
            return Some(user_data);
        }
        let last = self.overlapping_pair_array[last_pair_index];
        let last_hash = (get_hash(last.uid0 as u32, last.uid1 as u32) & mask) as usize;
        index = self.hash_table[last_hash];
        previous = BT_NULL_PAIR;
        while index != last_pair_index as i32 {
            previous = index;
            index = self.next[index as usize];
        }
        if previous != BT_NULL_PAIR {
            self.next[previous as usize] = self.next[last_pair_index];
        } else {
            self.hash_table[last_hash] = self.next[last_pair_index];
        }
        self.overlapping_pair_array[pair_index] = last;
        self.next[pair_index] = self.hash_table[last_hash];
        self.hash_table[last_hash] = pair_index as i32;
        self.overlapping_pair_array.pop();
        Some(user_data)
    }

    /// `processAllOverlappingPairs(callback, dispatcher)`: a pair is removed
    /// when the callback returns true.
    pub fn process_all_overlapping_pairs(
        &mut self,
        callback: &mut dyn FnMut(&mut BroadphasePair, Option<&mut dyn PairDispatcher>) -> bool,
        mut dispatcher: Option<&mut dyn PairDispatcher>,
    ) {
        let mut i = 0;
        while i < self.overlapping_pair_array.len() {
            let remove = {
                let pair = &mut self.overlapping_pair_array[i];
                callback(pair, rb(&mut dispatcher))
            };
            if remove {
                let p = self.overlapping_pair_array[i];
                self.remove_overlapping_pair_uids(p.uid0, p.uid1, rb(&mut dispatcher));
            } else {
                i += 1;
            }
        }
    }

    /// `removeOverlappingPairsContainingProxy(proxy, dispatcher)`.
    pub fn remove_overlapping_pairs_containing_proxy(
        &mut self,
        proxy: ProxyId,
        dispatcher: Option<&mut dyn PairDispatcher>,
    ) {
        self.process_all_overlapping_pairs(
            &mut |pair, _| pair.proxy0 == proxy || pair.proxy1 == proxy,
            dispatcher,
        );
    }

    /// `cleanProxyFromPairs(proxy, dispatcher)`.
    pub fn clean_proxy_from_pairs(
        &mut self,
        proxy: ProxyId,
        dispatcher: Option<&mut dyn PairDispatcher>,
    ) {
        self.process_all_overlapping_pairs(
            &mut |pair, d| {
                if pair.proxy0 == proxy || pair.proxy1 == proxy {
                    Self::clean_overlapping_pair(pair, d);
                }
                false
            },
            dispatcher,
        );
    }

    /// `sortOverlappingPairs(dispatcher)`. `proxy_of(id)` returns the proxy
    /// for the filter test of the re-added pairs.
    pub fn sort_overlapping_pairs(
        &mut self,
        mut dispatcher: Option<&mut dyn PairDispatcher>,
        proxy_of: &dyn Fn(ProxyId) -> BroadphaseProxy,
    ) {
        let mut tmp_pairs: Vec<BroadphasePair> = self.overlapping_pair_array.clone();
        for p in &tmp_pairs {
            self.remove_overlapping_pair_uids(p.uid0, p.uid1, rb(&mut dispatcher));
        }
        for n in self.next.iter_mut() {
            *n = BT_NULL_PAIR;
        }
        quick_sort(&mut tmp_pairs, broadphase_pair_sort_predicate);
        for p in &tmp_pairs {
            let a = proxy_of(p.proxy0);
            let b = proxy_of(p.proxy1);
            self.add_overlapping_pair(p.proxy0, &a, p.proxy1, &b);
        }
    }
}
