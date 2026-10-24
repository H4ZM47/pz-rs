// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2007 Erwin Coumans  http://continuousphysics.com/Bullet/
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
// btDbvt implementation by Nathanael Presson.
// Altered for pz-rs: Rust translation of btDbvt (Bullet 2.82, double
// precision, generic (non-SSE) code paths).

//! `btDbvt`: the dynamic AABB tree used by `btDbvtBroadphase` and
//! `btCompoundShape`.
//!
//! Nodes live in an arena ([`NodeId`]). Leaves carry a `usize` payload
//! (`btDbvtNode::data`): a proxy id in the broadphase, a child index in a
//! compound shape.
//!
//! # Node addresses
//!
//! `optimizeIncremental` calls `sort()`, which swaps a node with its parent
//! when `parent > node` *as pointers*. The swap exchanges the two nodes'
//! places and volumes, so the tree's shape, volumes and leaf order are the
//! same either way: heap addresses only decide which node object sits where.
//! The port keeps the comparison on an allocation counter (`addr`, or
//! [`Dbvt::addr_source`]); results do not depend on it. Verified against the
//! original under both a bump allocator and the system `malloc`, and with
//! reversed addresses here.

use crate::bt::linear_math::*;

/// Index of a node in a [`Dbvt`] arena.
pub type NodeId = u32;
/// "null" node.
pub const NIL: NodeId = u32::MAX;

/// `btDbvtAabbMm` / `btDbvtVolume`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DbvtAabbMm {
    pub mi: Vec3,
    pub mx: Vec3,
}

pub type DbvtVolume = DbvtAabbMm;

impl DbvtAabbMm {
    /// `Center()`: `(mi+mx)/2`.
    #[inline(always)]
    pub fn center(&self) -> Vec3 {
        (self.mi + self.mx) / 2.0
    }
    /// `Lengths()`.
    #[inline(always)]
    pub fn lengths(&self) -> Vec3 {
        self.mx - self.mi
    }
    /// `Extents()`.
    #[inline(always)]
    pub fn extents(&self) -> Vec3 {
        (self.mx - self.mi) / 2.0
    }
    #[inline(always)]
    pub fn mins(&self) -> &Vec3 {
        &self.mi
    }
    #[inline(always)]
    pub fn maxs(&self) -> &Vec3 {
        &self.mx
    }
    /// `FromCE`.
    #[inline(always)]
    pub fn from_ce(c: &Vec3, e: &Vec3) -> DbvtAabbMm {
        DbvtAabbMm {
            mi: *c - *e,
            mx: *c + *e,
        }
    }
    /// `FromCR`.
    #[inline(always)]
    pub fn from_cr(c: &Vec3, r: f64) -> DbvtAabbMm {
        DbvtAabbMm::from_ce(c, &Vec3::new(r, r, r))
    }
    /// `FromMM`.
    #[inline(always)]
    pub fn from_mm(mi: &Vec3, mx: &Vec3) -> DbvtAabbMm {
        DbvtAabbMm { mi: *mi, mx: *mx }
    }
    /// `FromPoints`.
    pub fn from_points(pts: &[Vec3]) -> DbvtAabbMm {
        let mut b = DbvtAabbMm {
            mi: pts[0],
            mx: pts[0],
        };
        for p in &pts[1..] {
            b.mi.set_min(p);
            b.mx.set_max(p);
        }
        b
    }
    /// `Expand(e)`.
    #[inline(always)]
    pub fn expand(&mut self, e: &Vec3) {
        self.mi -= *e;
        self.mx += *e;
    }
    /// `SignedExpand(e)`.
    #[inline(always)]
    pub fn signed_expand(&mut self, e: &Vec3) {
        if e.x > 0.0 {
            self.mx.x += e.x;
        } else {
            self.mi.x += e.x;
        }
        if e.y > 0.0 {
            self.mx.y += e.y;
        } else {
            self.mi.y += e.y;
        }
        if e.z > 0.0 {
            self.mx.z += e.z;
        } else {
            self.mi.z += e.z;
        }
    }
    /// `Contain(a)`.
    #[inline(always)]
    pub fn contain(&self, a: &DbvtAabbMm) -> bool {
        self.mi.x <= a.mi.x
            && self.mi.y <= a.mi.y
            && self.mi.z <= a.mi.z
            && self.mx.x >= a.mx.x
            && self.mx.y >= a.mx.y
            && self.mx.z >= a.mx.z
    }
}

