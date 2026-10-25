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
// Altered for pz-rs: Rust translation of btUnionFind (Bullet 2.82).

//! `btUnionFind` (`USE_PATH_COMPRESSION`, `STATIC_SIMULATION_ISLAND_OPTIMIZATION`).

use super::linear_math::quick_sort;

/// `btElement`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Element {
    pub id: i32,
    pub sz: i32,
}

/// `btUnionFind`.
#[derive(Clone, Debug, Default)]
pub struct UnionFind {
    pub elements: Vec<Element>,
}

impl UnionFind {
    pub fn new() -> UnionFind {
        UnionFind::default()
    }
    /// `allocate(N)`: `resize(N)` (new elements are zero).
    pub fn allocate(&mut self, n: usize) {
        self.elements.resize(n, Element::default());
    }
    /// `Free()`.
    pub fn free(&mut self) {
        self.elements.clear();
    }
    /// `reset(N)`.
    pub fn reset(&mut self, n: usize) {
        self.allocate(n);
        for (i, e) in self.elements.iter_mut().enumerate() {
            e.id = i as i32;
            e.sz = 1;
        }
    }
    #[inline(always)]
    pub fn get_num_elements(&self) -> usize {
        self.elements.len()
    }
    #[inline(always)]
    pub fn is_root(&self, x: i32) -> bool {
        x == self.elements[x as usize].id
    }
    #[inline(always)]
    pub fn get_element(&self, i: usize) -> &Element {
        &self.elements[i]
    }
    #[inline(always)]
    pub fn get_element_mut(&mut self, i: usize) -> &mut Element {
        &mut self.elements[i]
    }
    /// `find(x)` with path compression.
    #[inline]
    pub fn find(&mut self, mut x: i32) -> i32 {
        while x != self.elements[x as usize].id {
            let parent = self.elements[x as usize].id;
            let grand = self.elements[parent as usize].id;
            self.elements[x as usize].id = grand;
            x = grand;
        }
        x
    }
    /// `find(p, q)`.
    pub fn find2(&mut self, p: i32, q: i32) -> bool {
        self.find(p) == self.find(q)
    }
    /// `unite(p, q)`.
    pub fn unite(&mut self, p: i32, q: i32) {
        let i = self.find(p);
        let j = self.find(q);
        if i == j {
            return;
        }
        self.elements[i as usize].id = j;
        let szi = self.elements[i as usize].sz;
        self.elements[j as usize].sz += szi;
    }
    /// `sortIslands()`.
    pub fn sort_islands(&mut self) {
        let n = self.elements.len();
        for i in 0..n {
            let r = self.find(i as i32);
            self.elements[i].id = r;
        }
        quick_sort(&mut self.elements, |a, b| a.id < b.id);
    }
}
