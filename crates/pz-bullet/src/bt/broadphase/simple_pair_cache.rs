// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2013 Erwin Coumans  http://bulletphysics.org
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
// Altered for pz-rs: Rust translation of btHashedSimplePairCache (Bullet 2.82).

//! `btHashedSimplePairCache`: pairs of child indices for
//! `btCompoundCompoundCollisionAlgorithm`, each with a user payload
//! (`m_userPointer`, the child algorithm handle).

use super::pair_cache::{alloc_size, get_hash};

/// `BT_SIMPLE_NULL_PAIR`.
pub const BT_SIMPLE_NULL_PAIR: i32 = -1;

/// `btSimplePair`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SimplePair {
    pub index_a: i32,
    pub index_b: i32,
    /// `m_userPointer` / `m_userValue`.
    pub user_pointer: Option<usize>,
}

/// `btHashedSimplePairCache`.
#[derive(Clone, Debug)]
pub struct HashedSimplePairCache {
    pub overlapping_pair_array: Vec<SimplePair>,
    /// `m_overlappingPairArray.capacity()`.
    pub capacity: usize,
    pub hash_table: Vec<i32>,
    pub next: Vec<i32>,
}

impl Default for HashedSimplePairCache {
    fn default() -> Self {
        HashedSimplePairCache::new()
    }
}

impl HashedSimplePairCache {
    /// `btHashedSimplePairCache()`.
    pub fn new() -> HashedSimplePairCache {
        let mut c = HashedSimplePairCache {
            overlapping_pair_array: Vec::new(),
            capacity: 2,
            hash_table: Vec::new(),
            next: Vec::new(),
        };
        c.grow_tables();
        c
    }

    /// `removeAllPairs()`: `clear()` frees the arrays, then `reserve(2)`.
    pub fn remove_all_pairs(&mut self) {
        self.overlapping_pair_array.clear();
        self.hash_table.clear();
        self.next.clear();
        self.capacity = 2;
        self.grow_tables();
    }

    #[inline(always)]
    pub fn get_num_overlapping_pairs(&self) -> usize {
        self.overlapping_pair_array.len()
    }

    #[inline(always)]
    fn mask(&self) -> u32 {
        (self.capacity as u32).wrapping_sub(1)
    }

    fn grow_tables(&mut self) {
        let new_capacity = self.capacity;
        if self.hash_table.len() < new_capacity {
            let cur = self.hash_table.len();
            self.hash_table.resize(new_capacity, 0);
            self.next.resize(new_capacity, 0);
            for h in self.hash_table.iter_mut() {
                *h = BT_SIMPLE_NULL_PAIR;
            }
            for n in self.next.iter_mut() {
                *n = BT_SIMPLE_NULL_PAIR;
            }
            let mask = self.mask();
            for i in 0..cur {
                let p = &self.overlapping_pair_array[i];
                let hv = (get_hash(p.index_a as u32, p.index_b as u32) & mask) as usize;
                self.next[i] = self.hash_table[hv];
                self.hash_table[hv] = i as i32;
            }
        }
    }

    fn internal_find_pair(&self, a: i32, b: i32, hash: usize) -> i32 {
        let mut index = self.hash_table[hash];
        while index != BT_SIMPLE_NULL_PAIR {
            let p = &self.overlapping_pair_array[index as usize];
            if p.index_a == a && p.index_b == b {
                break;
            }
            index = self.next[index as usize];
        }
        index
    }

    /// `findPair(indexA, indexB)`: pair index.
    pub fn find_pair(&self, a: i32, b: i32) -> Option<usize> {
        let hash = (get_hash(a as u32, b as u32) & self.mask()) as usize;
        if hash >= self.hash_table.len() {
            return None;
        }
        let i = self.internal_find_pair(a, b, hash);
        if i == BT_SIMPLE_NULL_PAIR {
            None
        } else {
            Some(i as usize)
        }
    }

    /// `addOverlappingPair(indexA, indexB)` (`internalAddPair`): pair index.
    pub fn add_overlapping_pair(&mut self, a: i32, b: i32) -> usize {
        let mut hash = (get_hash(a as u32, b as u32) & self.mask()) as usize;
        let found = self.internal_find_pair(a, b, hash);
        if found != BT_SIMPLE_NULL_PAIR {
            return found as usize;
        }
        let count = self.overlapping_pair_array.len();
        let old_capacity = self.capacity;
        if count == self.capacity {
            self.capacity = alloc_size(count);
        }
        self.overlapping_pair_array.push(SimplePair::default());
        if old_capacity < self.capacity {
            self.grow_tables();
            hash = (get_hash(a as u32, b as u32) & self.mask()) as usize;
        }
        self.overlapping_pair_array[count] = SimplePair {
            index_a: a,
            index_b: b,
            user_pointer: None,
        };
        self.next[count] = self.hash_table[hash];
        self.hash_table[hash] = count as i32;
        count
    }

    /// `removeOverlappingPair(indexA, indexB)`: returns the removed pair's
    /// user pointer (`Some(None)` if it had none, `None` if not found).
    pub fn remove_overlapping_pair(&mut self, a: i32, b: i32) -> Option<Option<usize>> {
        let mask = self.mask();
        let hash = (get_hash(a as u32, b as u32) & mask) as usize;
        let pair_index = self.internal_find_pair(a, b, hash);
        if pair_index == BT_SIMPLE_NULL_PAIR {
            return None;
        }
        let pair_index = pair_index as usize;
        let user_data = self.overlapping_pair_array[pair_index].user_pointer;
        let mut index = self.hash_table[hash];
        let mut previous = BT_SIMPLE_NULL_PAIR;
        while index != pair_index as i32 {
            previous = index;
            index = self.next[index as usize];
        }
        if previous != BT_SIMPLE_NULL_PAIR {
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
        let last_hash = (get_hash(last.index_a as u32, last.index_b as u32) & mask) as usize;
        index = self.hash_table[last_hash];
        previous = BT_SIMPLE_NULL_PAIR;
        while index != last_pair_index as i32 {
            previous = index;
            index = self.next[index as usize];
        }
        if previous != BT_SIMPLE_NULL_PAIR {
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
}