/// `Intersect(a, b)` (generic implementation).
#[inline(always)]
pub fn intersect(a: &DbvtAabbMm, b: &DbvtAabbMm) -> bool {
    a.mi.x <= b.mx.x
        && a.mx.x >= b.mi.x
        && a.mi.y <= b.mx.y
        && a.mx.y >= b.mi.y
        && a.mi.z <= b.mx.z
        && a.mx.z >= b.mi.z
}

/// `Intersect(a, point)`.
#[inline(always)]
pub fn intersect_point(a: &DbvtAabbMm, b: &Vec3) -> bool {
    b.x >= a.mi.x
        && b.y >= a.mi.y
        && b.z >= a.mi.z
        && b.x <= a.mx.x
        && b.y <= a.mx.y
        && b.z <= a.mx.z
}

/// `Proximity(a, b)`.
#[inline(always)]
pub fn proximity(a: &DbvtAabbMm, b: &DbvtAabbMm) -> f64 {
    let d = (a.mi + a.mx) - (b.mi + b.mx);
    d.x.abs() + d.y.abs() + d.z.abs()
}

/// `Select(o, a, b)`.
#[inline(always)]
pub fn select(o: &DbvtAabbMm, a: &DbvtAabbMm, b: &DbvtAabbMm) -> usize {
    if proximity(o, a) < proximity(o, b) {
        0
    } else {
        1
    }
}

/// `Merge(a, b, r)`: writes x, y and z of `r` only.
#[inline(always)]
pub fn merge_into(a: &DbvtAabbMm, b: &DbvtAabbMm, r: &mut DbvtAabbMm) {
    for i in 0..3 {
        r.mi[i] = if a.mi[i] < b.mi[i] { a.mi[i] } else { b.mi[i] };
        r.mx[i] = if a.mx[i] > b.mx[i] { a.mx[i] } else { b.mx[i] };
    }
}

/// `NotEqual(a, b)`.
#[inline(always)]
pub fn not_equal(a: &DbvtAabbMm, b: &DbvtAabbMm) -> bool {
    a.mi.x != b.mi.x
        || a.mi.y != b.mi.y
        || a.mi.z != b.mi.z
        || a.mx.x != b.mx.x
        || a.mx.y != b.mx.y
        || a.mx.z != b.mx.z
}

/// `merge(a, b)` (btDbvt.cpp; the result is a default volume with x, y, z set).
#[inline(always)]
fn merge(a: &DbvtVolume, b: &DbvtVolume) -> DbvtVolume {
    let mut r = DbvtVolume::default();
    merge_into(a, b, &mut r);
    r
}

/// `size(a)`: volume plus edge lengths, `x*y*z + x` fused.
#[inline(always)]
fn size(a: &DbvtVolume) -> f64 {
    let e = a.lengths();
    fma(e.x * e.y, e.z, e.x) + e.y + e.z
}

/// Alias used by the compound shape.
pub type DbvtNodeId = NodeId;

/// `btDbvtNode`.
#[derive(Clone, Copy, Debug)]
pub struct DbvtNode {
    pub volume: DbvtVolume,
    pub parent: NodeId,
    /// `childs[0]`, `childs[1]`. A leaf has `childs[1] == NIL`.
    pub childs: [NodeId; 2],
    /// `data` of a leaf.
    pub data: usize,
    /// Allocation address (see the module docs).
    pub addr: u64,
}

impl DbvtNode {
    #[inline(always)]
    pub fn is_leaf(&self) -> bool {
        self.childs[1] == NIL
    }
    #[inline(always)]
    pub fn is_internal(&self) -> bool {
        !self.is_leaf()
    }
}

/// `btDbvt`.
pub struct Dbvt {
    pub nodes: Vec<DbvtNode>,
    /// Arena slots not in use (storage recycling only).
    free_slots: Vec<NodeId>,
    pub root: NodeId,
    /// `m_free`: one deleted node kept for the next `createnode`.
    pub free: NodeId,
    pub lkhd: i32,
    pub leaves: i32,
    pub opath: u32,
    next_addr: u64,
    /// Supplies the address of each freshly allocated node; `None` uses an
    /// increasing counter.
    pub addr_source: Option<Box<dyn FnMut() -> u64>>,
    stk_stack: Vec<(NodeId, NodeId)>,
}

