//! Port of mapbox `earcut.hpp` at commit a57e4ce (2019-09-19, "parity with
//! earcut v2.2.1"), instantiated as `Earcut<uint32_t>` over
//! `ClipperLib::IntPoint` rings in the game's pathfinding library.
//!
//! Copyright (c) 2015, Mapbox. Distributed under the ISC License (see
//! `LICENSE-ISC` in this crate).
//!
//! Floating point follows the arm64 build (see `clipper`): `a*b - c*d` in a
//! single expression is fused as `fma(a, b, -(c*d))`.

use crate::clipper::{fma, IntPoint};
use crate::cxxsort;

const NIL: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
struct Node {
    i: u32,
    x: f64,
    y: f64,
    prev: usize,
    next: usize,
    z: i32,
    prev_z: usize,
    next_z: usize,
    steiner: bool,
}

struct Earcut {
    indices: Vec<u32>,
    vertices: usize,
    nodes: Vec<Node>,
    hashing: bool,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    inv_size: f64,
}

/// `mapbox::earcut<uint32_t>(rings)`: triangle vertex indices into the
/// concatenation of all rings (outer ring first, then holes).
pub fn earcut(rings: &[Vec<IntPoint>]) -> Vec<u32> {
    let mut e = Earcut {
        indices: Vec::new(),
        vertices: 0,
        nodes: Vec::new(),
        hashing: false,
        min_x: 0.0,
        max_x: 0.0,
        min_y: 0.0,
        max_y: 0.0,
        inv_size: 0.0,
    };
    e.run(rings);
    e.indices
}

impl Earcut {
    #[inline]
    fn n(&self, i: usize) -> &Node {
        &self.nodes[i]
    }
    #[inline]
    fn nm(&mut self, i: usize) -> &mut Node {
        &mut self.nodes[i]
    }

    fn run(&mut self, points: &[Vec<IntPoint>]) {
        self.indices.clear();
        self.vertices = 0;
        if points.is_empty() {
            return;
        }
        let mut threshold: i32 = 80;
        let mut i = 0;
        while threshold >= 0 && i < points.len() {
            threshold = threshold.wrapping_sub(points[i].len() as i32);
            i += 1;
        }

        let mut outer_node = self.linked_list(&points[0], true);
        if outer_node == NIL || self.n(outer_node).prev == self.n(outer_node).next {
            return;
        }
        if points.len() > 1 {
            outer_node = self.eliminate_holes(points, outer_node);
        }

        self.hashing = threshold < 0;
        if self.hashing {
            let mut p = self.n(outer_node).next;
            self.min_x = self.n(outer_node).x;
            self.max_x = self.min_x;
            self.min_y = self.n(outer_node).y;
            self.max_y = self.min_y;
            loop {
                let x = self.n(p).x;
                let y = self.n(p).y;
                self.min_x = cxx_min(self.min_x, x);
                self.min_y = cxx_min(self.min_y, y);
                self.max_x = cxx_max(self.max_x, x);
                self.max_y = cxx_max(self.max_y, y);
                p = self.n(p).next;
                if p == outer_node {
                    break;
                }
            }
            self.inv_size = cxx_max(self.max_x - self.min_x, self.max_y - self.min_y);
            self.inv_size = if self.inv_size != 0.0 {
                1.0 / self.inv_size
            } else {
                0.0
            };
        }

        self.earcut_linked(outer_node, 0);
    }

