//! Port of Clipper 6.4.2 `ClipperBase` and `Clipper` (the Vatti sweep).
//!
//! Copyright (c) Angus Johnson 2010-2017. Distributed under the Boost
//! Software License, Version 1.0 (see `LICENSE-BOOST` in this crate).

use std::collections::BinaryHeap;

use super::{
    fma, pip_cross, round, ClipType, ClipperException, IntPoint, IntRect, Path, Paths,
    PolyFillType, PolyTree, PolyType, HI_RANGE, IO_PRESERVE_COLLINEAR, IO_REVERSE_SOLUTION,
    IO_STRICTLY_SIMPLE, LO_RANGE,
};
use crate::cxxsort;

/// A null pointer.
const NIL: usize = usize::MAX;
const HORIZONTAL: f64 = -1.0E+40;
/// Edge not currently 'owning' a solution.
const UNASSIGNED: i32 = -1;
/// Edge that would otherwise close a path.
const SKIP: i32 = -2;
/// `EdgeSide` (0 after `memset`).
const ES_LEFT: i32 = 1;
const ES_RIGHT: i32 = 2;

/// An exception thrown inside the sweep (caught by `ExecuteInternal`).
type Exc<T = ()> = Result<T, ClipperException>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    RightToLeft,
    LeftToRight,
}

#[derive(Clone, Copy, Debug)]
struct TEdge {
    bot: IntPoint,
    curr: IntPoint,
    top: IntPoint,
    dx: f64,
    poly_typ: PolyType,
    side: i32,
    wind_delta: i32,
    wind_cnt: i32,
    wind_cnt2: i32,
    out_idx: i32,
    next: usize,
    prev: usize,
    next_in_lml: usize,
    next_in_ael: usize,
    prev_in_ael: usize,
    next_in_sel: usize,
    prev_in_sel: usize,
}

impl TEdge {
    /// `std::memset(e, 0, sizeof(TEdge))`.
    const ZERO: TEdge = TEdge {
        bot: IntPoint::new(0, 0),
        curr: IntPoint::new(0, 0),
        top: IntPoint::new(0, 0),
        dx: 0.0,
        poly_typ: PolyType::Subject,
        side: 0,
        wind_delta: 0,
        wind_cnt: 0,
        wind_cnt2: 0,
        out_idx: 0,
        next: NIL,
        prev: NIL,
        next_in_lml: NIL,
        next_in_ael: NIL,
        prev_in_ael: NIL,
        next_in_sel: NIL,
        prev_in_sel: NIL,
    };
}

#[derive(Clone, Copy, Debug)]
struct IntersectNode {
    edge1: usize,
    edge2: usize,
    pt: IntPoint,
}

#[derive(Clone, Copy, Debug)]
struct LocalMinimum {
    y: i64,
    left_bound: usize,
    right_bound: usize,
}

#[derive(Clone, Copy, Debug)]
struct OutRec {
    idx: i32,
    is_hole: bool,
    is_open: bool,
    first_left: usize,
    poly_nd: usize,
    pts: usize,
    bottom_pt: usize,
}

#[derive(Clone, Copy, Debug)]
struct OutPt {
    idx: i32,
    pt: IntPoint,
    next: usize,
    prev: usize,
}

#[derive(Clone, Copy, Debug)]
struct Join {
    out_pt1: usize,
    out_pt2: usize,
    off_pt: IntPoint,
}

#[inline(always)]
fn is_horizontal(e: &TEdge) -> bool {
    e.dx == HORIZONTAL
}

#[inline]
fn get_dx(pt1: IntPoint, pt2: IntPoint) -> f64 {
    if pt1.y == pt2.y {
        HORIZONTAL
    } else {
        pt2.x.wrapping_sub(pt1.x) as f64 / pt2.y.wrapping_sub(pt1.y) as f64
    }
}

#[inline]
fn set_dx(e: &mut TEdge) {
    let dy = e.top.y.wrapping_sub(e.bot.y);
    if dy == 0 {
        e.dx = HORIZONTAL;
    } else {
        e.dx = e.top.x.wrapping_sub(e.bot.x) as f64 / dy as f64;
    }
}

#[inline]
fn top_x(edge: &TEdge, current_y: i64) -> i64 {
    if current_y == edge.top.y {
        edge.top.x
    } else {
        edge.bot
            .x
            .wrapping_add(round(edge.dx * current_y.wrapping_sub(edge.bot.y) as f64))
    }
}

/// `Int128Mul(a, b) == Int128Mul(c, d)`; the product is exact.
#[inline]
fn mul128_eq(a: i64, b: i64, c: i64, d: i64) -> bool {
    (a as i128) * (b as i128) == (c as i128) * (d as i128)
}

fn slopes_equal_e(e1: &TEdge, e2: &TEdge, full: bool) -> bool {
    let a = e1.top.y.wrapping_sub(e1.bot.y);
    let b = e2.top.x.wrapping_sub(e2.bot.x);
    let c = e1.top.x.wrapping_sub(e1.bot.x);
    let d = e2.top.y.wrapping_sub(e2.bot.y);
    if full {
        mul128_eq(a, b, c, d)
    } else {
        a.wrapping_mul(b) == c.wrapping_mul(d)
    }
}

fn slopes_equal3(pt1: IntPoint, pt2: IntPoint, pt3: IntPoint, full: bool) -> bool {
    let a = pt1.y.wrapping_sub(pt2.y);
    let b = pt2.x.wrapping_sub(pt3.x);
    let c = pt1.x.wrapping_sub(pt2.x);
    let d = pt2.y.wrapping_sub(pt3.y);
    if full {
        mul128_eq(a, b, c, d)
    } else {
        a.wrapping_mul(b) == c.wrapping_mul(d)
    }
}

fn slopes_equal4(pt1: IntPoint, pt2: IntPoint, pt3: IntPoint, pt4: IntPoint, full: bool) -> bool {
    let a = pt1.y.wrapping_sub(pt2.y);
    let b = pt3.x.wrapping_sub(pt4.x);
    let c = pt1.x.wrapping_sub(pt2.x);
    let d = pt3.y.wrapping_sub(pt4.y);
    if full {
        mul128_eq(a, b, c, d)
    } else {
        a.wrapping_mul(b) == c.wrapping_mul(d)
    }
}

fn intersect_point(e1: &TEdge, e2: &TEdge) -> IntPoint {
    let mut ip = IntPoint::default();
    if e1.dx == e2.dx {
        ip.y = e1.curr.y;
        ip.x = top_x(e1, ip.y);
        return ip;
    } else if e1.dx == 0.0 {
        ip.x = e1.bot.x;
        if is_horizontal(e2) {
            ip.y = e2.bot.y;
        } else {
            let b2 = e2.bot.y as f64 - (e2.bot.x as f64 / e2.dx);
            ip.y = round(ip.x as f64 / e2.dx + b2);
        }
    } else if e2.dx == 0.0 {
        ip.x = e2.bot.x;
        if is_horizontal(e1) {
            ip.y = e1.bot.y;
        } else {
            let b1 = e1.bot.y as f64 - (e1.bot.x as f64 / e1.dx);
            ip.y = round(ip.x as f64 / e1.dx + b1);
        }
    } else {
        let b1 = fma(-(e1.bot.y as f64), e1.dx, e1.bot.x as f64);
        let b2 = fma(-(e2.bot.y as f64), e2.dx, e2.bot.x as f64);
        let q = (b2 - b1) / (e1.dx - e2.dx);
        ip.y = round(q);
        if e1.dx.abs() < e2.dx.abs() {
            ip.x = round(fma(e1.dx, q, b1));
        } else {
            ip.x = round(fma(e2.dx, q, b2));
        }
    }

    if ip.y < e1.top.y || ip.y < e2.top.y {
        if e1.top.y > e2.top.y {
            ip.y = e1.top.y;
        } else {
            ip.y = e2.top.y;
        }
        if e1.dx.abs() < e2.dx.abs() {
            ip.x = top_x(e1, ip.y);
        } else {
            ip.x = top_x(e2, ip.y);
        }
    }
    // finally, don't allow 'ip' to be BELOW curr.Y (ie bottom of scanbeam) ...
    if ip.y > e1.curr.y {
        ip.y = e1.curr.y;
        // use the more vertical edge to derive X ...
        if e1.dx.abs() > e2.dx.abs() {
            ip.x = top_x(e2, ip.y);
        } else {
            ip.x = top_x(e1, ip.y);
        }
    }
    ip
}

#[inline]
fn reverse_horizontal(e: &mut TEdge) {
    std::mem::swap(&mut e.top.x, &mut e.bot.x);
}

fn pt2_is_between_pt1_and_pt3(pt1: IntPoint, pt2: IntPoint, pt3: IntPoint) -> bool {
    if pt1 == pt3 || pt1 == pt2 || pt3 == pt2 {
        false
    } else if pt1.x != pt3.x {
        (pt2.x > pt1.x) == (pt2.x < pt3.x)
    } else {
        (pt2.y > pt1.y) == (pt2.y < pt3.y)
    }
}

fn horz_segments_overlap(mut seg1a: i64, mut seg1b: i64, mut seg2a: i64, mut seg2b: i64) -> bool {
    if seg1a > seg1b {
        std::mem::swap(&mut seg1a, &mut seg1b);
    }
    if seg2a > seg2b {
        std::mem::swap(&mut seg2a, &mut seg2b);
    }
    (seg1a < seg2b) && (seg2a < seg1b)
}

fn range_test(pt: IntPoint, use_full_range: &mut bool) -> Exc {
    // The compiled checks are `X < -range` (not `-X > range`).
    if *use_full_range {
        if pt.x > HI_RANGE || pt.y > HI_RANGE || pt.x < -HI_RANGE || pt.y < -HI_RANGE {
            return Err(ClipperException("Coordinate outside allowed range"));
        }
    } else if pt.x > LO_RANGE || pt.y > LO_RANGE || pt.x < -LO_RANGE || pt.y < -LO_RANGE {
        *use_full_range = true;
        return range_test(pt, use_full_range);
    }
    Ok(())
}

fn get_overlap(a1: i64, a2: i64, b1: i64, b2: i64) -> (bool, i64, i64) {
    let (left, right);
    if a1 < a2 {
        if b1 < b2 {
            left = a1.max(b1);
            right = a2.min(b2);
        } else {
            left = a1.max(b2);
            right = a2.min(b1);
        }
    } else if b1 < b2 {
        left = a2.max(b1);
        right = a1.min(b2);
    } else {
        left = a2.max(b2);
        right = a1.min(b1);
    }
    (left < right, left, right)
}

/// `std::max<double>` / `std::min<double>` (first argument wins ties/NaN).
#[inline]
fn cxx_max(a: f64, b: f64) -> f64 {
    if a < b {
        b
    } else {
        a
    }
}
#[inline]
fn cxx_min(a: f64, b: f64) -> f64 {
    if b < a {
        b
    } else {
        a
    }
}

/// `ClipperLib::Clipper` (including its virtual base `ClipperBase`).
#[derive(Clone, Debug)]
pub struct Clipper {
    // ClipperBase
    current_lm: usize,
    minima_list: Vec<LocalMinimum>,
    use_full_range: bool,
    edges: Vec<TEdge>,
    preserve_collinear: bool,
    has_open_paths: bool,
    out_recs: Vec<OutRec>,
    out_pts: Vec<OutPt>,
    active_edges: usize,
    scanbeam: BinaryHeap<i64>,
    // Clipper
    joins: Vec<Join>,
    ghost_joins: Vec<Join>,
    intersect_list: Vec<IntersectNode>,
    clip_type: ClipType,
    maxima: Vec<i64>,
    sorted_edges: usize,
    execute_locked: bool,
    clip_fill_type: PolyFillType,
    subj_fill_type: PolyFillType,
    reverse_output: bool,
    using_poly_tree: bool,
    strict_simple: bool,
}

impl Default for Clipper {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Clipper {
    /// `Clipper(int initOptions = 0)`.
    pub fn new(init_options: i32) -> Self {
        Self {
            current_lm: 0,
            minima_list: Vec::new(),
            use_full_range: false,
            edges: Vec::new(),
            preserve_collinear: (init_options & IO_PRESERVE_COLLINEAR) != 0,
            has_open_paths: false,
            out_recs: Vec::new(),
            out_pts: Vec::new(),
            active_edges: NIL,
            scanbeam: BinaryHeap::new(),
            joins: Vec::new(),
            ghost_joins: Vec::new(),
            intersect_list: Vec::new(),
            clip_type: ClipType::Intersection,
            maxima: Vec::new(),
            sorted_edges: NIL,
            execute_locked: false,
            clip_fill_type: PolyFillType::EvenOdd,
            subj_fill_type: PolyFillType::EvenOdd,
            reverse_output: (init_options & IO_REVERSE_SOLUTION) != 0,
            using_poly_tree: false,
            strict_simple: (init_options & IO_STRICTLY_SIMPLE) != 0,
        }
    }

    /// `PreserveCollinear()`.
    pub fn preserve_collinear(&self) -> bool {
        self.preserve_collinear
    }
    /// `PreserveCollinear(bool)`.
    pub fn set_preserve_collinear(&mut self, value: bool) {
        self.preserve_collinear = value;
    }
    /// `ReverseSolution()`.
    pub fn reverse_solution(&self) -> bool {
        self.reverse_output
    }
    /// `ReverseSolution(bool)`.
    pub fn set_reverse_solution(&mut self, value: bool) {
        self.reverse_output = value;
    }
    /// `StrictlySimple()`.
    pub fn strictly_simple(&self) -> bool {
        self.strict_simple
    }
    /// `StrictlySimple(bool)`.
    pub fn set_strictly_simple(&mut self, value: bool) {
        self.strict_simple = value;
    }

    #[inline(always)]
    fn e(&self, i: usize) -> &TEdge {
        &self.edges[i]
    }
    #[inline(always)]
    fn em(&mut self, i: usize) -> &mut TEdge {
        &mut self.edges[i]
    }
    #[inline(always)]
    fn op(&self, i: usize) -> &OutPt {
        &self.out_pts[i]
    }
    #[inline(always)]
    fn opm(&mut self, i: usize) -> &mut OutPt {
        &mut self.out_pts[i]
    }
    #[inline(always)]
    fn or(&self, i: usize) -> &OutRec {
        &self.out_recs[i]
    }
    #[inline(always)]
    fn orm(&mut self, i: usize) -> &mut OutRec {
        &mut self.out_recs[i]
    }
    #[inline(always)]
    fn horz(&self, i: usize) -> bool {
        is_horizontal(&self.edges[i])
    }

    fn new_out_pt(&mut self, idx: i32, pt: IntPoint, next: usize, prev: usize) -> usize {
        let id = self.out_pts.len();
        self.out_pts.push(OutPt {
            idx,
            pt,
            next,
            prev,
        });
        id
    }

    // -----------------------------------------------------------------------
    // ClipperBase
    // -----------------------------------------------------------------------

    fn init_edge(&mut self, e: usize, e_next: usize, e_prev: usize, pt: IntPoint) {
        let ed = &mut self.edges[e];
        *ed = TEdge::ZERO;
        ed.next = e_next;
        ed.prev = e_prev;
        ed.curr = pt;
        ed.out_idx = UNASSIGNED;
    }

    fn init_edge2(&mut self, e: usize, pt: PolyType) {
        let next_curr = self.e(self.e(e).next).curr;
        let ed = &mut self.edges[e];
        if ed.curr.y >= next_curr.y {
            ed.bot = ed.curr;
            ed.top = next_curr;
        } else {
            ed.top = ed.curr;
            ed.bot = next_curr;
        }
        set_dx(ed);
        ed.poly_typ = pt;
    }

    fn remove_edge(&mut self, e: usize) -> usize {
        let (prev, next) = (self.e(e).prev, self.e(e).next);
        self.em(prev).next = next;
        self.em(next).prev = prev;
        self.em(e).prev = NIL;
        next
    }