impl Clone for Dbvt {
    /// Copies the tree; a custom `addr_source` is not carried over.
    fn clone(&self) -> Self {
        Dbvt {
            nodes: self.nodes.clone(),
            free_slots: self.free_slots.clone(),
            root: self.root,
            free: self.free,
            lkhd: self.lkhd,
            leaves: self.leaves,
            opath: self.opath,
            next_addr: self.next_addr,
            addr_source: None,
            stk_stack: Vec::new(),
        }
    }
}

impl Default for Dbvt {
    fn default() -> Self {
        Dbvt::new()
    }
}

impl std::fmt::Debug for Dbvt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dbvt")
            .field("root", &self.root)
            .field("leaves", &self.leaves)
            .finish()
    }
}

impl Dbvt {
    /// `btDbvt()`.
    pub fn new() -> Dbvt {
        Dbvt {
            nodes: Vec::new(),
            free_slots: Vec::new(),
            root: NIL,
            free: NIL,
            lkhd: -1,
            leaves: 0,
            opath: 0,
            next_addr: 0,
            addr_source: None,
            stk_stack: Vec::new(),
        }
    }

    /// Sets the payload of a leaf (`leaf->dataAsInt = ...`).
    #[inline(always)]
    pub fn set_leaf_data(&mut self, leaf: NodeId, data: usize) {
        self.nodes[leaf as usize].data = data;
    }
    #[inline(always)]
    pub fn node(&self, n: NodeId) -> &DbvtNode {
        &self.nodes[n as usize]
    }
    #[inline(always)]
    fn n(&mut self, n: NodeId) -> &mut DbvtNode {
        &mut self.nodes[n as usize]
    }
    #[inline(always)]
    pub fn empty(&self) -> bool {
        self.root == NIL
    }

    fn fresh_addr(&mut self) -> u64 {
        if let Some(f) = self.addr_source.as_mut() {
            f()
        } else {
            self.next_addr += 1;
            self.next_addr
        }
    }

    /// Releases an arena slot (`btAlignedFree`).
    fn release_slot(&mut self, n: NodeId) {
        if n != NIL {
            self.free_slots.push(n);
        }
    }

    /// `deletenode`: frees the previous `m_free`, keeps `node`.
    fn deletenode(&mut self, node: NodeId) {
        let old = self.free;
        self.release_slot(old);
        self.free = node;
    }

    fn recursedeletenode(&mut self, node: NodeId) {
        if !self.node(node).is_leaf() {
            let [c0, c1] = self.node(node).childs;
            self.recursedeletenode(c0);
            self.recursedeletenode(c1);
        }
        if node == self.root {
            self.root = NIL;
        }
        self.deletenode(node);
    }

    /// `createnode(pdbvt, parent, data)`.
    fn createnode(&mut self, parent: NodeId, data: usize) -> NodeId {
        let id;
        if self.free != NIL {
            id = self.free;
            self.free = NIL;
        } else {
            let addr = self.fresh_addr();
            let fresh = DbvtNode {
                volume: DbvtVolume::default(),
                parent: NIL,
                childs: [NIL, NIL],
                data: 0,
                addr,
            };
            if let Some(slot) = self.free_slots.pop() {
                self.nodes[slot as usize] = fresh;
                id = slot;
            } else {
                id = self.nodes.len() as NodeId;
                self.nodes.push(fresh);
            }
        }
        let n = self.n(id);
        n.parent = parent;
        n.data = data;
        n.childs = [NIL, NIL];
        id
    }

    fn createnode_vol(&mut self, parent: NodeId, volume: &DbvtVolume, data: usize) -> NodeId {
        let id = self.createnode(parent, data);
        self.n(id).volume = *volume;
        id
    }

    fn createnode_vol2(
        &mut self,
        parent: NodeId,
        v0: &DbvtVolume,
        v1: &DbvtVolume,
        data: usize,
    ) -> NodeId {
        let id = self.createnode(parent, data);
        merge_into(v0, v1, &mut self.n(id).volume);
        id
    }