    fn linked_list(&mut self, points: &[IntPoint], clockwise: bool) -> usize {
        let mut sum = 0.0f64;
        let len = points.len();
        let mut last = NIL;
        let mut j = if len > 0 { len - 1 } else { 0 };
        // clang vectorises this loop for rings of 8+ points: whole blocks of
        // 8 compute each product separately (no fusion) and add them in
        // order; the scalar remainder keeps the fused multiply-add.
        let vec_end = if len >= 8 { len & !7 } else { 0 };
        for i in 0..len {
            let p1 = points[i];
            let p2 = points[j];
            let p20 = p2.x as f64;
            let p10 = p1.x as f64;
            let p11 = p1.y as f64;
            let p21 = p2.y as f64;
            if i < vec_end {
                sum += (p20 - p10) * (p11 + p21);
            } else {
                sum = fma(p20 - p10, p11 + p21, sum);
            }
            j = i;
        }
        if clockwise == (sum > 0.0) {
            for (i, p) in points.iter().enumerate() {
                last = self.insert_node(self.vertices + i, *p, last);
            }
        } else {
            for i in (0..len).rev() {
                last = self.insert_node(self.vertices + i, points[i], last);
            }
        }
        if last != NIL && self.equals(last, self.n(last).next) {
            self.remove_node(last);
            last = self.n(last).next;
        }
        self.vertices += len;
        last
    }

    fn filter_points(&mut self, start: usize, end: usize) -> usize {
        let mut end = if end == NIL { start } else { end };
        let mut p = start;
        loop {
            let mut again = false;
            let pn = *self.n(p);
            if !pn.steiner && (self.equals(p, pn.next) || self.area(pn.prev, p, pn.next) == 0.0) {
                self.remove_node(p);
                p = self.n(p).prev;
                end = p;
                if p == self.n(p).next {
                    break;
                }
                again = true;
            } else {
                p = pn.next;
            }
            if !(again || p != end) {
                break;
            }
        }
        end
    }

    fn earcut_linked(&mut self, ear0: usize, pass: i32) {
        let mut ear = ear0;
        if ear == NIL {
            return;
        }
        if pass == 0 && self.hashing {
            self.index_curve(ear);
        }
        let mut stop = ear;
        while self.n(ear).prev != self.n(ear).next {
            let prev = self.n(ear).prev;
            let next = self.n(ear).next;
            let is_ear = if self.hashing {
                self.is_ear_hashed(ear)
            } else {
                self.is_ear(ear)
            };
            if is_ear {
                let (pi, ei, ni) = (self.n(prev).i, self.n(ear).i, self.n(next).i);
                self.indices.push(pi);
                self.indices.push(ei);
                self.indices.push(ni);
                self.remove_node(ear);
                ear = self.n(next).next;
                stop = self.n(next).next;
                continue;
            }
            ear = next;
            if ear == stop {
                if pass == 0 {
                    let f = self.filter_points(ear, NIL);
                    self.earcut_linked(f, 1);
                } else if pass == 1 {
                    let f = self.filter_points(ear, NIL);
                    ear = self.cure_local_intersections(f);
                    self.earcut_linked(ear, 2);
                } else if pass == 2 {
                    self.split_earcut(ear);
                }
                break;
            }
        }
    }

    fn is_ear(&self, ear: usize) -> bool {
        let a = self.n(ear).prev;
        let b = ear;
        let c = self.n(ear).next;
        if self.area(a, b, c) >= 0.0 {
            return false;
        }
        let (na, nb, nc) = (self.n(a), self.n(b), self.n(c));
        let mut p = self.n(self.n(ear).next).next;
        while p != self.n(ear).prev {
            let np = self.n(p);
            if point_in_triangle(na.x, na.y, nb.x, nb.y, nc.x, nc.y, np.x, np.y)
                && self.area(np.prev, p, np.next) >= 0.0
            {
                return false;
            }
            p = np.next;
        }
        true
    }