    fn find_next_loc_min(&self, mut e: usize) -> usize {
        loop {
            while self.e(e).bot != self.e(self.e(e).prev).bot || self.e(e).curr == self.e(e).top {
                e = self.e(e).next;
            }
            if !self.horz(e) && !self.horz(self.e(e).prev) {
                break;
            }
            while self.horz(self.e(e).prev) {
                e = self.e(e).prev;
            }
            let e2 = e;
            while self.horz(e) {
                e = self.e(e).next;
            }
            if self.e(e).top.y == self.e(self.e(e).prev).bot.y {
                continue; // ie just an intermediate horz.
            }
            if self.e(self.e(e2).prev).bot.x < self.e(e).bot.x {
                e = e2;
            }
            break;
        }
        e
    }

    fn process_bound(&mut self, mut e: usize, next_is_forward: bool) -> usize {
        let mut result = e;
        if self.e(e).out_idx == SKIP {
            // if edges still remain in the current bound beyond the skip edge
            // then create another LocMin and call ProcessBound once more
            if next_is_forward {
                while self.e(e).top.y == self.e(self.e(e).next).bot.y {
                    e = self.e(e).next;
                }
                while e != result && self.horz(e) {
                    e = self.e(e).prev;
                }
            } else {
                while self.e(e).top.y == self.e(self.e(e).prev).bot.y {
                    e = self.e(e).prev;
                }
                while e != result && self.horz(e) {
                    e = self.e(e).next;
                }
            }

            if e == result {
                if next_is_forward {
                    result = self.e(e).next;
                } else {
                    result = self.e(e).prev;
                }
            } else {
                // there are more edges in the bound beyond result starting with E
                if next_is_forward {
                    e = self.e(result).next;
                } else {
                    e = self.e(result).prev;
                }
                let loc_min = LocalMinimum {
                    y: self.e(e).bot.y,
                    left_bound: NIL,
                    right_bound: e,
                };
                self.em(e).wind_delta = 0;
                result = self.process_bound(e, next_is_forward);
                self.minima_list.push(loc_min);
            }
            return result;
        }

        if self.horz(e) {
            // We need to be careful with open paths because this may not be a
            // true local minima (ie E may be following a skip edge).
            // Also, consecutive horz. edges may start heading left before going right.
            let e_start = if next_is_forward {
                self.e(e).prev
            } else {
                self.e(e).next
            };
            if self.horz(e_start) {
                // ie an adjoining horizontal skip edge
                if self.e(e_start).bot.x != self.e(e).bot.x
                    && self.e(e_start).top.x != self.e(e).bot.x
                {
                    reverse_horizontal(self.em(e));
                }
            } else if self.e(e_start).bot.x != self.e(e).bot.x {
                reverse_horizontal(self.em(e));
            }
        }

        let e_start = e;
        if next_is_forward {
            while self.e(result).top.y == self.e(self.e(result).next).bot.y
                && self.e(self.e(result).next).out_idx != SKIP
            {
                result = self.e(result).next;
            }
            if self.horz(result) && self.e(self.e(result).next).out_idx != SKIP {
                // nb: at the top of a bound, horizontals are added to the bound
                // only when the preceding edge attaches to the horizontal's left vertex
                // unless a Skip edge is encountered when that becomes the top divide
                let mut horz = result;
                while self.horz(self.e(horz).prev) {
                    horz = self.e(horz).prev;
                }
                if self.e(self.e(horz).prev).top.x > self.e(self.e(result).next).top.x {
                    result = self.e(horz).prev;
                }
            }
            while e != result {
                let n = self.e(e).next;
                self.em(e).next_in_lml = n;
                if self.horz(e) && e != e_start && self.e(e).bot.x != self.e(self.e(e).prev).top.x {
                    reverse_horizontal(self.em(e));
                }
                e = self.e(e).next;
            }
            if self.horz(e) && e != e_start && self.e(e).bot.x != self.e(self.e(e).prev).top.x {
                reverse_horizontal(self.em(e));
            }
            result = self.e(result).next; // move to the edge just beyond current bound
        } else {
            while self.e(result).top.y == self.e(self.e(result).prev).bot.y
                && self.e(self.e(result).prev).out_idx != SKIP
            {
                result = self.e(result).prev;
            }
            if self.horz(result) && self.e(self.e(result).prev).out_idx != SKIP {
                let mut horz = result;
                while self.horz(self.e(horz).next) {
                    horz = self.e(horz).next;
                }
                let hn_top_x = self.e(self.e(horz).next).top.x;
                let rp_top_x = self.e(self.e(result).prev).top.x;
                if hn_top_x >= rp_top_x {
                    result = self.e(horz).next;
                }
            }
            while e != result {
                let p = self.e(e).prev;
                self.em(e).next_in_lml = p;
                if self.horz(e) && e != e_start && self.e(e).bot.x != self.e(self.e(e).next).top.x {
                    reverse_horizontal(self.em(e));
                }
                e = self.e(e).prev;
            }
            if self.horz(e) && e != e_start && self.e(e).bot.x != self.e(self.e(e).next).top.x {
                reverse_horizontal(self.em(e));
            }
            result = self.e(result).prev; // move to the edge just beyond current bound
        }
        result
    }

    /// `ClipperBase::AddPath()`. `Err` is the exception the original throws.
    pub fn add_path(
        &mut self,
        pg: &[IntPoint],
        poly_typ: PolyType,
        closed: bool,
    ) -> Result<bool, ClipperException> {
        if !closed && poly_typ == PolyType::Clip {
            return Err(ClipperException("AddPath: Open paths must be subject."));
        }

        let mut high_i = pg.len() as i32 - 1;
        if closed {
            while high_i > 0 && pg[high_i as usize] == pg[0] {
                high_i -= 1;
            }
        }
        while high_i > 0 && pg[high_i as usize] == pg[high_i as usize - 1] {
            high_i -= 1;
        }
        if (closed && high_i < 2) || (!closed && high_i < 1) {
            return Ok(false);
        }
        let hi = high_i as usize;

        // create a new edge array ...
        let base = self.edges.len();
        self.edges.resize(base + hi + 1, TEdge::ZERO);
        let mut is_flat = true;

        // 1. Basic (first) edge initialization ...
        let r: Exc = (|| {
            range_test(pg[0], &mut self.use_full_range)?;
            range_test(pg[hi], &mut self.use_full_range)?;
            self.init_edge(base, base + 1, base + hi, pg[0]);
            self.init_edge(base + hi, base, base + hi - 1, pg[hi]);
            for i in (1..hi).rev() {
                range_test(pg[i], &mut self.use_full_range)?;
                self.init_edge(base + i, base + i + 1, base + i - 1, pg[i]);
            }
            Ok(())
        })();
        if let Err(x) = r {
            self.edges.truncate(base);
            return Err(x);
        }

        let mut e_start = base;

        // 2. Remove duplicate vertices, and (when closed) collinear edges ...
        let mut e = e_start;
        let mut e_loop_stop = e_start;
        loop {
            // nb: allows matching start and end points when not Closed ...
            if self.e(e).curr == self.e(self.e(e).next).curr
                && (closed || self.e(e).next != e_start)
            {
                if e == self.e(e).next {
                    break;
                }
                if e == e_start {
                    e_start = self.e(e).next;
                }
                e = self.remove_edge(e);
                e_loop_stop = e;
                continue;
            }
            if self.e(e).prev == self.e(e).next {
                break; // only two vertices
            } else if closed
                && slopes_equal3(
                    self.e(self.e(e).prev).curr,
                    self.e(e).curr,
                    self.e(self.e(e).next).curr,
                    self.use_full_range,
                )
                && (!self.preserve_collinear
                    || !pt2_is_between_pt1_and_pt3(
                        self.e(self.e(e).prev).curr,
                        self.e(e).curr,
                        self.e(self.e(e).next).curr,
                    ))
            {
                // Collinear edges are allowed for open paths but in closed paths
                // the default is to merge adjacent collinear edges into a single edge.
                // However, if the PreserveCollinear property is enabled, only overlapping
                // collinear edges (ie spikes) will be removed from closed paths.
                if e == e_start {
                    e_start = self.e(e).next;
                }
                e = self.remove_edge(e);
                e = self.e(e).prev;
                e_loop_stop = e;
                continue;
            }
            e = self.e(e).next;
            if (e == e_loop_stop) || (!closed && self.e(e).next == e_start) {
                break;
            }
        }

        if (!closed && (e == self.e(e).next)) || (closed && (self.e(e).prev == self.e(e).next)) {
            self.edges.truncate(base);
            return Ok(false);
        }

        if !closed {
            self.has_open_paths = true;
            let p = self.e(e_start).prev;
            self.em(p).out_idx = SKIP;
        }

        // 3. Do second stage of edge initialization ...
        e = e_start;
        loop {
            self.init_edge2(e, poly_typ);
            e = self.e(e).next;
            if is_flat && self.e(e).curr.y != self.e(e_start).curr.y {
                is_flat = false;
            }
            if e == e_start {
                break;
            }
        }

        // 4. Finally, add edge bounds to LocalMinima list ...

        // Totally flat paths must be handled differently when adding them
        // to LocalMinima list to avoid endless loops etc ...
        if is_flat {
            if closed {
                self.edges.truncate(base);
                return Ok(false);
            }
            let p = self.e(e).prev;
            self.em(p).out_idx = SKIP;
            let loc_min = LocalMinimum {
                y: self.e(e).bot.y,
                left_bound: NIL,
                right_bound: e,
            };
            self.em(e).side = ES_RIGHT;
            self.em(e).wind_delta = 0;
            loop {
                if self.e(e).bot.x != self.e(self.e(e).prev).top.x {
                    reverse_horizontal(self.em(e));
                }
                if self.e(self.e(e).next).out_idx == SKIP {
                    break;
                }
                let n = self.e(e).next;
                self.em(e).next_in_lml = n;
                e = n;
            }
            self.minima_list.push(loc_min);
            return Ok(true);
        }

        let mut e_min = NIL;

        // workaround to avoid an endless loop in the while loop below when
        // open paths have matching start and end points ...
        let ep = self.e(e).prev;
        if self.e(ep).bot == self.e(ep).top {
            e = self.e(e).next;
        }

        loop {
            e = self.find_next_loc_min(e);
            if e == e_min {
                break;
            } else if e_min == NIL {
                e_min = e;
            }

            // E and E.Prev now share a local minima (left aligned if horizontal).
            // Compare their slopes to find which starts which bound ...
            let mut loc_min = LocalMinimum {
                y: self.e(e).bot.y,
                left_bound: NIL,
                right_bound: NIL,
            };
            let left_bound_is_forward = if self.e(e).dx < self.e(self.e(e).prev).dx {
                loc_min.left_bound = self.e(e).prev;
                loc_min.right_bound = e;
                false // Q.nextInLML = Q.prev
            } else {
                loc_min.left_bound = e;
                loc_min.right_bound = self.e(e).prev;
                true // Q.nextInLML = Q.next
            };

            let lb = loc_min.left_bound;
            let rb = loc_min.right_bound;
            if !closed {
                self.em(lb).wind_delta = 0;
            } else if self.e(lb).next == rb {
                self.em(lb).wind_delta = -1;
            } else {
                self.em(lb).wind_delta = 1;
            }
            let wd = self.e(lb).wind_delta;
            self.em(rb).wind_delta = -wd;

            e = self.process_bound(lb, left_bound_is_forward);
            if self.e(e).out_idx == SKIP {
                e = self.process_bound(e, left_bound_is_forward);
            }

            let mut e2 = self.process_bound(rb, !left_bound_is_forward);
            if self.e(e2).out_idx == SKIP {
                e2 = self.process_bound(e2, !left_bound_is_forward);
            }

            if self.e(lb).out_idx == SKIP {
                loc_min.left_bound = NIL;
            } else if self.e(rb).out_idx == SKIP {
                loc_min.right_bound = NIL;
            }
            self.minima_list.push(loc_min);
            if !left_bound_is_forward {
                e = e2;
            }
        }
        Ok(true)
    }

    /// `ClipperBase::AddPaths()`.
    pub fn add_paths(
        &mut self,
        ppg: &[Path],
        poly_typ: PolyType,
        closed: bool,
    ) -> Result<bool, ClipperException> {
        let mut result = false;
        for p in ppg {
            if self.add_path(p, poly_typ, closed)? {
                result = true;
            }
        }
        Ok(result)
    }

    /// `ClipperBase::Clear()`.
    pub fn clear(&mut self) {
        self.dispose_local_minima_list();
        self.edges.clear();
        self.use_full_range = false;
        self.has_open_paths = false;
    }

    fn reset(&mut self) {
        self.current_lm = 0;
        if self.minima_list.is_empty() {
            return; // ie nothing to process
        }
        cxxsort::sort(
            &mut self.minima_list,
            |a: &LocalMinimum, b: &LocalMinimum| b.y < a.y,
        );

        self.scanbeam = BinaryHeap::new(); // clears/resets priority_queue
                                           // reset all edges ...
        for i in 0..self.minima_list.len() {
            let lm = self.minima_list[i];
            self.scanbeam.push(lm.y);
            let e = lm.left_bound;
            if e != NIL {
                let ed = self.em(e);
                ed.curr = ed.bot;
                ed.side = ES_LEFT;
                ed.out_idx = UNASSIGNED;
            }
            let e = lm.right_bound;
            if e != NIL {
                let ed = self.em(e);
                ed.curr = ed.bot;
                ed.side = ES_RIGHT;
                ed.out_idx = UNASSIGNED;
            }
        }
        self.active_edges = NIL;
        self.current_lm = 0;
    }

    fn dispose_local_minima_list(&mut self) {
        self.minima_list.clear();
        self.current_lm = 0;
    }

    fn pop_local_minima(&mut self, y: i64) -> Option<LocalMinimum> {
        if self.current_lm >= self.minima_list.len() || self.minima_list[self.current_lm].y != y {
            return None;
        }
        let lm = self.minima_list[self.current_lm];
        self.current_lm += 1;
        Some(lm)
    }

    /// `ClipperBase::GetBounds()`.
    pub fn get_bounds(&self) -> IntRect {
        let mut result = IntRect::default();
        if self.minima_list.is_empty() {
            return result;
        }
        let lm0 = &self.minima_list[0];
        result.left = self.e(lm0.left_bound).bot.x;
        result.top = self.e(lm0.left_bound).bot.y;
        result.right = self.e(lm0.left_bound).bot.x;
        result.bottom = self.e(lm0.left_bound).bot.y;
        for lm in &self.minima_list {
            // todo - needs fixing for open paths
            result.bottom = result.bottom.max(self.e(lm.left_bound).bot.y);
            let mut e = lm.left_bound;
            loop {
                let bottom_e = e;
                while self.e(e).next_in_lml != NIL {
                    if self.e(e).bot.x < result.left {
                        result.left = self.e(e).bot.x;
                    }
                    if self.e(e).bot.x > result.right {
                        result.right = self.e(e).bot.x;
                    }
                    e = self.e(e).next_in_lml;
                }
                result.left = result.left.min(self.e(e).bot.x);
                result.right = result.right.max(self.e(e).bot.x);
                result.left = result.left.min(self.e(e).top.x);
                result.right = result.right.max(self.e(e).top.x);
                result.top = result.top.min(self.e(e).top.y);
                if bottom_e == lm.left_bound {
                    e = lm.right_bound;
                } else {
                    break;
                }
            }
        }
        result
    }

    #[inline]
    fn insert_scanbeam(&mut self, y: i64) {
        self.scanbeam.push(y);
    }

    fn pop_scanbeam(&mut self) -> Option<i64> {
        let y = self.scanbeam.pop()?;
        while self.scanbeam.peek() == Some(&y) {
            self.scanbeam.pop(); // Pop duplicates.
        }
        Some(y)
    }

    #[inline]
    fn local_minima_pending(&self) -> bool {
        self.current_lm < self.minima_list.len()
    }