    /// `indexof(node)`.
    #[inline(always)]
    fn indexof(&self, node: NodeId) -> usize {
        let p = self.node(node).parent;
        (self.node(p).childs[1] == node) as usize
    }

    /// `insertleaf(pdbvt, root, leaf)`.
    fn insertleaf(&mut self, mut root: NodeId, leaf: NodeId) {
        if self.root == NIL {
            self.root = leaf;
            self.n(leaf).parent = NIL;
            return;
        }
        if !self.node(root).is_leaf() {
            loop {
                let lv = self.node(leaf).volume;
                let [c0, c1] = self.node(root).childs;
                let s = select(&lv, &self.node(c0).volume, &self.node(c1).volume);
                root = self.node(root).childs[s];
                if self.node(root).is_leaf() {
                    break;
                }
            }
        }
        let mut prev = self.node(root).parent;
        let lv = self.node(leaf).volume;
        let rv = self.node(root).volume;
        let mut node = self.createnode_vol2(prev, &lv, &rv, 0);
        if prev != NIL {
            let i = self.indexof(root);
            self.n(prev).childs[i] = node;
            self.n(node).childs[0] = root;
            self.n(root).parent = node;
            self.n(node).childs[1] = leaf;
            self.n(leaf).parent = node;
            loop {
                let nv = self.node(node).volume;
                if !self.node(prev).volume.contain(&nv) {
                    let [c0, c1] = self.node(prev).childs;
                    let a = self.node(c0).volume;
                    let b = self.node(c1).volume;
                    merge_into(&a, &b, &mut self.n(prev).volume);
                } else {
                    break;
                }
                node = prev;
                prev = self.node(node).parent;
                if prev == NIL {
                    break;
                }
            }
        } else {
            self.n(node).childs[0] = root;
            self.n(root).parent = node;
            self.n(node).childs[1] = leaf;
            self.n(leaf).parent = node;
            self.root = node;
        }
    }

    /// `removeleaf(pdbvt, leaf)`: returns the node to restart insertion from.
    fn removeleaf(&mut self, leaf: NodeId) -> NodeId {
        if leaf == self.root {
            self.root = NIL;
            return NIL;
        }
        let parent = self.node(leaf).parent;
        let mut prev = self.node(parent).parent;
        let sibling = self.node(parent).childs[1 - self.indexof(leaf)];
        if prev != NIL {
            let i = self.indexof(parent);
            self.n(prev).childs[i] = sibling;
            self.n(sibling).parent = prev;
            self.deletenode(parent);
            while prev != NIL {
                let pb = self.node(prev).volume;
                let [c0, c1] = self.node(prev).childs;
                let a = self.node(c0).volume;
                let b = self.node(c1).volume;
                merge_into(&a, &b, &mut self.n(prev).volume);
                if not_equal(&pb, &self.node(prev).volume) {
                    prev = self.node(prev).parent;
                } else {
                    break;
                }
            }
            if prev != NIL {
                prev
            } else {
                self.root
            }
        } else {
            self.root = sibling;
            self.n(sibling).parent = NIL;
            self.deletenode(parent);
            self.root
        }
    }

    /// `fetchleaves(pdbvt, root, leaves, depth)`.
    fn fetchleaves(&mut self, root: NodeId, leaves: &mut Vec<NodeId>, depth: i32) {
        if self.node(root).is_internal() && depth != 0 {
            let [c0, c1] = self.node(root).childs;
            self.fetchleaves(c0, leaves, depth - 1);
            self.fetchleaves(c1, leaves, depth - 1);
            self.deletenode(root);
        } else {
            leaves.push(root);
        }
    }

    /// `split`.
    fn split(
        &self,
        leaves: &[NodeId],
        left: &mut Vec<NodeId>,
        right: &mut Vec<NodeId>,
        org: &Vec3,
        axis: &Vec3,
    ) {
        left.clear();
        right.clear();
        for &l in leaves {
            if axis.dot(&(self.node(l).volume.center() - *org)) < 0.0 {
                left.push(l);
            } else {
                right.push(l);
            }
        }
    }

    /// `bounds(leaves)`.
    fn bounds(&self, leaves: &[NodeId]) -> DbvtVolume {
        let mut volume = self.node(leaves[0]).volume;
        for &l in &leaves[1..] {
            let v = volume;
            merge_into(&v, &self.node(l).volume, &mut volume);
        }
        volume
    }