    fn is_ear_hashed(&self, ear: usize) -> bool {
        let a = self.n(ear).prev;
        let b = ear;
        let c = self.n(ear).next;
        if self.area(a, b, c) >= 0.0 {
            return false;
        }
        let (na, nb, nc) = (self.n(a), self.n(b), self.n(c));
        let min_tx = cxx_min(na.x, cxx_min(nb.x, nc.x));
        let min_ty = cxx_min(na.y, cxx_min(nb.y, nc.y));
        let max_tx = cxx_max(na.x, cxx_max(nb.x, nc.x));
        let max_ty = cxx_max(na.y, cxx_max(nb.y, nc.y));
        let min_z = self.z_order(min_tx, min_ty);
        let max_z = self.z_order(max_tx, max_ty);

        let ep = self.n(ear).prev;
        let en = self.n(ear).next;
        let mut p = self.n(ear).next_z;
        while p != NIL && self.n(p).z <= max_z {
            let np = self.n(p);
            if p != ep
                && p != en
                && point_in_triangle(na.x, na.y, nb.x, nb.y, nc.x, nc.y, np.x, np.y)
                && self.area(np.prev, p, np.next) >= 0.0
            {
                return false;
            }
            p = np.next_z;
        }
        p = self.n(ear).prev_z;
        while p != NIL && self.n(p).z >= min_z {
            let np = self.n(p);
            if p != ep
                && p != en
                && point_in_triangle(na.x, na.y, nb.x, nb.y, nc.x, nc.y, np.x, np.y)
                && self.area(np.prev, p, np.next) >= 0.0
            {
                return false;
            }
            p = np.prev_z;
        }
        true
    }

    fn cure_local_intersections(&mut self, mut start: usize) -> usize {
        let mut p = start;
        loop {
            let a = self.n(p).prev;
            let pn = self.n(p).next;
            let b = self.n(pn).next;
            if !self.equals(a, b)
                && self.intersects(a, p, pn, b)
                && self.locally_inside(a, b)
                && self.locally_inside(b, a)
            {
                let (ai, pi, bi) = (self.n(a).i, self.n(p).i, self.n(b).i);
                self.indices.push(ai);
                self.indices.push(pi);
                self.indices.push(bi);
                self.remove_node(p);
                let pn2 = self.n(p).next;
                self.remove_node(pn2);
                p = b;
                start = b;
            }
            p = self.n(p).next;
            if p == start {
                break;
            }
        }
        self.filter_points(p, NIL)
    }

    fn split_earcut(&mut self, start: usize) {
        let mut a = start;
        loop {
            let mut b = self.n(self.n(a).next).next;
            while b != self.n(a).prev {
                if self.n(a).i != self.n(b).i && self.is_valid_diagonal(a, b) {
                    let mut c = self.split_polygon(a, b);
                    let an = self.n(a).next;
                    a = self.filter_points(a, an);
                    let cn = self.n(c).next;
                    c = self.filter_points(c, cn);
                    self.earcut_linked(a, 0);
                    self.earcut_linked(c, 0);
                    return;
                }
                b = self.n(b).next;
            }
            a = self.n(a).next;
            if a == start {
                break;
            }
        }
    }

    fn eliminate_holes(&mut self, points: &[Vec<IntPoint>], mut outer_node: usize) -> usize {
        let len = points.len();
        let mut queue: Vec<usize> = Vec::new();
        for ring in points.iter().take(len).skip(1) {
            let list = self.linked_list(ring, false);
            if list != NIL {
                if list == self.n(list).next {
                    self.nm(list).steiner = true;
                }
                queue.push(self.get_leftmost(list));
            }
        }
        let nodes = &self.nodes;
        cxxsort::sort(&mut queue, |a: &usize, b: &usize| nodes[*a].x < nodes[*b].x);
        for &h in &queue {
            self.eliminate_hole(h, outer_node);
            let on = self.n(outer_node).next;
            outer_node = self.filter_points(outer_node, on);
        }
        outer_node
    }

    fn eliminate_hole(&mut self, hole: usize, outer_node: usize) {
        let bridge = self.find_hole_bridge(hole, outer_node);
        if bridge != NIL {
            let b = self.split_polygon(bridge, hole);
            let bn = self.n(b).next;
            self.filter_points(b, bn);
        }
    }