    fn dispose_all_out_recs(&mut self) {
        self.out_recs.clear();
        self.out_pts.clear();
    }

    fn delete_from_ael(&mut self, e: usize) {
        let ael_prev = self.e(e).prev_in_ael;
        let ael_next = self.e(e).next_in_ael;
        if ael_prev == NIL && ael_next == NIL && e != self.active_edges {
            return; // already deleted
        }
        if ael_prev != NIL {
            self.em(ael_prev).next_in_ael = ael_next;
        } else {
            self.active_edges = ael_next;
        }
        if ael_next != NIL {
            self.em(ael_next).prev_in_ael = ael_prev;
        }
        self.em(e).next_in_ael = NIL;
        self.em(e).prev_in_ael = NIL;
    }

    fn create_out_rec(&mut self) -> usize {
        let id = self.out_recs.len();
        self.out_recs.push(OutRec {
            idx: id as i32,
            is_hole: false,
            is_open: false,
            first_left: NIL,
            poly_nd: NIL,
            pts: NIL,
            bottom_pt: NIL,
        });
        id
    }

    fn swap_positions_in_ael(&mut self, edge1: usize, edge2: usize) {
        // check that one or other edge hasn't already been removed from AEL ...
        if self.e(edge1).next_in_ael == self.e(edge1).prev_in_ael
            || self.e(edge2).next_in_ael == self.e(edge2).prev_in_ael
        {
            return;
        }

        if self.e(edge1).next_in_ael == edge2 {
            let next = self.e(edge2).next_in_ael;
            if next != NIL {
                self.em(next).prev_in_ael = edge1;
            }
            let prev = self.e(edge1).prev_in_ael;
            if prev != NIL {
                self.em(prev).next_in_ael = edge2;
            }
            self.em(edge2).prev_in_ael = prev;
            self.em(edge2).next_in_ael = edge1;
            self.em(edge1).prev_in_ael = edge2;
            self.em(edge1).next_in_ael = next;
        } else if self.e(edge2).next_in_ael == edge1 {
            let next = self.e(edge1).next_in_ael;
            if next != NIL {
                self.em(next).prev_in_ael = edge2;
            }
            let prev = self.e(edge2).prev_in_ael;
            if prev != NIL {
                self.em(prev).next_in_ael = edge1;
            }
            self.em(edge1).prev_in_ael = prev;
            self.em(edge1).next_in_ael = edge2;
            self.em(edge2).prev_in_ael = edge1;
            self.em(edge2).next_in_ael = next;
        } else {
            let next = self.e(edge1).next_in_ael;
            let prev = self.e(edge1).prev_in_ael;
            let n2 = self.e(edge2).next_in_ael;
            self.em(edge1).next_in_ael = n2;
            if n2 != NIL {
                self.em(n2).prev_in_ael = edge1;
            }
            let p2 = self.e(edge2).prev_in_ael;
            self.em(edge1).prev_in_ael = p2;
            if p2 != NIL {
                self.em(p2).next_in_ael = edge1;
            }
            self.em(edge2).next_in_ael = next;
            if next != NIL {
                self.em(next).prev_in_ael = edge2;
            }
            self.em(edge2).prev_in_ael = prev;
            if prev != NIL {
                self.em(prev).next_in_ael = edge2;
            }
        }

        if self.e(edge1).prev_in_ael == NIL {
            self.active_edges = edge1;
        } else if self.e(edge2).prev_in_ael == NIL {
            self.active_edges = edge2;
        }
    }