    /// `bottomup(pdbvt, leaves)`.
    fn bottomup(&mut self, leaves: &mut Vec<NodeId>) {
        while leaves.len() > 1 {
            let mut minsize = SIMD_INFINITY;
            let mut minidx = [-1i64, -1i64];
            for i in 0..leaves.len() {
                for j in i + 1..leaves.len() {
                    let sz = size(&merge(
                        &self.node(leaves[i]).volume,
                        &self.node(leaves[j]).volume,
                    ));
                    if sz < minsize {
                        minsize = sz;
                        minidx = [i as i64, j as i64];
                    }
                }
            }
            let n = [leaves[minidx[0] as usize], leaves[minidx[1] as usize]];
            let v0 = self.node(n[0]).volume;
            let v1 = self.node(n[1]).volume;
            let p = self.createnode_vol2(NIL, &v0, &v1, 0);
            self.n(p).childs = [n[0], n[1]];
            self.n(n[0]).parent = p;
            self.n(n[1]).parent = p;
            leaves[minidx[0] as usize] = p;
            let last = leaves.len() - 1;
            leaves.swap(minidx[1] as usize, last);
            leaves.pop();
        }
    }

    /// `topdown(pdbvt, leaves, bu_treshold)`.
    fn topdown(&mut self, leaves: &mut Vec<NodeId>, bu_treshold: i32) -> NodeId {
        const AXIS: [Vec3; 3] = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        if leaves.len() > 1 {
            if leaves.len() as i32 > bu_treshold {
                let vol = self.bounds(leaves);
                let org = vol.center();
                let mut sets: [Vec<NodeId>; 2] = [Vec::new(), Vec::new()];
                let mut bestaxis = -1i32;
                let mut bestmidp = leaves.len() as i32;
                let mut splitcount = [[0i32; 2]; 3];
                for &l in leaves.iter() {
                    let x = self.node(l).volume.center() - org;
                    for (j, ax) in AXIS.iter().enumerate() {
                        splitcount[j][if x.dot(ax) > 0.0 { 1 } else { 0 }] += 1;
                    }
                }
                for (i, sc) in splitcount.iter().enumerate() {
                    if sc[0] > 0 && sc[1] > 0 {
                        let midp = ((sc[0] - sc[1]) as f64).abs() as i32;
                        if midp < bestmidp {
                            bestaxis = i as i32;
                            bestmidp = midp;
                        }
                    }
                }
                if bestaxis >= 0 {
                    let (l, r) = sets.split_at_mut(1);
                    self.split(leaves, &mut l[0], &mut r[0], &org, &AXIS[bestaxis as usize]);
                } else {
                    for (i, &l) in leaves.iter().enumerate() {
                        sets[i & 1].push(l);
                    }
                }
                let node = self.createnode_vol(NIL, &vol, 0);
                let c0 = self.topdown(&mut sets[0], bu_treshold);
                self.n(node).childs[0] = c0;
                let c1 = self.topdown(&mut sets[1], bu_treshold);
                self.n(node).childs[1] = c1;
                self.n(c0).parent = node;
                self.n(c1).parent = node;
                return node;
            } else {
                self.bottomup(leaves);
                return leaves[0];
            }
        }
        leaves[0]
    }

    /// `sort(n, r)`: swaps `n` with its parent when the parent's address is higher.
    fn sort(&mut self, n: NodeId) -> NodeId {
        let p = self.node(n).parent;
        if p != NIL && self.node(p).addr > self.node(n).addr {
            let i = self.indexof(n);
            let j = 1 - i;
            let s = self.node(p).childs[j];
            let q = self.node(p).parent;
            if q != NIL {
                let k = self.indexof(p);
                self.n(q).childs[k] = n;
            } else {
                self.root = n;
            }
            self.n(s).parent = n;
            self.n(p).parent = n;
            self.n(n).parent = q;
            let [nc0, nc1] = self.node(n).childs;
            self.n(p).childs = [nc0, nc1];
            self.n(nc0).parent = p;
            self.n(nc1).parent = p;
            self.n(n).childs[i] = p;
            self.n(n).childs[j] = s;
            let pv = self.node(p).volume;
            let nv = self.node(n).volume;
            self.n(p).volume = nv;
            self.n(n).volume = pv;
            return p;
        }
        n
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        if self.root != NIL {
            let r = self.root;
            self.recursedeletenode(r);
        }
        let f = self.free;
        self.release_slot(f);
        self.free = NIL;
        self.lkhd = -1;
        self.stk_stack.clear();
        self.opath = 0;
    }