    fn find_hole_bridge(&self, hole: usize, outer_node: usize) -> usize {
        let mut p = outer_node;
        let hx = self.n(hole).x;
        let hy = self.n(hole).y;
        let mut qx = f64::NEG_INFINITY;
        let mut m = NIL;
        loop {
            let np = self.n(p);
            let nn = self.n(np.next);
            if hy <= np.y && hy >= nn.y && nn.y != np.y {
                let x = np.x + (hy - np.y) * (nn.x - np.x) / (nn.y - np.y);
                if x <= hx && x > qx {
                    qx = x;
                    if x == hx {
                        if hy == np.y {
                            return p;
                        }
                        if hy == nn.y {
                            return np.next;
                        }
                    }
                    m = if np.x < nn.x { p } else { np.next };
                }
            }
            p = np.next;
            if p == outer_node {
                break;
            }
        }
        if m == NIL {
            return NIL;
        }
        if hx == qx {
            return m;
        }
        let stop = m;
        let mut tan_min = f64::INFINITY;
        p = m;
        let mx = self.n(m).x;
        let my = self.n(m).y;
        loop {
            let np = self.n(p);
            if hx >= np.x
                && np.x >= mx
                && hx != np.x
                && point_in_triangle(
                    if hy < my { hx } else { qx },
                    hy,
                    mx,
                    my,
                    if hy < my { qx } else { hx },
                    hy,
                    np.x,
                    np.y,
                )
            {
                let tan_cur = (hy - np.y).abs() / (hx - np.x);
                if self.locally_inside(p, hole)
                    && (tan_cur < tan_min
                        || (tan_cur == tan_min
                            && (np.x > self.n(m).x || self.sector_contains_sector(m, p))))
                {
                    m = p;
                    tan_min = tan_cur;
                }
            }
            p = np.next;
            if p == stop {
                break;
            }
        }
        m
    }

    fn sector_contains_sector(&self, m: usize, p: usize) -> bool {
        self.area(self.n(m).prev, m, self.n(p).prev) < 0.0
            && self.area(self.n(p).next, m, self.n(m).next) < 0.0
    }

    fn index_curve(&mut self, start: usize) {
        let mut p = start;
        loop {
            let z = if self.n(p).z != 0 {
                self.n(p).z
            } else {
                self.z_order(self.n(p).x, self.n(p).y)
            };
            let (prev, next) = (self.n(p).prev, self.n(p).next);
            let np = self.nm(p);
            np.z = z;
            np.prev_z = prev;
            np.next_z = next;
            p = next;
            if p == start {
                break;
            }
        }
        let pz = self.n(p).prev_z;
        self.nm(pz).next_z = NIL;
        self.nm(p).prev_z = NIL;
        self.sort_linked(p);
    }

    #[allow(clippy::if_same_then_else)]
    fn sort_linked(&mut self, mut list: usize) -> usize {
        let mut in_size: i32 = 1;
        loop {
            let mut p = list;
            list = NIL;
            let mut tail = NIL;
            let mut num_merges = 0;
            while p != NIL {
                num_merges += 1;
                let mut q = p;
                let mut p_size = 0;
                for _ in 0..in_size {
                    p_size += 1;
                    q = self.n(q).next_z;
                    if q == NIL {
                        break;
                    }
                }
                let mut q_size = in_size;
                while p_size > 0 || (q_size > 0 && q != NIL) {
                    let e;
                    if p_size == 0 {
                        e = q;
                        q = self.n(q).next_z;
                        q_size -= 1;
                    } else if q_size == 0 || q == NIL {
                        e = p;
                        p = self.n(p).next_z;
                        p_size -= 1;
                    } else if self.n(p).z <= self.n(q).z {
                        e = p;
                        p = self.n(p).next_z;
                        p_size -= 1;
                    } else {
                        e = q;
                        q = self.n(q).next_z;
                        q_size -= 1;
                    }
                    if tail != NIL {
                        self.nm(tail).next_z = e;
                    } else {
                        list = e;
                    }
                    self.nm(e).prev_z = tail;
                    tail = e;
                }
                p = q;
            }
            self.nm(tail).next_z = NIL;
            if num_merges <= 1 {
                return list;
            }
            in_size *= 2;
        }
    }