    fn update_edge_into_ael(&mut self, e: &mut usize) -> Exc {
        let ee = *e;
        let nl = self.e(ee).next_in_lml;
        if nl == NIL {
            return Err(ClipperException("UpdateEdgeIntoAEL: invalid call"));
        }
        let src = *self.e(ee);
        self.em(nl).out_idx = src.out_idx;
        let ael_prev = src.prev_in_ael;
        let ael_next = src.next_in_ael;
        if ael_prev != NIL {
            self.em(ael_prev).next_in_ael = nl;
        } else {
            self.active_edges = nl;
        }
        if ael_next != NIL {
            self.em(ael_next).prev_in_ael = nl;
        }
        {
            let n = self.em(nl);
            n.side = src.side;
            n.wind_delta = src.wind_delta;
            n.wind_cnt = src.wind_cnt;
            n.wind_cnt2 = src.wind_cnt2;
        }
        *e = nl;
        let n = self.em(nl);
        n.curr = n.bot;
        n.prev_in_ael = ael_prev;
        n.next_in_ael = ael_next;
        if !is_horizontal(n) {
            let ty = n.top.y;
            self.insert_scanbeam(ty);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Clipper
    // -----------------------------------------------------------------------

    /// `Clipper::Execute(ClipType, Paths&, PolyFillType, PolyFillType)`.
    /// `Err` is the exception the original throws (open paths present).
    pub fn execute(
        &mut self,
        clip_type: ClipType,
        solution: &mut Paths,
        subj_fill_type: PolyFillType,
        clip_fill_type: PolyFillType,
    ) -> Result<bool, ClipperException> {
        if self.execute_locked {
            return Ok(false);
        }
        if self.has_open_paths {
            return Err(ClipperException(
                "Error: PolyTree struct is needed for open path clipping.",
            ));
        }
        self.execute_locked = true;
        solution.clear();
        self.subj_fill_type = subj_fill_type;
        self.clip_fill_type = clip_fill_type;
        self.clip_type = clip_type;
        self.using_poly_tree = false;
        let succeeded = self.execute_internal();
        if succeeded {
            self.build_result(solution);
        }
        self.dispose_all_out_recs();
        self.execute_locked = false;
        Ok(succeeded)
    }

    /// `Clipper::Execute(ClipType, PolyTree&, PolyFillType, PolyFillType)`.
    pub fn execute_tree(
        &mut self,
        clip_type: ClipType,
        polytree: &mut PolyTree,
        subj_fill_type: PolyFillType,
        clip_fill_type: PolyFillType,
    ) -> bool {
        if self.execute_locked {
            return false;
        }
        self.execute_locked = true;
        self.subj_fill_type = subj_fill_type;
        self.clip_fill_type = clip_fill_type;
        self.clip_type = clip_type;
        self.using_poly_tree = true;
        let succeeded = self.execute_internal();
        if succeeded {
            self.build_result2(polytree);
        }
        self.dispose_all_out_recs();
        self.execute_locked = false;
        succeeded
    }

    fn fix_hole_linkage(&mut self, outrec: usize) {
        // skip OutRecs that (a) contain outermost polygons or
        // (b) already have the correct owner/child linkage ...
        let o = *self.or(outrec);
        if o.first_left == NIL
            || (o.is_hole != self.or(o.first_left).is_hole && self.or(o.first_left).pts != NIL)
        {
            return;
        }
        let mut orfl = o.first_left;
        while orfl != NIL && (self.or(orfl).is_hole == o.is_hole || self.or(orfl).pts == NIL) {
            orfl = self.or(orfl).first_left;
        }
        self.orm(outrec).first_left = orfl;
    }

    fn execute_internal(&mut self) -> bool {
        // Ok(None): `return false` from inside the try block (skips the
        // clean-up below). Ok(Some(b)) / Err: the try block's outcome.
        let r: Exc<Option<bool>> = (|| {
            self.reset();
            self.maxima.clear();
            self.sorted_edges = NIL;
            let Some(mut bot_y) = self.pop_scanbeam() else {
                return Ok(None);
            };
            self.insert_local_minima_into_ael(bot_y)?;
            // `PopScanbeam(topY) || LocalMinimaPending()`: a failed pop leaves
            // topY unchanged (never happens in practice: every local
            // minimum's Y is in the scanbeam).
            let mut top_y = bot_y;
            loop {
                match self.pop_scanbeam() {
                    Some(y) => top_y = y,
                    None if self.local_minima_pending() => {}
                    None => break,
                }
                self.process_horizontals()?;
                self.clear_ghost_joins();
                if !self.process_intersections(top_y)? {
                    return Ok(Some(false));
                }
                self.process_edges_at_top_of_scanbeam(top_y)?;
                bot_y = top_y;
                self.insert_local_minima_into_ael(bot_y)?;
            }
            Ok(Some(true))
        })();
        let succeeded = match r {
            Ok(None) => return false,
            Ok(Some(b)) => b,
            Err(_) => false,
        };

        if succeeded {
            // fix orientations ...
            for i in 0..self.out_recs.len() {
                let o = *self.or(i);
                if o.pts == NIL || o.is_open {
                    continue;
                }
                if (o.is_hole ^ self.reverse_output) == (self.area_op(o.pts) > 0.0) {
                    self.reverse_poly_pt_links(o.pts);
                }
            }

            if !self.joins.is_empty() {
                self.join_common_edges();
            }

            // unfortunately FixupOutPolygon() must be done after JoinCommonEdges()
            for i in 0..self.out_recs.len() {
                let o = *self.or(i);
                if o.pts == NIL {
                    continue;
                }
                if o.is_open {
                    self.fixup_out_polyline(i);
                } else {
                    self.fixup_out_polygon(i);
                }
            }

            if self.strict_simple {
                self.do_simple_polygons();
            }
        }

        self.clear_joins();
        self.clear_ghost_joins();
        succeeded
    }

    // ---- OutPt helpers ----------------------------------------------------

    /// `Area(const OutPt*)`.
    fn area_op(&self, op: usize) -> f64 {
        if op == NIL {
            return 0.0;
        }
        let start = op;
        let mut op = op;
        let mut a = 0.0f64;
        loop {
            let p = self.op(op);
            let pp = self.op(p.prev);
            a = fma(
                pp.pt.x.wrapping_add(p.pt.x) as f64,
                pp.pt.y.wrapping_sub(p.pt.y) as f64,
                a,
            );
            op = p.next;
            if op == start {
                break;
            }
        }
        a * 0.5
    }

    fn reverse_poly_pt_links(&mut self, pp: usize) {
        if pp == NIL {
            return;
        }
        let mut pp1 = pp;
        loop {
            let pp2 = self.op(pp1).next;
            let o = self.opm(pp1);
            o.next = o.prev;
            o.prev = pp2;
            pp1 = pp2;
            if pp1 == pp {
                break;
            }
        }
    }

    /// `PointInPolygon(const IntPoint&, OutPt*)`.
    fn point_in_polygon_op(&self, pt: IntPoint, op: usize) -> i32 {
        let mut result = 0;
        let start = op;
        let mut op = op;
        loop {
            let o = self.op(op);
            let n = self.op(o.next);
            if n.pt.y == pt.y
                && (n.pt.x == pt.x || (o.pt.y == pt.y && ((n.pt.x > pt.x) == (o.pt.x < pt.x))))
            {
                return -1;
            }
            if (o.pt.y < pt.y) != (n.pt.y < pt.y) {
                if o.pt.x >= pt.x {
                    if n.pt.x > pt.x {
                        result = 1 - result;
                    } else {
                        let d = pip_cross(o.pt, n.pt, pt);
                        if d == 0.0 {
                            return -1;
                        }
                        if (d > 0.0) == (n.pt.y > o.pt.y) {
                            result = 1 - result;
                        }
                    }
                } else if n.pt.x > pt.x {
                    let d = pip_cross(o.pt, n.pt, pt);
                    if d == 0.0 {
                        return -1;
                    }
                    if (d > 0.0) == (n.pt.y > o.pt.y) {
                        result = 1 - result;
                    }
                }
            }
            op = o.next;
            if start == op {
                break;
            }
        }
        result
    }

    fn poly2_contains_poly1(&self, out_pt1: usize, out_pt2: usize) -> bool {
        let mut op = out_pt1;
        loop {
            // nb: PointInPolygon returns 0 if false, +1 if true, -1 if pt on polygon
            let res = self.point_in_polygon_op(self.op(op).pt, out_pt2);
            if res >= 0 {
                return res > 0;
            }
            op = self.op(op).next;
            if op == out_pt1 {
                break;
            }
        }
        true
    }

    fn first_is_bottom_pt(&self, btm_pt1: usize, btm_pt2: usize) -> bool {
        let b1 = self.op(btm_pt1).pt;
        let b2 = self.op(btm_pt2).pt;
        let mut p = self.op(btm_pt1).prev;
        while self.op(p).pt == b1 && p != btm_pt1 {
            p = self.op(p).prev;
        }
        let dx1p = get_dx(b1, self.op(p).pt).abs();
        p = self.op(btm_pt1).next;
        while self.op(p).pt == b1 && p != btm_pt1 {
            p = self.op(p).next;
        }
        let dx1n = get_dx(b1, self.op(p).pt).abs();

        p = self.op(btm_pt2).prev;
        while self.op(p).pt == b2 && p != btm_pt2 {
            p = self.op(p).prev;
        }
        let dx2p = get_dx(b2, self.op(p).pt).abs();
        p = self.op(btm_pt2).next;
        while self.op(p).pt == b2 && p != btm_pt2 {
            p = self.op(p).next;
        }
        let dx2n = get_dx(b2, self.op(p).pt).abs();

        if cxx_max(dx1p, dx1n) == cxx_max(dx2p, dx2n) && cxx_min(dx1p, dx1n) == cxx_min(dx2p, dx2n)
        {
            self.area_op(btm_pt1) > 0.0 // if otherwise identical use orientation
        } else {
            (dx1p >= dx2p && dx1p >= dx2n) || (dx1n >= dx2p && dx1n >= dx2n)
        }
    }

    fn get_bottom_pt(&self, pp: usize) -> usize {
        let mut pp = pp;
        let mut dups = NIL;
        let mut p = self.op(pp).next;
        while p != pp {
            let (ppt, pppt) = (self.op(p).pt, self.op(pp).pt);
            if ppt.y > pppt.y {
                pp = p;
                dups = NIL;
            } else if ppt.y == pppt.y && ppt.x <= pppt.x {
                if ppt.x < pppt.x {
                    dups = NIL;
                    pp = p;
                } else if self.op(p).next != pp && self.op(p).prev != pp {
                    dups = p;
                }
            }
            p = self.op(p).next;
        }
        if dups != NIL {
            // there appears to be at least 2 vertices at BottomPt so ...
            while dups != p {
                if !self.first_is_bottom_pt(p, dups) {
                    pp = dups;
                }
                dups = self.op(dups).next;
                while self.op(dups).pt != self.op(pp).pt {
                    dups = self.op(dups).next;
                }
            }
        }
        pp
    }

    #[allow(clippy::if_same_then_else)]
    fn get_lowermost_rec(&mut self, out_rec1: usize, out_rec2: usize) -> usize {
        // work out which polygon fragment has the correct hole state ...
        if self.or(out_rec1).bottom_pt == NIL {
            let b = self.get_bottom_pt(self.or(out_rec1).pts);
            self.orm(out_rec1).bottom_pt = b;
        }
        if self.or(out_rec2).bottom_pt == NIL {
            let b = self.get_bottom_pt(self.or(out_rec2).pts);
            self.orm(out_rec2).bottom_pt = b;
        }
        let op1 = self.or(out_rec1).bottom_pt;
        let op2 = self.or(out_rec2).bottom_pt;
        let (p1, p2) = (self.op(op1).pt, self.op(op2).pt);
        if p1.y > p2.y {
            out_rec1
        } else if p1.y < p2.y {
            out_rec2
        } else if p1.x < p2.x {
            out_rec1
        } else if p1.x > p2.x {
            out_rec2
        } else if self.op(op1).next == op1 {
            out_rec2
        } else if self.op(op2).next == op2 {
            out_rec1
        } else if self.first_is_bottom_pt(op1, op2) {
            out_rec1
        } else {
            out_rec2
        }
    }

    fn out_rec1_right_of_out_rec2(&self, mut out_rec1: usize, out_rec2: usize) -> bool {
        loop {
            out_rec1 = self.or(out_rec1).first_left;
            if out_rec1 == out_rec2 {
                return true;
            }
            if out_rec1 == NIL {
                return false;
            }
        }
    }

    fn get_out_rec(&self, idx: i32) -> usize {
        let mut outrec = idx as usize;
        while outrec != self.or(outrec).idx as usize {
            outrec = self.or(outrec).idx as usize;
        }
        outrec
    }

    fn point_count(&self, pts: usize) -> i32 {
        if pts == NIL {
            return 0;
        }
        let mut result = 0;
        let mut p = pts;
        loop {
            result += 1;
            p = self.op(p).next;
            if p == pts {
                break;
            }
        }
        result
    }

    fn dup_out_pt(&mut self, out_pt: usize, insert_after: bool) -> usize {
        let o = *self.op(out_pt);
        if insert_after {
            let r = self.new_out_pt(o.idx, o.pt, o.next, out_pt);
            self.opm(o.next).prev = r;
            self.opm(out_pt).next = r;
            r
        } else {
            let r = self.new_out_pt(o.idx, o.pt, out_pt, o.prev);
            self.opm(o.prev).next = r;
            self.opm(out_pt).prev = r;
            r
        }
    }

    fn update_out_pt_idxs(&mut self, outrec: usize) {
        let o = *self.or(outrec);
        let mut op = o.pts;
        loop {
            self.opm(op).idx = o.idx;
            op = self.op(op).prev;
            if op == o.pts {
                break;
            }
        }
    }

    // ---- winding ----------------------------------------------------------

    fn set_winding_count(&mut self, edge: usize) {
        let mut e = self.e(edge).prev_in_ael;
        let ed = *self.e(edge);
        // find the edge of the same polytype that immediately preceeds 'edge' in AEL
        while e != NIL && (self.e(e).poly_typ != ed.poly_typ || self.e(e).wind_delta == 0) {
            e = self.e(e).prev_in_ael;
        }
        let (wind_cnt, wind_cnt2);
        if e == NIL {
            if ed.wind_delta == 0 {
                let pft = if ed.poly_typ == PolyType::Subject {
                    self.subj_fill_type
                } else {
                    self.clip_fill_type
                };
                wind_cnt = if pft == PolyFillType::Negative { -1 } else { 1 };
            } else {
                wind_cnt = ed.wind_delta;
            }
            wind_cnt2 = 0;
            e = self.active_edges; // ie get ready to calc WindCnt2
        } else if ed.wind_delta == 0 && self.clip_type != ClipType::Union {
            wind_cnt = 1;
            wind_cnt2 = self.e(e).wind_cnt2;
            e = self.e(e).next_in_ael; // ie get ready to calc WindCnt2
        } else if self.is_even_odd_fill_type(&ed) {
            // EvenOdd filling ...
            if ed.wind_delta == 0 {
                // are we inside a subj polygon ...
                let mut inside = true;
                let mut e2 = self.e(e).prev_in_ael;
                while e2 != NIL {
                    if self.e(e2).poly_typ == self.e(e).poly_typ && self.e(e2).wind_delta != 0 {
                        inside = !inside;
                    }
                    e2 = self.e(e2).prev_in_ael;
                }
                wind_cnt = if inside { 0 } else { 1 };
            } else {
                wind_cnt = ed.wind_delta;
            }
            wind_cnt2 = self.e(e).wind_cnt2;
            e = self.e(e).next_in_ael; // ie get ready to calc WindCnt2
        } else {
            // nonZero, Positive or Negative filling ...
            let pe = *self.e(e);
            if pe.wind_cnt.wrapping_mul(pe.wind_delta) < 0 {
                // prev edge is 'decreasing' WindCount (WC) toward zero
                // so we're outside the previous polygon ...
                if pe.wind_cnt.wrapping_abs() > 1 {
                    // outside prev poly but still inside another.
                    // when reversing direction of prev poly use the same WC
                    if pe.wind_delta.wrapping_mul(ed.wind_delta) < 0 {
                        wind_cnt = pe.wind_cnt;
                    } else {
                        // otherwise continue to 'decrease' WC ...
                        wind_cnt = pe.wind_cnt.wrapping_add(ed.wind_delta);
                    }
                } else {
                    // now outside all polys of same polytype so set own WC ...
                    wind_cnt = if ed.wind_delta == 0 { 1 } else { ed.wind_delta };
                }
            } else {
                // prev edge is 'increasing' WindCount (WC) away from zero
                // so we're inside the previous polygon ...
                if ed.wind_delta == 0 {
                    wind_cnt = if pe.wind_cnt < 0 {
                        pe.wind_cnt.wrapping_sub(1)
                    } else {
                        pe.wind_cnt.wrapping_add(1)
                    };
                } else if pe.wind_delta.wrapping_mul(ed.wind_delta) < 0 {
                    // if wind direction is reversing prev then use same WC
                    wind_cnt = pe.wind_cnt;
                } else {
                    // otherwise add to WC ...
                    wind_cnt = pe.wind_cnt.wrapping_add(ed.wind_delta);
                }
            }
            wind_cnt2 = pe.wind_cnt2;
            e = pe.next_in_ael; // ie get ready to calc WindCnt2
        }
        self.em(edge).wind_cnt = wind_cnt;
        let mut wc2 = wind_cnt2;

        // update WindCnt2 ...
        if self.is_even_odd_alt_fill_type(&ed) {
            // EvenOdd filling ...
            while e != edge {
                if self.e(e).wind_delta != 0 {
                    wc2 = if wc2 == 0 { 1 } else { 0 };
                }
                e = self.e(e).next_in_ael;
            }
        } else {
            // nonZero, Positive or Negative filling ...
            while e != edge {
                wc2 = wc2.wrapping_add(self.e(e).wind_delta);
                e = self.e(e).next_in_ael;
            }
        }
        self.em(edge).wind_cnt2 = wc2;
    }

    #[inline]
    fn is_even_odd_fill_type(&self, edge: &TEdge) -> bool {
        if edge.poly_typ == PolyType::Subject {
            self.subj_fill_type == PolyFillType::EvenOdd
        } else {
            self.clip_fill_type == PolyFillType::EvenOdd
        }
    }

    #[inline]
    fn is_even_odd_alt_fill_type(&self, edge: &TEdge) -> bool {
        if edge.poly_typ == PolyType::Subject {
            self.clip_fill_type == PolyFillType::EvenOdd
        } else {
            self.subj_fill_type == PolyFillType::EvenOdd
        }
    }

    fn is_contributing(&self, edge: usize) -> bool {
        let edge = self.e(edge);
        let (pft, pft2) = if edge.poly_typ == PolyType::Subject {
            (self.subj_fill_type, self.clip_fill_type)
        } else {
            (self.clip_fill_type, self.subj_fill_type)
        };

        match pft {
            PolyFillType::EvenOdd => {
                // return false if a subj line has been flagged as inside a subj polygon
                if edge.wind_delta == 0 && edge.wind_cnt != 1 {
                    return false;
                }
            }
            PolyFillType::NonZero => {
                if edge.wind_cnt.wrapping_abs() != 1 {
                    return false;
                }
            }
            PolyFillType::Positive => {
                if edge.wind_cnt != 1 {
                    return false;
                }
            }
            PolyFillType::Negative => {
                if edge.wind_cnt != -1 {
                    return false;
                }
            }
        }

        let w2 = edge.wind_cnt2;
        let ne_zero = |pft2: PolyFillType| match pft2 {
            PolyFillType::EvenOdd | PolyFillType::NonZero => w2 != 0,
            PolyFillType::Positive => w2 > 0,
            PolyFillType::Negative => w2 < 0,
        };
        let eq_zero = |pft2: PolyFillType| match pft2 {
            PolyFillType::EvenOdd | PolyFillType::NonZero => w2 == 0,
            PolyFillType::Positive => w2 <= 0,
            PolyFillType::Negative => w2 >= 0,
        };
        match self.clip_type {
            ClipType::Intersection => ne_zero(pft2),
            ClipType::Union => eq_zero(pft2),
            ClipType::Difference => {
                if edge.poly_typ == PolyType::Subject {
                    eq_zero(pft2)
                } else {
                    ne_zero(pft2)
                }
            }
            ClipType::Xor => {
                if edge.wind_delta == 0 {
                    // XOr always contributing unless open
                    eq_zero(pft2)
                } else {
                    true
                }
            }
        }
    }

    // ---- output polygons ---------------------------------------------------

    fn add_local_min_poly(&mut self, e1: usize, e2: usize, pt: IntPoint) -> usize {
        let result;
        let e;
        let prev_e;
        if self.horz(e2) || self.e(e1).dx > self.e(e2).dx {
            result = self.add_out_pt(e1, pt);
            let oi = self.e(e1).out_idx;
            self.em(e2).out_idx = oi;
            self.em(e1).side = ES_LEFT;
            self.em(e2).side = ES_RIGHT;
            e = e1;
            if self.e(e).prev_in_ael == e2 {
                prev_e = self.e(e2).prev_in_ael;
            } else {
                prev_e = self.e(e).prev_in_ael;
            }
        } else {
            result = self.add_out_pt(e2, pt);
            let oi = self.e(e2).out_idx;
            self.em(e1).out_idx = oi;
            self.em(e1).side = ES_RIGHT;
            self.em(e2).side = ES_LEFT;
            e = e2;
            if self.e(e).prev_in_ael == e1 {
                prev_e = self.e(e1).prev_in_ael;
            } else {
                prev_e = self.e(e).prev_in_ael;
            }
        }

        if prev_e != NIL
            && self.e(prev_e).out_idx >= 0
            && self.e(prev_e).top.y < pt.y
            && self.e(e).top.y < pt.y
        {
            let x_prev = top_x(self.e(prev_e), pt.y);
            let x_e = top_x(self.e(e), pt.y);
            if x_prev == x_e
                && self.e(e).wind_delta != 0
                && self.e(prev_e).wind_delta != 0
                && slopes_equal4(
                    IntPoint::new(x_prev, pt.y),
                    self.e(prev_e).top,
                    IntPoint::new(x_e, pt.y),
                    self.e(e).top,
                    self.use_full_range,
                )
            {
                let out_pt = self.add_out_pt(prev_e, pt);
                let et = self.e(e).top;
                self.add_join(result, out_pt, et);
            }
        }
        result
    }

    fn add_local_max_poly(&mut self, e1: usize, e2: usize, pt: IntPoint) {
        self.add_out_pt(e1, pt);
        if self.e(e2).wind_delta == 0 {
            self.add_out_pt(e2, pt);
        }
        let (i1, i2) = (self.e(e1).out_idx, self.e(e2).out_idx);
        if i1 == i2 {
            self.em(e1).out_idx = UNASSIGNED;
            self.em(e2).out_idx = UNASSIGNED;
        } else if i1 < i2 {
            self.append_polygon(e1, e2);
        } else {
            self.append_polygon(e2, e1);
        }
    }

    fn add_out_pt(&mut self, e: usize, pt: IntPoint) -> usize {
        if self.e(e).out_idx < 0 {
            let out_rec = self.create_out_rec();
            let is_open = self.e(e).wind_delta == 0;
            self.orm(out_rec).is_open = is_open;
            let idx = self.or(out_rec).idx;
            let new_op = self.new_out_pt(idx, pt, NIL, NIL);
            self.orm(out_rec).pts = new_op;
            self.opm(new_op).next = new_op;
            self.opm(new_op).prev = new_op;
            if !is_open {
                self.set_hole_state(e, out_rec);
            }
            self.em(e).out_idx = idx;
            new_op
        } else {
            let out_rec = self.e(e).out_idx as usize;
            // OutRec.Pts is the 'Left-most' point & OutRec.Pts.Prev is the 'Right-most'
            let op = self.or(out_rec).pts;
            let to_front = self.e(e).side == ES_LEFT;
            if to_front && pt == self.op(op).pt {
                return op;
            } else if !to_front && pt == self.op(self.op(op).prev).pt {
                return self.op(op).prev;
            }
            let idx = self.or(out_rec).idx;
            let op_prev = self.op(op).prev;
            let new_op = self.new_out_pt(idx, pt, op, op_prev);
            self.opm(op_prev).next = new_op;
            self.opm(op).prev = new_op;
            if to_front {
                self.orm(out_rec).pts = new_op;
            }
            new_op
        }
    }

    fn get_last_out_pt(&self, e: usize) -> usize {
        let out_rec = self.or(self.e(e).out_idx as usize);
        if self.e(e).side == ES_LEFT {
            out_rec.pts
        } else {
            self.op(out_rec.pts).prev
        }
    }

    fn set_hole_state(&mut self, e: usize, outrec: usize) {
        let mut e2 = self.e(e).prev_in_ael;
        let mut e_tmp = NIL;
        while e2 != NIL {
            if self.e(e2).out_idx >= 0 && self.e(e2).wind_delta != 0 {
                if e_tmp == NIL {
                    e_tmp = e2;
                } else if self.e(e_tmp).out_idx == self.e(e2).out_idx {
                    e_tmp = NIL;
                }
            }
            e2 = self.e(e2).prev_in_ael;
        }
        if e_tmp == NIL {
            self.orm(outrec).first_left = NIL;
            self.orm(outrec).is_hole = false;
        } else {
            let fl = self.e(e_tmp).out_idx as usize;
            self.orm(outrec).first_left = fl;
            let h = !self.or(fl).is_hole;
            self.orm(outrec).is_hole = h;
        }
    }

    fn append_polygon(&mut self, e1: usize, e2: usize) {
        // get the start and ends of both output polygons ...
        let out_rec1 = self.e(e1).out_idx as usize;
        let out_rec2 = self.e(e2).out_idx as usize;

        let hole_state_rec = if self.out_rec1_right_of_out_rec2(out_rec1, out_rec2) {
            out_rec2
        } else if self.out_rec1_right_of_out_rec2(out_rec2, out_rec1) {
            out_rec1
        } else {
            self.get_lowermost_rec(out_rec1, out_rec2)
        };

        // get the start and ends of both output polygons and
        // join e2 poly onto e1 poly and delete pointers to e2 ...
        let p1_lft = self.or(out_rec1).pts;
        let p1_rt = self.op(p1_lft).prev;
        let p2_lft = self.or(out_rec2).pts;
        let p2_rt = self.op(p2_lft).prev;

        // join e2 poly onto e1 poly and delete pointers to e2 ...
        if self.e(e1).side == ES_LEFT {
            if self.e(e2).side == ES_LEFT {
                // z y x a b c
                self.reverse_poly_pt_links(p2_lft);
                self.opm(p2_lft).next = p1_lft;
                self.opm(p1_lft).prev = p2_lft;
                self.opm(p1_rt).next = p2_rt;
                self.opm(p2_rt).prev = p1_rt;
                self.orm(out_rec1).pts = p2_rt;
            } else {
                // x y z a b c
                self.opm(p2_rt).next = p1_lft;
                self.opm(p1_lft).prev = p2_rt;
                self.opm(p2_lft).prev = p1_rt;
                self.opm(p1_rt).next = p2_lft;
                self.orm(out_rec1).pts = p2_lft;
            }
        } else if self.e(e2).side == ES_RIGHT {
            // a b c z y x
            self.reverse_poly_pt_links(p2_lft);
            self.opm(p1_rt).next = p2_rt;
            self.opm(p2_rt).prev = p1_rt;
            self.opm(p2_lft).next = p1_lft;
            self.opm(p1_lft).prev = p2_lft;
        } else {
            // a b c x y z
            self.opm(p1_rt).next = p2_lft;
            self.opm(p2_lft).prev = p1_rt;
            self.opm(p1_lft).prev = p2_rt;
            self.opm(p2_rt).next = p1_lft;
        }

        self.orm(out_rec1).bottom_pt = NIL;
        if hole_state_rec == out_rec2 {
            if self.or(out_rec2).first_left != out_rec1 {
                let fl = self.or(out_rec2).first_left;
                self.orm(out_rec1).first_left = fl;
            }
            let h = self.or(out_rec2).is_hole;
            self.orm(out_rec1).is_hole = h;
        }
        {
            let o2 = self.orm(out_rec2);
            o2.pts = NIL;
            o2.bottom_pt = NIL;
            o2.first_left = out_rec1;
        }

        let ok_idx = self.e(e1).out_idx;
        let obsolete_idx = self.e(e2).out_idx;

        self.em(e1).out_idx = UNASSIGNED; // nb: safe because we only get here via AddLocalMaxPoly
        self.em(e2).out_idx = UNASSIGNED;

        let mut e = self.active_edges;
        while e != NIL {
            if self.e(e).out_idx == obsolete_idx {
                self.em(e).out_idx = ok_idx;
                let s = self.e(e1).side;
                self.em(e).side = s;
                break;
            }
            e = self.e(e).next_in_ael;
        }

        let i1 = self.or(out_rec1).idx;
        self.orm(out_rec2).idx = i1;
    }

    // ---- SEL / joins -------------------------------------------------------

    fn add_edge_to_sel(&mut self, edge: usize) {
        // SEL pointers in PEdge are reused to build a list of horizontal edges.
        // However, we don't need to worry about order with horizontal edge processing.
        if self.sorted_edges == NIL {
            self.sorted_edges = edge;
            self.em(edge).prev_in_sel = NIL;
            self.em(edge).next_in_sel = NIL;
        } else {
            let se = self.sorted_edges;
            self.em(edge).next_in_sel = se;
            self.em(edge).prev_in_sel = NIL;
            self.em(se).prev_in_sel = edge;
            self.sorted_edges = edge;
        }
    }

    fn pop_edge_from_sel(&mut self) -> Option<usize> {
        if self.sorted_edges == NIL {
            return None;
        }
        let edge = self.sorted_edges;
        self.delete_from_sel(edge);
        Some(edge)
    }

    fn copy_ael_to_sel(&mut self) {
        let mut e = self.active_edges;
        self.sorted_edges = e;
        while e != NIL {
            let ed = self.em(e);
            ed.prev_in_sel = ed.prev_in_ael;
            ed.next_in_sel = ed.next_in_ael;
            e = ed.next_in_ael;
        }
    }

    fn add_join(&mut self, op1: usize, op2: usize, off_pt: IntPoint) {
        self.joins.push(Join {
            out_pt1: op1,
            out_pt2: op2,
            off_pt,
        });
    }

    fn clear_joins(&mut self) {
        self.joins.clear();
    }

    fn clear_ghost_joins(&mut self) {
        self.ghost_joins.clear();
    }

    fn add_ghost_join(&mut self, op: usize, off_pt: IntPoint) {
        self.ghost_joins.push(Join {
            out_pt1: op,
            out_pt2: NIL,
            off_pt,
        });
    }

    fn delete_from_sel(&mut self, e: usize) {
        let sel_prev = self.e(e).prev_in_sel;
        let sel_next = self.e(e).next_in_sel;
        if sel_prev == NIL && sel_next == NIL && e != self.sorted_edges {
            return; // already deleted
        }
        if sel_prev != NIL {
            self.em(sel_prev).next_in_sel = sel_next;
        } else {
            self.sorted_edges = sel_next;
        }
        if sel_next != NIL {
            self.em(sel_next).prev_in_sel = sel_prev;
        }
        self.em(e).next_in_sel = NIL;
        self.em(e).prev_in_sel = NIL;
    }

    fn swap_positions_in_sel(&mut self, edge1: usize, edge2: usize) {
        if self.e(edge1).next_in_sel == NIL && self.e(edge1).prev_in_sel == NIL {
            return;
        }
        if self.e(edge2).next_in_sel == NIL && self.e(edge2).prev_in_sel == NIL {
            return;
        }

        if self.e(edge1).next_in_sel == edge2 {
            let next = self.e(edge2).next_in_sel;
            if next != NIL {
                self.em(next).prev_in_sel = edge1;
            }
            let prev = self.e(edge1).prev_in_sel;
            if prev != NIL {
                self.em(prev).next_in_sel = edge2;
            }
            self.em(edge2).prev_in_sel = prev;
            self.em(edge2).next_in_sel = edge1;
            self.em(edge1).prev_in_sel = edge2;
            self.em(edge1).next_in_sel = next;
        } else if self.e(edge2).next_in_sel == edge1 {
            let next = self.e(edge1).next_in_sel;
            if next != NIL {
                self.em(next).prev_in_sel = edge2;
            }
            let prev = self.e(edge2).prev_in_sel;
            if prev != NIL {
                self.em(prev).next_in_sel = edge1;
            }
            self.em(edge1).prev_in_sel = prev;
            self.em(edge1).next_in_sel = edge2;
            self.em(edge2).prev_in_sel = edge1;
            self.em(edge2).next_in_sel = next;
        } else {
            let next = self.e(edge1).next_in_sel;
            let prev = self.e(edge1).prev_in_sel;
            let n2 = self.e(edge2).next_in_sel;
            self.em(edge1).next_in_sel = n2;
            if n2 != NIL {
                self.em(n2).prev_in_sel = edge1;
            }
            let p2 = self.e(edge2).prev_in_sel;
            self.em(edge1).prev_in_sel = p2;
            if p2 != NIL {
                self.em(p2).next_in_sel = edge1;
            }
            self.em(edge2).next_in_sel = next;
            if next != NIL {
                self.em(next).prev_in_sel = edge2;
            }
            self.em(edge2).prev_in_sel = prev;
            if prev != NIL {
                self.em(prev).next_in_sel = edge2;
            }
        }

        if self.e(edge1).prev_in_sel == NIL {
            self.sorted_edges = edge1;
        } else if self.e(edge2).prev_in_sel == NIL {
            self.sorted_edges = edge2;
        }
    }

    // ---- the sweep ---------------------------------------------------------

    fn insert_local_minima_into_ael(&mut self, bot_y: i64) -> Exc {
        while let Some(lm) = self.pop_local_minima(bot_y) {
            let lb = lm.left_bound;
            let rb = lm.right_bound;

            let mut op1 = NIL;
            if lb == NIL {
                // nb: don't insert LB into either AEL or SEL
                self.insert_edge_into_ael(rb, NIL);
                self.set_winding_count(rb);
                if self.is_contributing(rb) {
                    let b = self.e(rb).bot;
                    op1 = self.add_out_pt(rb, b);
                }
            } else if rb == NIL {
                self.insert_edge_into_ael(lb, NIL);
                self.set_winding_count(lb);
                if self.is_contributing(lb) {
                    let b = self.e(lb).bot;
                    op1 = self.add_out_pt(lb, b);
                }
                let ty = self.e(lb).top.y;
                self.insert_scanbeam(ty);
            } else {
                self.insert_edge_into_ael(lb, NIL);
                self.insert_edge_into_ael(rb, lb);
                self.set_winding_count(lb);
                let (wc, wc2) = (self.e(lb).wind_cnt, self.e(lb).wind_cnt2);
                self.em(rb).wind_cnt = wc;
                self.em(rb).wind_cnt2 = wc2;
                if self.is_contributing(lb) {
                    let b = self.e(lb).bot;
                    op1 = self.add_local_min_poly(lb, rb, b);
                }
                let ty = self.e(lb).top.y;
                self.insert_scanbeam(ty);
            }

            if rb != NIL {
                if self.horz(rb) {
                    self.add_edge_to_sel(rb);
                    let nl = self.e(rb).next_in_lml;
                    if nl != NIL {
                        let ty = self.e(nl).top.y;
                        self.insert_scanbeam(ty);
                    }
                } else {
                    let ty = self.e(rb).top.y;
                    self.insert_scanbeam(ty);
                }
            }

            if lb == NIL || rb == NIL {
                continue;
            }

            // if any output polygons share an edge, they'll need joining later ...
            if op1 != NIL
                && self.horz(rb)
                && !self.ghost_joins.is_empty()
                && self.e(rb).wind_delta != 0
            {
                for i in 0..self.ghost_joins.len() {
                    let jr = self.ghost_joins[i];
                    // if the horizontal Rb and a 'ghost' horizontal overlap, then convert
                    // the 'ghost' join to a real join ready for later ...
                    if horz_segments_overlap(
                        self.op(jr.out_pt1).pt.x,
                        jr.off_pt.x,
                        self.e(rb).bot.x,
                        self.e(rb).top.x,
                    ) {
                        self.add_join(jr.out_pt1, op1, jr.off_pt);
                    }
                }
            }

            let lbp = self.e(lb).prev_in_ael;
            if self.e(lb).out_idx >= 0
                && lbp != NIL
                && self.e(lbp).curr.x == self.e(lb).bot.x
                && self.e(lbp).out_idx >= 0
                && slopes_equal4(
                    self.e(lbp).bot,
                    self.e(lbp).top,
                    self.e(lb).curr,
                    self.e(lb).top,
                    self.use_full_range,
                )
                && self.e(lb).wind_delta != 0
                && self.e(lbp).wind_delta != 0
            {
                let b = self.e(lb).bot;
                let op2 = self.add_out_pt(lbp, b);
                let t = self.e(lb).top;
                self.add_join(op1, op2, t);
            }

            if self.e(lb).next_in_ael != rb {
                let rbp = self.e(rb).prev_in_ael;
                if self.e(rb).out_idx >= 0
                    && self.e(rbp).out_idx >= 0
                    && slopes_equal4(
                        self.e(rbp).curr,
                        self.e(rbp).top,
                        self.e(rb).curr,
                        self.e(rb).top,
                        self.use_full_range,
                    )
                    && self.e(rb).wind_delta != 0
                    && self.e(rbp).wind_delta != 0
                {
                    let b = self.e(rb).bot;
                    let op2 = self.add_out_pt(rbp, b);
                    let t = self.e(rb).top;
                    self.add_join(op1, op2, t);
                }

                let mut e = self.e(lb).next_in_ael;
                if e != NIL {
                    while e != rb {
                        // nb: For calculating winding counts etc, IntersectEdges() assumes
                        // that param1 will be to the Right of param2 ABOVE the intersection ...
                        let c = self.e(lb).curr;
                        self.intersect_edges(rb, e, c); // order important here
                        e = self.e(e).next_in_ael;
                    }
                }
            }
        }
        Ok(())
    }

    fn intersect_edges(&mut self, e1: usize, e2: usize, pt: IntPoint) {
        let e1_contributing = self.e(e1).out_idx >= 0;
        let e2_contributing = self.e(e2).out_idx >= 0;

        // if either edge is on an OPEN path ...
        if self.e(e1).wind_delta == 0 || self.e(e2).wind_delta == 0 {
            let (d1, d2) = (*self.e(e1), *self.e(e2));
            // ignore subject-subject open path intersections UNLESS they
            // are both open paths, AND they are both 'contributing maximas' ...
            if d1.wind_delta == 0 && d2.wind_delta == 0 {
                return;
            }
            // if intersecting a subj line with a subj poly ...
            else if d1.poly_typ == d2.poly_typ
                && d1.wind_delta != d2.wind_delta
                && self.clip_type == ClipType::Union
            {
                if d1.wind_delta == 0 {
                    if e2_contributing {
                        self.add_out_pt(e1, pt);
                        if e1_contributing {
                            self.em(e1).out_idx = UNASSIGNED;
                        }
                    }
                } else if e1_contributing {
                    self.add_out_pt(e2, pt);
                    if e2_contributing {
                        self.em(e2).out_idx = UNASSIGNED;
                    }
                }
            } else if d1.poly_typ != d2.poly_typ {
                // toggle subj open path OutIdx on/off when Abs(clip.WndCnt) == 1 ...
                if d1.wind_delta == 0
                    && d2.wind_cnt.wrapping_abs() == 1
                    && (self.clip_type != ClipType::Union || d2.wind_cnt2 == 0)
                {
                    self.add_out_pt(e1, pt);
                    if e1_contributing {
                        self.em(e1).out_idx = UNASSIGNED;
                    }
                } else if d2.wind_delta == 0
                    && d1.wind_cnt.wrapping_abs() == 1
                    && (self.clip_type != ClipType::Union || d1.wind_cnt2 == 0)
                {
                    self.add_out_pt(e2, pt);
                    if e2_contributing {
                        self.em(e2).out_idx = UNASSIGNED;
                    }
                }
            }
            return;
        }

        // update winding counts...
        // assumes that e1 will be to the Right of e2 ABOVE the intersection
        if self.e(e1).poly_typ == self.e(e2).poly_typ {
            if self.is_even_odd_fill_type(self.e(e1)) {
                let old_e1_wind_cnt = self.e(e1).wind_cnt;
                let w2 = self.e(e2).wind_cnt;
                self.em(e1).wind_cnt = w2;
                self.em(e2).wind_cnt = old_e1_wind_cnt;
            } else {
                let d2 = self.e(e2).wind_delta;
                let w1 = self.e(e1).wind_cnt;
                if w1.wrapping_add(d2) == 0 {
                    self.em(e1).wind_cnt = w1.wrapping_neg();
                } else {
                    self.em(e1).wind_cnt = w1.wrapping_add(d2);
                }
                let d1 = self.e(e1).wind_delta;
                let w2 = self.e(e2).wind_cnt;
                if w2.wrapping_sub(d1) == 0 {
                    self.em(e2).wind_cnt = w2.wrapping_neg();
                } else {
                    self.em(e2).wind_cnt = w2.wrapping_sub(d1);
                }
            }
        } else {
            if !self.is_even_odd_fill_type(self.e(e2)) {
                let d2 = self.e(e2).wind_delta;
                let w = self.e(e1).wind_cnt2.wrapping_add(d2);
                self.em(e1).wind_cnt2 = w;
            } else {
                let w = if self.e(e1).wind_cnt2 == 0 { 1 } else { 0 };
                self.em(e1).wind_cnt2 = w;
            }
            if !self.is_even_odd_fill_type(self.e(e1)) {
                let d1 = self.e(e1).wind_delta;
                let w = self.e(e2).wind_cnt2.wrapping_sub(d1);
                self.em(e2).wind_cnt2 = w;
            } else {
                let w = if self.e(e2).wind_cnt2 == 0 { 1 } else { 0 };
                self.em(e2).wind_cnt2 = w;
            }
        }

        let (e1_fill_type, e1_fill_type2) = if self.e(e1).poly_typ == PolyType::Subject {
            (self.subj_fill_type, self.clip_fill_type)
        } else {
            (self.clip_fill_type, self.subj_fill_type)
        };
        let (e2_fill_type, e2_fill_type2) = if self.e(e2).poly_typ == PolyType::Subject {
            (self.subj_fill_type, self.clip_fill_type)
        } else {
            (self.clip_fill_type, self.subj_fill_type)
        };

        let wc = |pft: PolyFillType, w: i32| -> i64 {
            match pft {
                PolyFillType::Positive => w as i64,
                PolyFillType::Negative => -(w as i64),
                _ => (w as i64).abs(),
            }
        };
        let e1_wc = wc(e1_fill_type, self.e(e1).wind_cnt);
        let e2_wc = wc(e2_fill_type, self.e(e2).wind_cnt);

        if e1_contributing && e2_contributing {
            if (e1_wc != 0 && e1_wc != 1)
                || (e2_wc != 0 && e2_wc != 1)
                || (self.e(e1).poly_typ != self.e(e2).poly_typ && self.clip_type != ClipType::Xor)
            {
                self.add_local_max_poly(e1, e2, pt);
            } else {
                self.add_out_pt(e1, pt);
                self.add_out_pt(e2, pt);
                self.swap_sides(e1, e2);
                self.swap_poly_indexes(e1, e2);
            }
        } else if e1_contributing {
            if e2_wc == 0 || e2_wc == 1 {
                self.add_out_pt(e1, pt);
                self.swap_sides(e1, e2);
                self.swap_poly_indexes(e1, e2);
            }
        } else if e2_contributing {
            if e1_wc == 0 || e1_wc == 1 {
                self.add_out_pt(e2, pt);
                self.swap_sides(e1, e2);
                self.swap_poly_indexes(e1, e2);
            }
        } else if (e1_wc == 0 || e1_wc == 1) && (e2_wc == 0 || e2_wc == 1) {
            // neither edge is currently contributing ...
            let e1_wc2 = wc(e1_fill_type2, self.e(e1).wind_cnt2);
            let e2_wc2 = wc(e2_fill_type2, self.e(e2).wind_cnt2);

            if self.e(e1).poly_typ != self.e(e2).poly_typ {
                self.add_local_min_poly(e1, e2, pt);
            } else if e1_wc == 1 && e2_wc == 1 {
                match self.clip_type {
                    ClipType::Intersection => {
                        if e1_wc2 > 0 && e2_wc2 > 0 {
                            self.add_local_min_poly(e1, e2, pt);
                        }
                    }
                    ClipType::Union => {
                        if e1_wc2 <= 0 && e2_wc2 <= 0 {
                            self.add_local_min_poly(e1, e2, pt);
                        }
                    }
                    ClipType::Difference => {
                        let pt1 = self.e(e1).poly_typ;
                        if (pt1 == PolyType::Clip && e1_wc2 > 0 && e2_wc2 > 0)
                            || (pt1 == PolyType::Subject && e1_wc2 <= 0 && e2_wc2 <= 0)
                        {
                            self.add_local_min_poly(e1, e2, pt);
                        }
                    }
                    ClipType::Xor => {
                        self.add_local_min_poly(e1, e2, pt);
                    }
                }
            } else {
                self.swap_sides(e1, e2);
            }
        }
    }

    #[inline]
    fn swap_sides(&mut self, e1: usize, e2: usize) {
        let s = self.e(e1).side;
        let s2 = self.e(e2).side;
        self.em(e1).side = s2;
        self.em(e2).side = s;
    }

    #[inline]
    fn swap_poly_indexes(&mut self, e1: usize, e2: usize) {
        let i = self.e(e1).out_idx;
        let i2 = self.e(e2).out_idx;
        self.em(e1).out_idx = i2;
        self.em(e2).out_idx = i;
    }

    fn process_horizontals(&mut self) -> Exc {
        while let Some(horz_edge) = self.pop_edge_from_sel() {
            self.process_horizontal(horz_edge)?;
        }
        Ok(())
    }

    #[inline]
    fn is_maxima(&self, e: usize, y: i64) -> bool {
        e != NIL && self.e(e).top.y == y && self.e(e).next_in_lml == NIL
    }

    #[inline]
    fn is_intermediate(&self, e: usize, y: i64) -> bool {
        self.e(e).top.y == y && self.e(e).next_in_lml != NIL
    }

    fn get_maxima_pair(&self, e: usize) -> usize {
        let ed = self.e(e);
        if self.e(ed.next).top == ed.top && self.e(ed.next).next_in_lml == NIL {
            ed.next
        } else if self.e(ed.prev).top == ed.top && self.e(ed.prev).next_in_lml == NIL {
            ed.prev
        } else {
            NIL
        }
    }

    fn get_maxima_pair_ex(&self, e: usize) -> usize {
        // as GetMaximaPair() but returns 0 if MaxPair isn't in AEL (unless it's horizontal)
        let result = self.get_maxima_pair(e);
        if result != NIL {
            let r = self.e(result);
            if r.out_idx == SKIP || (r.next_in_ael == r.prev_in_ael && !is_horizontal(r)) {
                return NIL;
            }
        }
        result
    }

    #[inline]
    fn get_next_in_ael(&self, e: usize, dir: Direction) -> usize {
        if dir == Direction::LeftToRight {
            self.e(e).next_in_ael
        } else {
            self.e(e).prev_in_ael
        }
    }

    fn get_horz_direction(&self, e: usize) -> (Direction, i64, i64) {
        let h = self.e(e);
        if h.bot.x < h.top.x {
            (Direction::LeftToRight, h.bot.x, h.top.x)
        } else {
            (Direction::RightToLeft, h.top.x, h.bot.x)
        }
    }

    fn process_horizontal(&mut self, mut horz_edge: usize) -> Exc {
        let is_open = self.e(horz_edge).wind_delta == 0;

        let (mut dir, mut horz_left, mut horz_right) = self.get_horz_direction(horz_edge);

        let mut e_last_horz = horz_edge;
        let mut e_max_pair = NIL;
        while self.e(e_last_horz).next_in_lml != NIL && self.horz(self.e(e_last_horz).next_in_lml) {
            e_last_horz = self.e(e_last_horz).next_in_lml;
        }
        if self.e(e_last_horz).next_in_lml == NIL {
            e_max_pair = self.get_maxima_pair(e_last_horz);
        }

        // std::list iterators over the (sorted) maxima: `max_it` indexes
        // forwards, `max_rit` counts reverse steps from the back.
        let n_max = self.maxima.len();
        let mut max_it = n_max;
        let mut max_rit = n_max;
        if n_max > 0 {
            // get the first maxima in range (X) ...
            let hbx = self.e(horz_edge).bot.x;
            let lht = self.e(e_last_horz).top.x;
            if dir == Direction::LeftToRight {
                max_it = 0;
                while max_it != n_max && self.maxima[max_it] <= hbx {
                    max_it += 1;
                }
                if max_it != n_max && self.maxima[max_it] >= lht {
                    max_it = n_max;
                }
            } else {
                max_rit = 0;
                while max_rit != n_max && self.maxima[n_max - 1 - max_rit] > hbx {
                    max_rit += 1;
                }
                if max_rit != n_max && self.maxima[n_max - 1 - max_rit] <= lht {
                    max_rit = n_max;
                }
            }
        }

        let mut op1 = NIL;

        loop {
            // loop through consec. horizontal edges
            let is_last_horz = horz_edge == e_last_horz;
            let mut e = self.get_next_in_ael(horz_edge, dir);
            while e != NIL {
                // this code block inserts extra coords into horizontal edges (in output
                // polygons) whereever maxima touch these horizontal edges. This helps
                // 'simplifying' polygons (ie if the Simplify property is set).
                if n_max > 0 {
                    if dir == Direction::LeftToRight {
                        while max_it != n_max && self.maxima[max_it] < self.e(e).curr.x {
                            if self.e(horz_edge).out_idx >= 0 && !is_open {
                                let p = IntPoint::new(self.maxima[max_it], self.e(horz_edge).bot.y);
                                self.add_out_pt(horz_edge, p);
                            }
                            max_it += 1;
                        }
                    } else {
                        while max_rit != n_max
                            && self.maxima[n_max - 1 - max_rit] > self.e(e).curr.x
                        {
                            if self.e(horz_edge).out_idx >= 0 && !is_open {
                                let p = IntPoint::new(
                                    self.maxima[n_max - 1 - max_rit],
                                    self.e(horz_edge).bot.y,
                                );
                                self.add_out_pt(horz_edge, p);
                            }
                            max_rit += 1;
                        }
                    }
                }

                if (dir == Direction::LeftToRight && self.e(e).curr.x > horz_right)
                    || (dir == Direction::RightToLeft && self.e(e).curr.x < horz_left)
                {
                    break;
                }

                // Also break if we've got to the end of an intermediate horizontal edge ...
                // nb: Smaller Dx's are to the right of larger Dx's ABOVE the horizontal.
                let hnl = self.e(horz_edge).next_in_lml;
                if self.e(e).curr.x == self.e(horz_edge).top.x
                    && hnl != NIL
                    && self.e(e).dx < self.e(hnl).dx
                {
                    break;
                }

                if self.e(horz_edge).out_idx >= 0 && !is_open {
                    // note: may be done multiple times
                    let c = self.e(e).curr;
                    op1 = self.add_out_pt(horz_edge, c);
                    let mut e_next_horz = self.sorted_edges;
                    while e_next_horz != NIL {
                        if self.e(e_next_horz).out_idx >= 0
                            && horz_segments_overlap(
                                self.e(horz_edge).bot.x,
                                self.e(horz_edge).top.x,
                                self.e(e_next_horz).bot.x,
                                self.e(e_next_horz).top.x,
                            )
                        {
                            let op2 = self.get_last_out_pt(e_next_horz);
                            let t = self.e(e_next_horz).top;
                            self.add_join(op2, op1, t);
                        }
                        e_next_horz = self.e(e_next_horz).next_in_sel;
                    }
                    let b = self.e(horz_edge).bot;
                    self.add_ghost_join(op1, b);
                }

                // OK, so far we're still in range of the horizontal Edge  but make sure
                // we're at the last of consec. horizontals when matching with eMaxPair
                if e == e_max_pair && is_last_horz {
                    if self.e(horz_edge).out_idx >= 0 {
                        let t = self.e(horz_edge).top;
                        self.add_local_max_poly(horz_edge, e_max_pair, t);
                    }
                    self.delete_from_ael(horz_edge);
                    self.delete_from_ael(e_max_pair);
                    return Ok(());
                }

                let p = IntPoint::new(self.e(e).curr.x, self.e(horz_edge).curr.y);
                if dir == Direction::LeftToRight {
                    self.intersect_edges(horz_edge, e, p);
                } else {
                    self.intersect_edges(e, horz_edge, p);
                }
                let e_next = self.get_next_in_ael(e, dir);
                self.swap_positions_in_ael(horz_edge, e);
                e = e_next;
            } // end while(e)

            // Break out of loop if HorzEdge.NextInLML is not also horizontal ...
            let hnl = self.e(horz_edge).next_in_lml;
            if hnl == NIL || !self.horz(hnl) {
                break;
            }

            self.update_edge_into_ael(&mut horz_edge)?;
            if self.e(horz_edge).out_idx >= 0 {
                let b = self.e(horz_edge).bot;
                self.add_out_pt(horz_edge, b);
            }
            (dir, horz_left, horz_right) = self.get_horz_direction(horz_edge);
        } // end for (;;)

        if self.e(horz_edge).out_idx >= 0 && op1 == NIL {
            op1 = self.get_last_out_pt(horz_edge);
            let mut e_next_horz = self.sorted_edges;
            while e_next_horz != NIL {
                if self.e(e_next_horz).out_idx >= 0
                    && horz_segments_overlap(
                        self.e(horz_edge).bot.x,
                        self.e(horz_edge).top.x,
                        self.e(e_next_horz).bot.x,
                        self.e(e_next_horz).top.x,
                    )
                {
                    let op2 = self.get_last_out_pt(e_next_horz);
                    let t = self.e(e_next_horz).top;
                    self.add_join(op2, op1, t);
                }
                e_next_horz = self.e(e_next_horz).next_in_sel;
            }
            let t = self.e(horz_edge).top;
            self.add_ghost_join(op1, t);
        }

        if self.e(horz_edge).next_in_lml != NIL {
            if self.e(horz_edge).out_idx >= 0 {
                let t = self.e(horz_edge).top;
                op1 = self.add_out_pt(horz_edge, t);
                self.update_edge_into_ael(&mut horz_edge)?;
                if self.e(horz_edge).wind_delta == 0 {
                    return Ok(());
                }
                // nb: HorzEdge is no longer horizontal here
                let h = *self.e(horz_edge);
                let e_prev = h.prev_in_ael;
                let e_next = h.next_in_ael;
                if e_prev != NIL
                    && self.e(e_prev).curr.x == h.bot.x
                    && self.e(e_prev).curr.y == h.bot.y
                    && self.e(e_prev).wind_delta != 0
                    && (self.e(e_prev).out_idx >= 0
                        && self.e(e_prev).curr.y > self.e(e_prev).top.y
                        && slopes_equal_e(&h, self.e(e_prev), self.use_full_range))
                {
                    let op2 = self.add_out_pt(e_prev, h.bot);
                    self.add_join(op1, op2, h.top);
                } else if e_next != NIL
                    && self.e(e_next).curr.x == h.bot.x
                    && self.e(e_next).curr.y == h.bot.y
                    && self.e(e_next).wind_delta != 0
                    && self.e(e_next).out_idx >= 0
                    && self.e(e_next).curr.y > self.e(e_next).top.y
                    && slopes_equal_e(&h, self.e(e_next), self.use_full_range)
                {
                    let op2 = self.add_out_pt(e_next, h.bot);
                    self.add_join(op1, op2, h.top);
                }
            } else {
                self.update_edge_into_ael(&mut horz_edge)?;
            }
        } else {
            if self.e(horz_edge).out_idx >= 0 {
                let t = self.e(horz_edge).top;
                self.add_out_pt(horz_edge, t);
            }
            self.delete_from_ael(horz_edge);
        }
        Ok(())
    }

    fn process_intersections(&mut self, top_y: i64) -> Exc<bool> {
        if self.active_edges == NIL {
            return Ok(true);
        }
        // (nothing inside the original's try block can throw)
        self.build_intersect_list(top_y);
        let il_size = self.intersect_list.len();
        if il_size == 0 {
            return Ok(true);
        }
        if il_size == 1 || self.fixup_intersection_order() {
            self.process_intersect_list();
        } else {
            return Ok(false);
        }
        self.sorted_edges = NIL;
        Ok(true)
    }

    fn build_intersect_list(&mut self, top_y: i64) {
        if self.active_edges == NIL {
            return;
        }

        // prepare for sorting ...
        let mut e = self.active_edges;
        self.sorted_edges = e;
        while e != NIL {
            let tx = top_x(self.e(e), top_y);
            let ed = self.em(e);
            ed.prev_in_sel = ed.prev_in_ael;
            ed.next_in_sel = ed.next_in_ael;
            ed.curr.x = tx;
            e = ed.next_in_ael;
        }

        // bubblesort ...
        loop {
            let mut is_modified = false;
            e = self.sorted_edges;
            while self.e(e).next_in_sel != NIL {
                let e_next = self.e(e).next_in_sel;
                if self.e(e).curr.x > self.e(e_next).curr.x {
                    let mut pt = intersect_point(self.e(e), self.e(e_next));
                    if pt.y < top_y {
                        pt = IntPoint::new(top_x(self.e(e), top_y), top_y);
                    }
                    self.intersect_list.push(IntersectNode {
                        edge1: e,
                        edge2: e_next,
                        pt,
                    });
                    self.swap_positions_in_sel(e, e_next);
                    is_modified = true;
                } else {
                    e = e_next;
                }
            }
            let p = self.e(e).prev_in_sel;
            if p != NIL {
                self.em(p).next_in_sel = NIL;
            } else {
                break;
            }
            if !is_modified {
                break;
            }
        }
        self.sorted_edges = NIL; // important
    }

    fn process_intersect_list(&mut self) {
        for i in 0..self.intersect_list.len() {
            let n = self.intersect_list[i];
            self.intersect_edges(n.edge1, n.edge2, n.pt);
            self.swap_positions_in_ael(n.edge1, n.edge2);
        }
        self.intersect_list.clear();
    }

    #[inline]
    fn edges_adjacent(&self, n: &IntersectNode) -> bool {
        self.e(n.edge1).next_in_sel == n.edge2 || self.e(n.edge1).prev_in_sel == n.edge2
    }

    fn fixup_intersection_order(&mut self) -> bool {
        // pre-condition: intersections are sorted Bottom-most first.
        // Now it's crucial that intersections are made only between adjacent edges,
        // so to ensure this the order of intersections may need adjusting ...
        self.copy_ael_to_sel();
        cxxsort::sort(
            &mut self.intersect_list,
            |a: &IntersectNode, b: &IntersectNode| b.pt.y < a.pt.y,
        );
        let cnt = self.intersect_list.len();
        for i in 0..cnt {
            if !self.edges_adjacent(&self.intersect_list[i]) {
                let mut j = i + 1;
                while j < cnt && !self.edges_adjacent(&self.intersect_list[j]) {
                    j += 1;
                }
                if j == cnt {
                    return false;
                }
                self.intersect_list.swap(i, j);
            }
            let n = self.intersect_list[i];
            self.swap_positions_in_sel(n.edge1, n.edge2);
        }
        true
    }

    fn do_maxima(&mut self, e: usize) -> Exc {
        let e_max_pair = self.get_maxima_pair_ex(e);
        if e_max_pair == NIL {
            if self.e(e).out_idx >= 0 {
                let t = self.e(e).top;
                self.add_out_pt(e, t);
            }
            self.delete_from_ael(e);
            return Ok(());
        }

        let mut e_next = self.e(e).next_in_ael;
        while e_next != NIL && e_next != e_max_pair {
            let t = self.e(e).top;
            self.intersect_edges(e, e_next, t);
            self.swap_positions_in_ael(e, e_next);
            e_next = self.e(e).next_in_ael;
        }

        if self.e(e).out_idx == UNASSIGNED && self.e(e_max_pair).out_idx == UNASSIGNED {
            self.delete_from_ael(e);
            self.delete_from_ael(e_max_pair);
        } else if self.e(e).out_idx >= 0 && self.e(e_max_pair).out_idx >= 0 {
            if self.e(e).out_idx >= 0 {
                let t = self.e(e).top;
                self.add_local_max_poly(e, e_max_pair, t);
            }
            self.delete_from_ael(e);
            self.delete_from_ael(e_max_pair);
        } else if self.e(e).wind_delta == 0 {
            if self.e(e).out_idx >= 0 {
                let t = self.e(e).top;
                self.add_out_pt(e, t);
                self.em(e).out_idx = UNASSIGNED;
            }
            self.delete_from_ael(e);

            if self.e(e_max_pair).out_idx >= 0 {
                let t = self.e(e).top;
                self.add_out_pt(e_max_pair, t);
                self.em(e_max_pair).out_idx = UNASSIGNED;
            }
            self.delete_from_ael(e_max_pair);
        } else {
            return Err(ClipperException("DoMaxima error"));
        }
        Ok(())
    }

    fn process_edges_at_top_of_scanbeam(&mut self, top_y: i64) -> Exc {
        let mut e = self.active_edges;
        while e != NIL {
            // 1. process maxima, treating them as if they're 'bent' horizontal edges,
            //    but exclude maxima with horizontal edges. nb: e can't be a horizontal.
            let mut is_maxima_edge = self.is_maxima(e, top_y);

            if is_maxima_edge {
                let e_max_pair = self.get_maxima_pair_ex(e);
                is_maxima_edge = e_max_pair == NIL || !self.horz(e_max_pair);
            }

            if is_maxima_edge {
                if self.strict_simple {
                    let tx = self.e(e).top.x;
                    self.maxima.push(tx);
                }
                let e_prev = self.e(e).prev_in_ael;
                self.do_maxima(e)?;
                if e_prev == NIL {
                    e = self.active_edges;
                } else {
                    e = self.e(e_prev).next_in_ael;
                }
            } else {
                // 2. promote horizontal edges, otherwise update Curr.X and Curr.Y ...
                if self.is_intermediate(e, top_y) && self.horz(self.e(e).next_in_lml) {
                    self.update_edge_into_ael(&mut e)?;
                    if self.e(e).out_idx >= 0 {
                        let b = self.e(e).bot;
                        self.add_out_pt(e, b);
                    }
                    self.add_edge_to_sel(e);
                } else {
                    let tx = top_x(self.e(e), top_y);
                    let ed = self.em(e);
                    ed.curr.x = tx;
                    ed.curr.y = top_y;
                }

                // When StrictlySimple and 'e' is being touched by another edge, then
                // make sure both edges have a vertex here ...
                if self.strict_simple {
                    let e_prev = self.e(e).prev_in_ael;
                    if self.e(e).out_idx >= 0
                        && self.e(e).wind_delta != 0
                        && e_prev != NIL
                        && self.e(e_prev).out_idx >= 0
                        && self.e(e_prev).curr.x == self.e(e).curr.x
                        && self.e(e_prev).wind_delta != 0
                    {
                        let pt = self.e(e).curr;
                        let op = self.add_out_pt(e_prev, pt);
                        let op2 = self.add_out_pt(e, pt);
                        self.add_join(op, op2, pt); // StrictlySimple (type-3) join
                    }
                }

                e = self.e(e).next_in_ael;
            }
        }

        // 3. Process horizontals at the Top of the scanbeam ...
        self.maxima.sort_unstable();
        self.process_horizontals()?;
        self.maxima.clear();

        // 4. Promote intermediate vertices ...
        e = self.active_edges;
        while e != NIL {
            if self.is_intermediate(e, top_y) {
                let mut op = NIL;
                if self.e(e).out_idx >= 0 {
                    let t = self.e(e).top;
                    op = self.add_out_pt(e, t);
                }
                self.update_edge_into_ael(&mut e)?;

                // if output polygons share an edge, they'll need joining later ...
                let ed = *self.e(e);
                let e_prev = ed.prev_in_ael;
                let e_next = ed.next_in_ael;
                if e_prev != NIL
                    && self.e(e_prev).curr.x == ed.bot.x
                    && self.e(e_prev).curr.y == ed.bot.y
                    && op != NIL
                    && self.e(e_prev).out_idx >= 0
                    && self.e(e_prev).curr.y > self.e(e_prev).top.y
                    && slopes_equal4(
                        ed.curr,
                        ed.top,
                        self.e(e_prev).curr,
                        self.e(e_prev).top,
                        self.use_full_range,
                    )
                    && ed.wind_delta != 0
                    && self.e(e_prev).wind_delta != 0
                {
                    let op2 = self.add_out_pt(e_prev, ed.bot);
                    self.add_join(op, op2, ed.top);
                } else if e_next != NIL
                    && self.e(e_next).curr.x == ed.bot.x
                    && self.e(e_next).curr.y == ed.bot.y
                    && op != NIL
                    && self.e(e_next).out_idx >= 0
                    && self.e(e_next).curr.y > self.e(e_next).top.y
                    && slopes_equal4(
                        ed.curr,
                        ed.top,
                        self.e(e_next).curr,
                        self.e(e_next).top,
                        self.use_full_range,
                    )
                    && ed.wind_delta != 0
                    && self.e(e_next).wind_delta != 0
                {
                    let op2 = self.add_out_pt(e_next, ed.bot);
                    self.add_join(op, op2, ed.top);
                }
            }
            e = self.e(e).next_in_ael;
        }
        Ok(())
    }

    fn fixup_out_polyline(&mut self, outrec: usize) {
        let mut pp = self.or(outrec).pts;
        let mut last_pp = self.op(pp).prev;
        while pp != last_pp {
            pp = self.op(pp).next;
            if self.op(pp).pt == self.op(self.op(pp).prev).pt {
                if pp == last_pp {
                    last_pp = self.op(pp).prev;
                }
                let tmp_pp = self.op(pp).prev;
                let pn = self.op(pp).next;
                self.opm(tmp_pp).next = pn;
                self.opm(pn).prev = tmp_pp;
                pp = tmp_pp;
            }
        }

        if pp == self.op(pp).prev {
            self.orm(outrec).pts = NIL;
        }
    }

    fn fixup_out_polygon(&mut self, outrec: usize) {
        // FixupOutPolygon() - removes duplicate points and simplifies consecutive
        // parallel edges by removing the middle vertex.
        let mut last_ok = NIL;
        self.orm(outrec).bottom_pt = NIL;
        let mut pp = self.or(outrec).pts;
        let preserve_col = self.preserve_collinear || self.strict_simple;

        loop {
            let p = *self.op(pp);
            if p.prev == pp || p.prev == p.next {
                self.orm(outrec).pts = NIL;
                return;
            }

            // test for duplicate points and collinear edges ...
            let (prev_pt, next_pt) = (self.op(p.prev).pt, self.op(p.next).pt);
            if p.pt == next_pt
                || p.pt == prev_pt
                || (slopes_equal3(prev_pt, p.pt, next_pt, self.use_full_range)
                    && (!preserve_col || !pt2_is_between_pt1_and_pt3(prev_pt, p.pt, next_pt)))
            {
                last_ok = NIL;
                self.opm(p.prev).next = p.next;
                self.opm(p.next).prev = p.prev;
                pp = p.prev;
            } else if pp == last_ok {
                break;
            } else {
                if last_ok == NIL {
                    last_ok = pp;
                }
                pp = p.next;
            }
        }
        self.orm(outrec).pts = pp;
    }

    fn build_result(&self, polys: &mut Paths) {
        polys.reserve(self.out_recs.len());
        for o in &self.out_recs {
            if o.pts == NIL {
                continue;
            }
            let mut p = self.op(o.pts).prev;
            let cnt = self.point_count(p);
            if cnt < 2 {
                continue;
            }
            let mut pg = Path::with_capacity(cnt as usize);
            for _ in 0..cnt {
                pg.push(self.op(p).pt);
                p = self.op(p).prev;
            }
            polys.push(pg);
        }
    }

    fn build_result2(&mut self, polytree: &mut PolyTree) {
        polytree.clear();
        // add each output polygon/contour to polytree ...
        for i in 0..self.out_recs.len() {
            let o = *self.or(i);
            let cnt = self.point_count(o.pts);
            if (o.is_open && cnt < 2) || (!o.is_open && cnt < 3) {
                continue;
            }
            self.fix_hole_linkage(i);
            let pn = polytree.new_node();
            self.orm(i).poly_nd = pn;
            let mut contour = Path::with_capacity(cnt as usize);
            let mut op = self.op(o.pts).prev;
            for _ in 0..cnt {
                contour.push(self.op(op).pt);
                op = self.op(op).prev;
            }
            let node = polytree.node_mut(pn);
            node.parent = None;
            node.index = 0;
            node.contour = contour;
        }

        // fixup PolyNode links etc ...
        for i in 0..self.out_recs.len() {
            let o = *self.or(i);
            if o.poly_nd == NIL {
                continue;
            }
            if o.is_open {
                polytree.node_mut(o.poly_nd).is_open = true;
                polytree.add_child(PolyTree::ROOT, o.poly_nd);
            } else if o.first_left != NIL && self.or(o.first_left).poly_nd != NIL {
                polytree.add_child(self.or(o.first_left).poly_nd, o.poly_nd);
            } else {
                polytree.add_child(PolyTree::ROOT, o.poly_nd);
            }
        }
    }

    fn e2_inserts_before_e1(&self, e1: usize, e2: usize) -> bool {
        let (e1, e2) = (self.e(e1), self.e(e2));
        if e2.curr.x == e1.curr.x {
            if e2.top.y > e1.top.y {
                e2.top.x < top_x(e1, e2.top.y)
            } else {
                e1.top.x > top_x(e2, e1.top.y)
            }
        } else {
            e2.curr.x < e1.curr.x
        }
    }

    fn insert_edge_into_ael(&mut self, edge: usize, mut start_edge: usize) {
        if self.active_edges == NIL {
            self.em(edge).prev_in_ael = NIL;
            self.em(edge).next_in_ael = NIL;
            self.active_edges = edge;
        } else if start_edge == NIL && self.e2_inserts_before_e1(self.active_edges, edge) {
            let ae = self.active_edges;
            self.em(edge).prev_in_ael = NIL;
            self.em(edge).next_in_ael = ae;
            self.em(ae).prev_in_ael = edge;
            self.active_edges = edge;
        } else {
            if start_edge == NIL {
                start_edge = self.active_edges;
            }
            while self.e(start_edge).next_in_ael != NIL
                && !self.e2_inserts_before_e1(self.e(start_edge).next_in_ael, edge)
            {
                start_edge = self.e(start_edge).next_in_ael;
            }
            let sn = self.e(start_edge).next_in_ael;
            self.em(edge).next_in_ael = sn;
            if sn != NIL {
                self.em(sn).prev_in_ael = edge;
            }
            self.em(edge).prev_in_ael = start_edge;
            self.em(start_edge).next_in_ael = edge;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn join_horz(
        &mut self,
        mut op1: usize,
        mut op1b: usize,
        mut op2: usize,
        mut op2b: usize,
        pt: IntPoint,
        discard_left: bool,
    ) -> bool {
        let dir1 = if self.op(op1).pt.x > self.op(op1b).pt.x {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        };
        let dir2 = if self.op(op2).pt.x > self.op(op2b).pt.x {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        };
        if dir1 == dir2 {
            return false;
        }

        // When DiscardLeft, we want Op1b to be on the Left of Op1, otherwise we
        // want Op1b to be on the Right. (And likewise with Op2 and Op2b.)
        // So, to facilitate this while inserting Op1b and Op2b ...
        // when DiscardLeft, make sure we're AT or RIGHT of Pt before adding Op1b,
        // otherwise make sure we're AT or LEFT of Pt. (Likewise with Op2b.)
        if dir1 == Direction::LeftToRight {
            loop {
                let n = self.op(self.op(op1).next).pt;
                if n.x <= pt.x && n.x >= self.op(op1).pt.x && n.y == pt.y {
                    op1 = self.op(op1).next;
                } else {
                    break;
                }
            }
            if discard_left && self.op(op1).pt.x != pt.x {
                op1 = self.op(op1).next;
            }
            op1b = self.dup_out_pt(op1, !discard_left);
            if self.op(op1b).pt != pt {
                op1 = op1b;
                self.opm(op1).pt = pt;
                op1b = self.dup_out_pt(op1, !discard_left);
            }
        } else {
            loop {
                let n = self.op(self.op(op1).next).pt;
                if n.x >= pt.x && n.x <= self.op(op1).pt.x && n.y == pt.y {
                    op1 = self.op(op1).next;
                } else {
                    break;
                }
            }
            if !discard_left && self.op(op1).pt.x != pt.x {
                op1 = self.op(op1).next;
            }
            op1b = self.dup_out_pt(op1, discard_left);
            if self.op(op1b).pt != pt {
                op1 = op1b;
                self.opm(op1).pt = pt;
                op1b = self.dup_out_pt(op1, discard_left);
            }
        }

        if dir2 == Direction::LeftToRight {
            loop {
                let n = self.op(self.op(op2).next).pt;
                if n.x <= pt.x && n.x >= self.op(op2).pt.x && n.y == pt.y {
                    op2 = self.op(op2).next;
                } else {
                    break;
                }
            }
            if discard_left && self.op(op2).pt.x != pt.x {
                op2 = self.op(op2).next;
            }
            op2b = self.dup_out_pt(op2, !discard_left);
            if self.op(op2b).pt != pt {
                op2 = op2b;
                self.opm(op2).pt = pt;
                op2b = self.dup_out_pt(op2, !discard_left);
            }
        } else {
            loop {
                let n = self.op(self.op(op2).next).pt;
                if n.x >= pt.x && n.x <= self.op(op2).pt.x && n.y == pt.y {
                    op2 = self.op(op2).next;
                } else {
                    break;
                }
            }
            if !discard_left && self.op(op2).pt.x != pt.x {
                op2 = self.op(op2).next;
            }
            op2b = self.dup_out_pt(op2, discard_left);
            if self.op(op2b).pt != pt {
                op2 = op2b;
                self.opm(op2).pt = pt;
                op2b = self.dup_out_pt(op2, discard_left);
            }
        }

        if (dir1 == Direction::LeftToRight) == discard_left {
            self.opm(op1).prev = op2;
            self.opm(op2).next = op1;
            self.opm(op1b).next = op2b;
            self.opm(op2b).prev = op1b;
        } else {
            self.opm(op1).next = op2;
            self.opm(op2).prev = op1;
            self.opm(op1b).prev = op2b;
            self.opm(op2b).next = op1b;
        }
        true
    }

    fn join_points(&mut self, ji: usize, out_rec1: usize, out_rec2: usize) -> bool {
        let j = self.joins[ji];
        let mut op1 = j.out_pt1;
        let mut op2 = j.out_pt2;
        let mut op1b;
        let mut op2b;

        // There are 3 kinds of joins for output polygons ...
        // 1. Horizontal joins where Join.OutPt1 & Join.OutPt2 are vertices anywhere
        // along (horizontal) collinear edges (& Join.OffPt is on the same horizontal).
        // 2. Non-horizontal joins where Join.OutPt1 & Join.OutPt2 are at the same
        // location at the Bottom of the overlapping segment (& Join.OffPt is above).
        // 3. StrictSimple joins where edges touch but are not collinear and where
        // Join.OutPt1, Join.OutPt2 & Join.OffPt all share the same point.
        let is_horizontal = self.op(j.out_pt1).pt.y == j.off_pt.y;

        if is_horizontal && j.off_pt == self.op(j.out_pt1).pt && j.off_pt == self.op(j.out_pt2).pt {
            // Strictly Simple join ...
            if out_rec1 != out_rec2 {
                return false;
            }
            op1b = self.op(j.out_pt1).next;
            while op1b != op1 && self.op(op1b).pt == j.off_pt {
                op1b = self.op(op1b).next;
            }
            let reverse1 = self.op(op1b).pt.y > j.off_pt.y;
            op2b = self.op(j.out_pt2).next;
            while op2b != op2 && self.op(op2b).pt == j.off_pt {
                op2b = self.op(op2b).next;
            }
            let reverse2 = self.op(op2b).pt.y > j.off_pt.y;
            if reverse1 == reverse2 {
                return false;
            }
            if reverse1 {
                op1b = self.dup_out_pt(op1, false);
                op2b = self.dup_out_pt(op2, true);
                self.opm(op1).prev = op2;
                self.opm(op2).next = op1;
                self.opm(op1b).next = op2b;
                self.opm(op2b).prev = op1b;
            } else {
                op1b = self.dup_out_pt(op1, true);
                op2b = self.dup_out_pt(op2, false);
                self.opm(op1).next = op2;
                self.opm(op2).prev = op1;
                self.opm(op1b).prev = op2b;
                self.opm(op2b).next = op1b;
            }
            self.joins[ji].out_pt1 = op1;
            self.joins[ji].out_pt2 = op1b;
            true
        } else if is_horizontal {
            // treat horizontal joins differently to non-horizontal joins since with
            // them we're not yet sure where the overlapping is. OutPt1.Pt & OutPt2.Pt
            // may be anywhere along the horizontal edge.
            op1b = op1;
            while self.op(self.op(op1).prev).pt.y == self.op(op1).pt.y
                && self.op(op1).prev != op1b
                && self.op(op1).prev != op2
            {
                op1 = self.op(op1).prev;
            }
            while self.op(self.op(op1b).next).pt.y == self.op(op1b).pt.y
                && self.op(op1b).next != op1
                && self.op(op1b).next != op2
            {
                op1b = self.op(op1b).next;
            }
            if self.op(op1b).next == op1 || self.op(op1b).next == op2 {
                return false; // a flat 'polygon'
            }

            op2b = op2;
            while self.op(self.op(op2).prev).pt.y == self.op(op2).pt.y
                && self.op(op2).prev != op2b
                && self.op(op2).prev != op1b
            {
                op2 = self.op(op2).prev;
            }
            while self.op(self.op(op2b).next).pt.y == self.op(op2b).pt.y
                && self.op(op2b).next != op2
                && self.op(op2b).next != op1
            {
                op2b = self.op(op2b).next;
            }
            if self.op(op2b).next == op2 || self.op(op2b).next == op1 {
                return false; // a flat 'polygon'
            }

            // Op1 --> Op1b & Op2 --> Op2b are the extremites of the horizontal edges
            let (p1, p1b, p2, p2b) = (
                self.op(op1).pt,
                self.op(op1b).pt,
                self.op(op2).pt,
                self.op(op2b).pt,
            );
            let (ok, left, right) = get_overlap(p1.x, p1b.x, p2.x, p2b.x);
            if !ok {
                return false;
            }

            // DiscardLeftSide: when overlapping edges are joined, a spike will created
            // which needs to be cleaned up. However, we don't want Op1 or Op2 caught up
            // on the discard Side as either may still be needed for other joins ...
            let (pt, discard_left_side) = if p1.x >= left && p1.x <= right {
                (p1, p1.x > p1b.x)
            } else if p2.x >= left && p2.x <= right {
                (p2, p2.x > p2b.x)
            } else if p1b.x >= left && p1b.x <= right {
                (p1b, p1b.x > p1.x)
            } else {
                (p2b, p2b.x > p2.x)
            };
            self.joins[ji].out_pt1 = op1;
            self.joins[ji].out_pt2 = op2;
            self.join_horz(op1, op1b, op2, op2b, pt, discard_left_side)
        } else {
            // nb: For non-horizontal joins ...
            //    1. Jr.OutPt1.Pt.Y == Jr.OutPt2.Pt.Y
            //    2. Jr.OutPt1.Pt > Jr.OffPt.Y

            // make sure the polygons are correctly oriented ...
            let full = self.use_full_range;
            op1b = self.op(op1).next;
            while self.op(op1b).pt == self.op(op1).pt && op1b != op1 {
                op1b = self.op(op1b).next;
            }
            let reverse1 = self.op(op1b).pt.y > self.op(op1).pt.y
                || !slopes_equal3(self.op(op1).pt, self.op(op1b).pt, j.off_pt, full);
            if reverse1 {
                op1b = self.op(op1).prev;
                while self.op(op1b).pt == self.op(op1).pt && op1b != op1 {
                    op1b = self.op(op1b).prev;
                }
                if self.op(op1b).pt.y > self.op(op1).pt.y
                    || !slopes_equal3(self.op(op1).pt, self.op(op1b).pt, j.off_pt, full)
                {
                    return false;
                }
            }
            op2b = self.op(op2).next;
            while self.op(op2b).pt == self.op(op2).pt && op2b != op2 {
                op2b = self.op(op2b).next;
            }
            let reverse2 = self.op(op2b).pt.y > self.op(op2).pt.y
                || !slopes_equal3(self.op(op2).pt, self.op(op2b).pt, j.off_pt, full);
            if reverse2 {
                op2b = self.op(op2).prev;
                while self.op(op2b).pt == self.op(op2).pt && op2b != op2 {
                    op2b = self.op(op2b).prev;
                }
                if self.op(op2b).pt.y > self.op(op2).pt.y
                    || !slopes_equal3(self.op(op2).pt, self.op(op2b).pt, j.off_pt, full)
                {
                    return false;
                }
            }

            if op1b == op1
                || op2b == op2
                || op1b == op2b
                || (out_rec1 == out_rec2 && reverse1 == reverse2)
            {
                return false;
            }

            if reverse1 {
                op1b = self.dup_out_pt(op1, false);
                op2b = self.dup_out_pt(op2, true);
                self.opm(op1).prev = op2;
                self.opm(op2).next = op1;
                self.opm(op1b).next = op2b;
                self.opm(op2b).prev = op1b;
            } else {
                op1b = self.dup_out_pt(op1, true);
                op2b = self.dup_out_pt(op2, false);
                self.opm(op1).next = op2;
                self.opm(op2).prev = op1;
                self.opm(op1b).prev = op2b;
                self.opm(op2b).next = op1b;
            }
            self.joins[ji].out_pt1 = op1;
            self.joins[ji].out_pt2 = op1b;
            true
        }
    }

    fn parse_first_left(&self, mut first_left: usize) -> usize {
        while first_left != NIL && self.or(first_left).pts == NIL {
            first_left = self.or(first_left).first_left;
        }
        first_left
    }

    fn fixup_first_lefts1(&mut self, old_out_rec: usize, new_out_rec: usize) {
        // tests if NewOutRec contains the polygon before reassigning FirstLeft
        for i in 0..self.out_recs.len() {
            let o = *self.or(i);
            let first_left = self.parse_first_left(o.first_left);
            if o.pts != NIL
                && first_left == old_out_rec
                && self.poly2_contains_poly1(o.pts, self.or(new_out_rec).pts)
            {
                self.orm(i).first_left = new_out_rec;
            }
        }
    }

    fn fixup_first_lefts2(&mut self, inner_out_rec: usize, outer_out_rec: usize) {
        // A polygon has split into two such that one is now the inner of the other.
        // It's possible that these polygons now wrap around other polygons, so check
        // every polygon that's also contained by OuterOutRec's FirstLeft container
        // (including 0) to see if they've become inner to the new inner polygon ...
        let orfl = self.or(outer_out_rec).first_left;
        for i in 0..self.out_recs.len() {
            let o = *self.or(i);
            if o.pts == NIL || i == outer_out_rec || i == inner_out_rec {
                continue;
            }
            let first_left = self.parse_first_left(o.first_left);
            if first_left != orfl && first_left != inner_out_rec && first_left != outer_out_rec {
                continue;
            }
            if self.poly2_contains_poly1(o.pts, self.or(inner_out_rec).pts) {
                self.orm(i).first_left = inner_out_rec;
            } else if self.poly2_contains_poly1(o.pts, self.or(outer_out_rec).pts) {
                self.orm(i).first_left = outer_out_rec;
            } else if o.first_left == inner_out_rec || o.first_left == outer_out_rec {
                self.orm(i).first_left = orfl;
            }
        }
    }

    fn fixup_first_lefts3(&mut self, old_out_rec: usize, new_out_rec: usize) {
        // reassigns FirstLeft WITHOUT testing if NewOutRec contains the polygon
        for i in 0..self.out_recs.len() {
            let o = *self.or(i);
            let first_left = self.parse_first_left(o.first_left);
            if o.pts != NIL && first_left == old_out_rec {
                self.orm(i).first_left = new_out_rec;
            }
        }
    }

    fn join_common_edges(&mut self) {
        for i in 0..self.joins.len() {
            let join = self.joins[i];

            let out_rec1 = self.get_out_rec(self.op(join.out_pt1).idx);
            let mut out_rec2 = self.get_out_rec(self.op(join.out_pt2).idx);

            if self.or(out_rec1).pts == NIL || self.or(out_rec2).pts == NIL {
                continue;
            }
            if self.or(out_rec1).is_open || self.or(out_rec2).is_open {
                continue;
            }

            // get the polygon fragment with the correct hole state (FirstLeft)
            // before calling JoinPoints() ...
            let hole_state_rec = if out_rec1 == out_rec2 {
                out_rec1
            } else if self.out_rec1_right_of_out_rec2(out_rec1, out_rec2) {
                out_rec2
            } else if self.out_rec1_right_of_out_rec2(out_rec2, out_rec1) {
                out_rec1
            } else {
                self.get_lowermost_rec(out_rec1, out_rec2)
            };

            if !self.join_points(i, out_rec1, out_rec2) {
                continue;
            }
            let join = self.joins[i];

            if out_rec1 == out_rec2 {
                // instead of joining two polygons, we've just created a new one by
                // splitting one polygon into two.
                self.orm(out_rec1).pts = join.out_pt1;
                self.orm(out_rec1).bottom_pt = NIL;
                out_rec2 = self.create_out_rec();
                self.orm(out_rec2).pts = join.out_pt2;

                // update all OutRec2.Pts Idx's ...
                self.update_out_pt_idxs(out_rec2);

                if self.poly2_contains_poly1(self.or(out_rec2).pts, self.or(out_rec1).pts) {
                    // outRec1 contains outRec2 ...
                    let h = !self.or(out_rec1).is_hole;
                    self.orm(out_rec2).is_hole = h;
                    self.orm(out_rec2).first_left = out_rec1;

                    if self.using_poly_tree {
                        self.fixup_first_lefts2(out_rec2, out_rec1);
                    }

                    let o2 = *self.or(out_rec2);
                    if (o2.is_hole ^ self.reverse_output) == (self.area_op(o2.pts) > 0.0) {
                        self.reverse_poly_pt_links(o2.pts);
                    }
                } else if self.poly2_contains_poly1(self.or(out_rec1).pts, self.or(out_rec2).pts) {
                    // outRec2 contains outRec1 ...
                    let h1 = self.or(out_rec1).is_hole;
                    self.orm(out_rec2).is_hole = h1;
                    self.orm(out_rec1).is_hole = !h1;
                    let fl = self.or(out_rec1).first_left;
                    self.orm(out_rec2).first_left = fl;
                    self.orm(out_rec1).first_left = out_rec2;

                    if self.using_poly_tree {
                        self.fixup_first_lefts2(out_rec1, out_rec2);
                    }

                    let o1 = *self.or(out_rec1);
                    if (o1.is_hole ^ self.reverse_output) == (self.area_op(o1.pts) > 0.0) {
                        self.reverse_poly_pt_links(o1.pts);
                    }
                } else {
                    // the 2 polygons are completely separate ...
                    let o1 = *self.or(out_rec1);
                    self.orm(out_rec2).is_hole = o1.is_hole;
                    self.orm(out_rec2).first_left = o1.first_left;

                    // fixup FirstLeft pointers that may need reassigning to OutRec2
                    if self.using_poly_tree {
                        self.fixup_first_lefts1(out_rec1, out_rec2);
                    }
                }
            } else {
                // joined 2 polygons together ...
                self.orm(out_rec2).pts = NIL;
                self.orm(out_rec2).bottom_pt = NIL;
                let i1 = self.or(out_rec1).idx;
                self.orm(out_rec2).idx = i1;

                let h = self.or(hole_state_rec).is_hole;
                self.orm(out_rec1).is_hole = h;
                if hole_state_rec == out_rec2 {
                    let fl = self.or(out_rec2).first_left;
                    self.orm(out_rec1).first_left = fl;
                }
                self.orm(out_rec2).first_left = out_rec1;

                if self.using_poly_tree {
                    self.fixup_first_lefts3(out_rec2, out_rec1);
                }
            }
        }
    }

    fn do_simple_polygons(&mut self) {
        let mut i = 0;
        while i < self.out_recs.len() {
            let outrec = i;
            i += 1;
            let mut op = self.or(outrec).pts;
            if op == NIL || self.or(outrec).is_open {
                continue;
            }
            loop {
                // for each Pt in Polygon until duplicate found do ...
                let mut op2 = self.op(op).next;
                while op2 != self.or(outrec).pts {
                    if self.op(op).pt == self.op(op2).pt
                        && self.op(op2).next != op
                        && self.op(op2).prev != op
                    {
                        // split the polygon into two ...
                        let op3 = self.op(op).prev;
                        let op4 = self.op(op2).prev;
                        self.opm(op).prev = op4;
                        self.opm(op4).next = op;
                        self.opm(op2).prev = op3;
                        self.opm(op3).next = op2;

                        self.orm(outrec).pts = op;
                        let outrec2 = self.create_out_rec();
                        self.orm(outrec2).pts = op2;
                        self.update_out_pt_idxs(outrec2);
                        if self.poly2_contains_poly1(self.or(outrec2).pts, self.or(outrec).pts) {
                            // OutRec2 is contained by OutRec1 ...
                            let h = !self.or(outrec).is_hole;
                            self.orm(outrec2).is_hole = h;
                            self.orm(outrec2).first_left = outrec;
                            if self.using_poly_tree {
                                self.fixup_first_lefts2(outrec2, outrec);
                            }
                        } else if self
                            .poly2_contains_poly1(self.or(outrec).pts, self.or(outrec2).pts)
                        {
                            // OutRec1 is contained by OutRec2 ...
                            let h = self.or(outrec).is_hole;
                            self.orm(outrec2).is_hole = h;
                            self.orm(outrec).is_hole = !h;
                            let fl = self.or(outrec).first_left;
                            self.orm(outrec2).first_left = fl;
                            self.orm(outrec).first_left = outrec2;
                            if self.using_poly_tree {
                                self.fixup_first_lefts2(outrec, outrec2);
                            }
                        } else {
                            // the 2 polygons are separate ...
                            let o = *self.or(outrec);
                            self.orm(outrec2).is_hole = o.is_hole;
                            self.orm(outrec2).first_left = o.first_left;
                            if self.using_poly_tree {
                                self.fixup_first_lefts1(outrec, outrec2);
                            }
                        }
                        op2 = op; // ie get ready for the Next iteration
                    }
                    op2 = self.op(op2).next;
                }
                op = self.op(op).next;
                if op == self.or(outrec).pts {
                    break;
                }
            }
        }
    }
}