    /// `optimizeBottomUp()`.
    pub fn optimize_bottom_up(&mut self) {
        if self.root != NIL {
            let mut leaves = Vec::with_capacity(self.leaves.max(0) as usize);
            let r = self.root;
            self.fetchleaves(r, &mut leaves, -1);
            self.bottomup(&mut leaves);
            self.root = leaves[0];
        }
    }

    /// `optimizeTopDown(bu_treshold = 128)`.
    pub fn optimize_top_down(&mut self, bu_treshold: i32) {
        if self.root != NIL {
            let mut leaves = Vec::with_capacity(self.leaves.max(0) as usize);
            let r = self.root;
            self.fetchleaves(r, &mut leaves, -1);
            self.root = self.topdown(&mut leaves, bu_treshold);
        }
    }

    /// `optimizeIncremental(passes)`.
    pub fn optimize_incremental(&mut self, mut passes: i32) {
        if passes < 0 {
            passes = self.leaves;
        }
        if self.root != NIL && passes > 0 {
            loop {
                let mut node = self.root;
                let mut bit: u32 = 0;
                while self.node(node).is_internal() {
                    let s = self.sort(node);
                    node = self.node(s).childs[((self.opath >> bit) & 1) as usize];
                    bit = (bit + 1) & (u32::BITS - 1);
                }
                self.update_lookahead(node, -1);
                self.opath = self.opath.wrapping_add(1);
                passes -= 1;
                if passes == 0 {
                    break;
                }
            }
        }
    }

    /// `insert(volume, data)`.
    pub fn insert(&mut self, volume: &DbvtVolume, data: usize) -> NodeId {
        let leaf = self.createnode_vol(NIL, volume, data);
        let r = self.root;
        self.insertleaf(r, leaf);
        self.leaves += 1;
        leaf
    }

    /// `update(leaf, lookahead = -1)`.
    pub fn update_lookahead(&mut self, leaf: NodeId, lookahead: i32) {
        let mut root = self.removeleaf(leaf);
        if root != NIL {
            if lookahead >= 0 {
                let mut i = 0;
                while i < lookahead && self.node(root).parent != NIL {
                    root = self.node(root).parent;
                    i += 1;
                }
            } else {
                root = self.root;
            }
        }
        self.insertleaf(root, leaf);
    }

    /// `update(leaf, volume)`.
    pub fn update(&mut self, leaf: NodeId, volume: &DbvtVolume) {
        let mut root = self.removeleaf(leaf);
        if root != NIL {
            if self.lkhd >= 0 {
                let mut i = 0;
                while i < self.lkhd && self.node(root).parent != NIL {
                    root = self.node(root).parent;
                    i += 1;
                }
            } else {
                root = self.root;
            }
        }
        self.n(leaf).volume = *volume;
        self.insertleaf(root, leaf);
    }

    /// `update(leaf, volume, velocity, margin)`.
    pub fn update_velocity_margin(
        &mut self,
        leaf: NodeId,
        volume: &mut DbvtVolume,
        velocity: &Vec3,
        margin: f64,
    ) -> bool {
        if self.node(leaf).volume.contain(volume) {
            return false;
        }
        volume.expand(&Vec3::new(margin, margin, margin));
        volume.signed_expand(velocity);
        self.update(leaf, volume);
        true
    }

    /// `update(leaf, volume, velocity)`.
    pub fn update_velocity(
        &mut self,
        leaf: NodeId,
        volume: &mut DbvtVolume,
        velocity: &Vec3,
    ) -> bool {
        if self.node(leaf).volume.contain(volume) {
            return false;
        }
        volume.signed_expand(velocity);
        self.update(leaf, volume);
        true
    }

    /// `update(leaf, volume, margin)`.
    pub fn update_margin(&mut self, leaf: NodeId, volume: &mut DbvtVolume, margin: f64) -> bool {
        if self.node(leaf).volume.contain(volume) {
            return false;
        }
        volume.expand(&Vec3::new(margin, margin, margin));
        self.update(leaf, volume);
        true
    }