    fn z_order(&self, x_: f64, y_: f64) -> i32 {
        // static_cast<int32_t>(double) is emitted as a 64-bit conversion
        // followed by truncation to 32 bits.
        let mut x = ((32767.0 * (x_ - self.min_x) * self.inv_size) as i64) as i32;
        let mut y = ((32767.0 * (y_ - self.min_y) * self.inv_size) as i64) as i32;
        x = (x | (x << 8)) & 0x00FF00FF;
        x = (x | (x << 4)) & 0x0F0F0F0F;
        x = (x | (x << 2)) & 0x33333333;
        x = (x | (x << 1)) & 0x55555555;
        y = (y | (y << 8)) & 0x00FF00FF;
        y = (y | (y << 4)) & 0x0F0F0F0F;
        y = (y | (y << 2)) & 0x33333333;
        y = (y | (y << 1)) & 0x55555555;
        x | (y << 1)
    }

    fn get_leftmost(&self, start: usize) -> usize {
        let mut p = start;
        let mut leftmost = start;
        loop {
            let np = self.n(p);
            let nl = self.n(leftmost);
            if np.x < nl.x || (np.x == nl.x && np.y < nl.y) {
                leftmost = p;
            }
            p = np.next;
            if p == start {
                break;
            }
        }
        leftmost
    }

    fn is_valid_diagonal(&self, a: usize, b: usize) -> bool {
        let (na, nb) = (self.n(a), self.n(b));
        self.n(na.next).i != nb.i
            && self.n(na.prev).i != nb.i
            && !self.intersects_polygon(a, b)
            && ((self.locally_inside(a, b)
                && self.locally_inside(b, a)
                && self.middle_inside(a, b)
                && (self.area(na.prev, a, nb.prev) != 0.0 || self.area(a, nb.prev, b) != 0.0))
                || (self.equals(a, b)
                    && self.area(na.prev, a, na.next) > 0.0
                    && self.area(nb.prev, b, nb.next) > 0.0))
    }

    #[inline]
    fn area(&self, p: usize, q: usize, r: usize) -> f64 {
        let (p, q, r) = (self.n(p), self.n(q), self.n(r));
        fma(q.y - p.y, r.x - q.x, -((q.x - p.x) * (r.y - q.y)))
    }

    #[inline]
    fn equals(&self, p1: usize, p2: usize) -> bool {
        let (a, b) = (self.n(p1), self.n(p2));
        a.x == b.x && a.y == b.y
    }

    fn intersects(&self, p1: usize, q1: usize, p2: usize, q2: usize) -> bool {
        let o1 = sign(self.area(p1, q1, p2));
        let o2 = sign(self.area(p1, q1, q2));
        let o3 = sign(self.area(p2, q2, p1));
        let o4 = sign(self.area(p2, q2, q1));
        if o1 != o2 && o3 != o4 {
            return true;
        }
        if o1 == 0 && self.on_segment(p1, p2, q1) {
            return true;
        }
        if o2 == 0 && self.on_segment(p1, q2, q1) {
            return true;
        }
        if o3 == 0 && self.on_segment(p2, p1, q2) {
            return true;
        }
        if o4 == 0 && self.on_segment(p2, q1, q2) {
            return true;
        }
        false
    }

    fn on_segment(&self, p: usize, q: usize, r: usize) -> bool {
        let (p, q, r) = (self.n(p), self.n(q), self.n(r));
        q.x <= cxx_max(p.x, r.x)
            && q.x >= cxx_min(p.x, r.x)
            && q.y <= cxx_max(p.y, r.y)
            && q.y >= cxx_min(p.y, r.y)
    }