    /// `remove(leaf)`.
    pub fn remove(&mut self, leaf: NodeId) {
        self.removeleaf(leaf);
        self.deletenode(leaf);
        self.leaves -= 1;
    }

    /// `maxdepth(node)`.
    pub fn maxdepth(&self, node: NodeId) -> i32 {
        fn get(t: &Dbvt, n: NodeId, depth: i32, maxdepth: &mut i32) {
            if t.node(n).is_internal() {
                get(t, t.node(n).childs[0], depth + 1, maxdepth);
                get(t, t.node(n).childs[1], depth + 1, maxdepth);
            } else {
                *maxdepth = bt_max(*maxdepth, depth);
            }
        }
        let mut depth = 0;
        if node != NIL {
            get(self, node, 1, &mut depth);
        }
        depth
    }

    /// `countLeaves(node)`.
    pub fn count_leaves(&self, node: NodeId) -> i32 {
        if self.node(node).is_internal() {
            self.count_leaves(self.node(node).childs[0])
                + self.count_leaves(self.node(node).childs[1])
        } else {
            1
        }
    }

    /// `extractLeaves(node, leaves)`.
    pub fn extract_leaves(&self, node: NodeId, leaves: &mut Vec<NodeId>) {
        if self.node(node).is_internal() {
            self.extract_leaves(self.node(node).childs[0], leaves);
            self.extract_leaves(self.node(node).childs[1], leaves);
        } else {
            leaves.push(node);
        }
    }

    /// `enumNodes(root, policy)`: pre-order.
    pub fn enum_nodes(&self, root: NodeId, policy: &mut dyn FnMut(NodeId)) {
        policy(root);
        if self.node(root).is_internal() {
            self.enum_nodes(self.node(root).childs[0], policy);
            self.enum_nodes(self.node(root).childs[1], policy);
        }
    }

    /// `enumLeaves(root, policy)`.
    pub fn enum_leaves(&self, root: NodeId, policy: &mut dyn FnMut(NodeId)) {
        if self.node(root).is_internal() {
            self.enum_leaves(self.node(root).childs[0], policy);
            self.enum_leaves(self.node(root).childs[1], policy);
        } else {
            policy(root);
        }
    }

    /// `collideTV(root, volume, policy)`: `policy` gets each overlapping leaf.
    pub fn collide_tv(&self, root: NodeId, vol: &DbvtVolume, policy: &mut dyn FnMut(NodeId)) {
        if root == NIL {
            return;
        }
        let volume = *vol;
        let mut stack: Vec<NodeId> = Vec::with_capacity(64);
        stack.push(root);
        while let Some(n) = stack.pop() {
            let node = self.node(n);
            if intersect(&node.volume, &volume) {
                if node.is_internal() {
                    stack.push(node.childs[0]);
                    stack.push(node.childs[1]);
                } else {
                    policy(n);
                }
            }
        }
    }

    /// `collideTT(root0, root1, policy)` with both roots in this tree.
    pub fn collide_tt(&self, root0: NodeId, root1: NodeId, policy: &mut dyn FnMut(NodeId, NodeId)) {
        let mut stack = Vec::new();
        collide_tt_impl(self, root0, self, root1, true, &mut stack, policy);
    }

    /// `collideTTpersistentStack(root0, root1, policy)`: `root0` is a node of
    /// this tree, `root1` a node of `other` (which may be this tree:
    /// pass `None`). Uses (and keeps) `m_stkStack`.
    pub fn collide_tt_persistent_stack(
        &mut self,
        root0: NodeId,
        other: Option<&Dbvt>,
        root1: NodeId,
        policy: &mut dyn FnMut(NodeId, NodeId),
    ) {
        let mut stack = std::mem::take(&mut self.stk_stack);
        match other {
            Some(o) => collide_tt_impl(self, root0, o, root1, false, &mut stack, policy),
            None => collide_tt_impl(self, root0, self, root1, true, &mut stack, policy),
        }
        self.stk_stack = stack;
    }