    fn intersects_polygon(&self, a: usize, b: usize) -> bool {
        let ai = self.n(a).i;
        let bi = self.n(b).i;
        let mut p = a;
        loop {
            let np = self.n(p);
            let nni = self.n(np.next).i;
            if np.i != ai
                && nni != ai
                && np.i != bi
                && nni != bi
                && self.intersects(p, np.next, a, b)
            {
                return true;
            }
            p = np.next;
            if p == a {
                break;
            }
        }
        false
    }

    fn locally_inside(&self, a: usize, b: usize) -> bool {
        let na = self.n(a);
        if self.area(na.prev, a, na.next) < 0.0 {
            self.area(a, b, na.next) >= 0.0 && self.area(a, na.prev, b) >= 0.0
        } else {
            self.area(a, b, na.prev) < 0.0 || self.area(a, na.next, b) < 0.0
        }
    }

    fn middle_inside(&self, a: usize, b: usize) -> bool {
        let mut p = a;
        let mut inside = false;
        let px = (self.n(a).x + self.n(b).x) / 2.0;
        let py = (self.n(a).y + self.n(b).y) / 2.0;
        loop {
            let np = self.n(p);
            let nn = self.n(np.next);
            if ((np.y > py) != (nn.y > py))
                && nn.y != np.y
                && (px < (nn.x - np.x) * (py - np.y) / (nn.y - np.y) + np.x)
            {
                inside = !inside;
            }
            p = np.next;
            if p == a {
                break;
            }
        }
        inside
    }

    fn split_polygon(&mut self, a: usize, b: usize) -> usize {
        let (na, nb) = (*self.n(a), *self.n(b));
        let a2 = self.new_node(na.i, na.x, na.y);
        let b2 = self.new_node(nb.i, nb.x, nb.y);
        let an = na.next;
        let bp = nb.prev;
        self.nm(a).next = b;
        self.nm(b).prev = a;
        self.nm(a2).next = an;
        self.nm(an).prev = a2;
        self.nm(b2).next = a2;
        self.nm(a2).prev = b2;
        self.nm(bp).next = b2;
        self.nm(b2).prev = bp;
        b2
    }

    fn new_node(&mut self, i: u32, x: f64, y: f64) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            i,
            x,
            y,
            prev: NIL,
            next: NIL,
            z: 0,
            prev_z: NIL,
            next_z: NIL,
            steiner: false,
        });
        id
    }

    fn insert_node(&mut self, i: usize, pt: IntPoint, last: usize) -> usize {
        let p = self.new_node(i as u32, pt.x as f64, pt.y as f64);
        if last == NIL {
            self.nm(p).prev = p;
            self.nm(p).next = p;
        } else {
            let ln = self.n(last).next;
            self.nm(p).next = ln;
            self.nm(p).prev = last;
            self.nm(ln).prev = p;
            self.nm(last).next = p;
        }
        p
    }

    fn remove_node(&mut self, p: usize) {
        let np = *self.n(p);
        self.nm(np.next).prev = np.prev;
        self.nm(np.prev).next = np.next;
        if np.prev_z != NIL {
            self.nm(np.prev_z).next_z = np.next_z;
        }
        if np.next_z != NIL {
            self.nm(np.next_z).prev_z = np.prev_z;
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn point_in_triangle(
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    px: f64,
    py: f64,
) -> bool {
    fma(cx - px, ay - py, -((ax - px) * (cy - py))) >= 0.0
        && fma(ax - px, by - py, -((bx - px) * (ay - py))) >= 0.0
        && fma(bx - px, cy - py, -((cx - px) * (by - py))) >= 0.0
}

#[inline]
fn sign(val: f64) -> i32 {
    (0.0 < val) as i32 - (val < 0.0) as i32
}

/// `std::min<double>(a, b)`: `(b < a) ? b : a`.
#[inline]
fn cxx_min(a: f64, b: f64) -> f64 {
    if b < a {
        b
    } else {
        a
    }
}

/// `std::max<double>(a, b)`: `(a < b) ? b : a`.
#[inline]
fn cxx_max(a: f64, b: f64) -> f64 {
    if a < b {
        b
    } else {
        a
    }
}