    /// `rayTest(root, rayFrom, rayTo, policy)` (static in Bullet).
    pub fn ray_test(
        &self,
        root: NodeId,
        ray_from: &Vec3,
        ray_to: &Vec3,
        policy: &mut dyn FnMut(NodeId),
    ) {
        if root == NIL {
            return;
        }
        let mut ray_dir = *ray_to - *ray_from;
        ray_dir.normalize();
        let mut inv = Vec3::ZERO;
        inv.x = if ray_dir.x == 0.0 {
            BT_LARGE_FLOAT
        } else {
            1.0 / ray_dir.x
        };
        inv.y = if ray_dir.y == 0.0 {
            BT_LARGE_FLOAT
        } else {
            1.0 / ray_dir.y
        };
        inv.z = if ray_dir.z == 0.0 {
            BT_LARGE_FLOAT
        } else {
            1.0 / ray_dir.z
        };
        let signs = [
            (inv.x < 0.0) as u32,
            (inv.y < 0.0) as u32,
            (inv.z < 0.0) as u32,
        ];
        let lambda_max = ray_dir.dot(&(*ray_to - *ray_from));
        let mut stack: Vec<NodeId> = vec![root];
        while let Some(n) = stack.pop() {
            let node = self.node(n);
            let bounds = [node.volume.mi, node.volume.mx];
            let mut tmin = 1.0;
            if bt_ray_aabb2(ray_from, &inv, &signs, &bounds, &mut tmin, 0.0, lambda_max) {
                if node.is_internal() {
                    stack.push(node.childs[0]);
                    stack.push(node.childs[1]);
                } else {
                    policy(n);
                }
            }
        }
    }

    /// `rayTestInternal(root, rayFrom, rayTo, rayDirectionInverse, signs,
    /// lambda_max, aabbMin, aabbMax, policy)`.
    #[allow(clippy::too_many_arguments)]
    pub fn ray_test_internal(
        &self,
        root: NodeId,
        ray_from: &Vec3,
        ray_direction_inverse: &Vec3,
        signs: &[u32; 3],
        lambda_max: f64,
        aabb_min: &Vec3,
        aabb_max: &Vec3,
        policy: &mut dyn FnMut(NodeId),
    ) {
        if root == NIL {
            return;
        }
        let mut stack: Vec<NodeId> = vec![root];
        while let Some(n) = stack.pop() {
            let node = self.node(n);
            let bounds = [node.volume.mi - *aabb_max, node.volume.mx - *aabb_min];
            let mut tmin = 1.0;
            if bt_ray_aabb2(
                ray_from,
                ray_direction_inverse,
                signs,
                &bounds,
                &mut tmin,
                0.0,
                lambda_max,
            ) {
                if node.is_internal() {
                    stack.push(node.childs[0]);
                    stack.push(node.childs[1]);
                } else {
                    policy(n);
                }
            }
        }
    }
}

/// Shared body of `collideTT` / `collideTTpersistentStack`: `a` nodes come
/// from `ta`, `b` nodes from `tb`; node identity is only possible when the
/// trees are the same.
fn collide_tt_impl(
    ta: &Dbvt,
    root0: NodeId,
    tb: &Dbvt,
    root1: NodeId,
    same: bool,
    stack: &mut Vec<(NodeId, NodeId)>,
    policy: &mut dyn FnMut(NodeId, NodeId),
) {
    if root0 == NIL || root1 == NIL {
        return;
    }
    stack.clear();
    stack.push((root0, root1));
    while let Some((a, b)) = stack.pop() {
        let na = ta.node(a);
        let nb = tb.node(b);
        if same && a == b {
            if na.is_internal() {
                stack.push((na.childs[0], na.childs[0]));
                stack.push((na.childs[1], na.childs[1]));
                stack.push((na.childs[0], na.childs[1]));
            }
        } else if intersect(&na.volume, &nb.volume) {
            if na.is_internal() {
                if nb.is_internal() {
                    stack.push((na.childs[0], nb.childs[0]));
                    stack.push((na.childs[1], nb.childs[0]));
                    stack.push((na.childs[0], nb.childs[1]));
                    stack.push((na.childs[1], nb.childs[1]));
                } else {
                    stack.push((na.childs[0], b));
                    stack.push((na.childs[1], b));
                }
            } else if nb.is_internal() {
                stack.push((a, nb.childs[0]));
                stack.push((a, nb.childs[1]));
            } else {
                policy(a, b);
            }
        }
    }
}
