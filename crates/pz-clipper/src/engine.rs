//! Port of Clipper2 1.5.4 `clipper.engine.cpp`: the Vatti sweep used by
//! `Clipper64` (boolean operations producing a `PolyTree64`).
//!
//! Copyright (c) Angus Johnson 2010-2025. Distributed under the Boost
//! Software License, Version 1.0 (see `LICENSE-BOOST` in this crate).
//!
//! The C++ pointer graph (vertices, active edges, output points and output
//! records) is kept in index arenas; `NIL` stands for `nullptr`. Objects the
//! reference deletes are simply left unreferenced in the arena.

use crate::core::*;
use crate::cxxsort;

const NIL: usize = usize::MAX;

/// `ClipType`. Values outside the C++ enum take the `default:` branches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipType {
    NoClip,
    Intersection,
    Union,
    Difference,
    Xor,
    /// Any other value received through JNI.
    Other(i32),
}

impl ClipType {
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => ClipType::NoClip,
            1 => ClipType::Intersection,
            2 => ClipType::Union,
            3 => ClipType::Difference,
            4 => ClipType::Xor,
            o => ClipType::Other(o),
        }
    }
}

/// `PathType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathType {
    Subject,
    Clip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JoinWith {
    NoJoin,
    Left,
    Right,
}

const VF_OPEN_START: u32 = 1;
const VF_OPEN_END: u32 = 2;
const VF_LOCAL_MAX: u32 = 4;
const VF_LOCAL_MIN: u32 = 8;

#[derive(Clone, Copy, Debug, Default)]
struct Vertex {
    pt: Point64,
    next: usize,
    prev: usize,
    flags: u32,
}

#[derive(Clone, Copy, Debug)]
struct LocalMinima {
    vertex: usize,
    polytype: PathType,
    is_open: bool,
}

#[derive(Clone, Copy, Debug)]
struct OutPt {
    pt: Point64,
    next: usize,
    prev: usize,
    outrec: usize,
    /// `horz != nullptr` (only ever tested for null).
    horz: bool,
}

#[derive(Clone, Debug)]
struct OutRec {
    idx: usize,
    owner: usize,
    front_edge: usize,
    back_edge: usize,
    pts: usize,
    polypath: usize,
    splits: Option<Vec<usize>>,
    recursive_split: usize,
    bounds: Rect64,
    path: Path64,
    is_open: bool,
}

#[derive(Clone, Copy, Debug)]
struct Active {
    bot: Point64,
    top: Point64,
    curr_x: i64,
    dx: f64,
    wind_dx: i32,
    wind_cnt: i32,
    wind_cnt2: i32,
    outrec: usize,
    prev_in_ael: usize,
    next_in_ael: usize,
    prev_in_sel: usize,
    next_in_sel: usize,
    jump: usize,
    vertex_top: usize,
    local_min: usize,
    is_left_bound: bool,
    join_with: JoinWith,
}

impl Default for Active {
    fn default() -> Self {
        Active {
            bot: Point64::default(),
            top: Point64::default(),
            curr_x: 0,
            dx: 0.0,
            wind_dx: 1,
            wind_cnt: 0,
            wind_cnt2: 0,
            outrec: NIL,
            prev_in_ael: NIL,
            next_in_ael: NIL,
            prev_in_sel: NIL,
            next_in_sel: NIL,
            jump: NIL,
            vertex_top: NIL,
            local_min: NIL,
            is_left_bound: false,
            join_with: JoinWith::NoJoin,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct IntersectNode {
    pt: Point64,
    edge1: usize,
    edge2: usize,
}

#[derive(Clone, Copy, Debug)]
struct HorzSegment {
    left_op: usize,
    right_op: usize,
    left_to_right: bool,
}

#[derive(Clone, Copy, Debug)]
struct HorzJoin {
    op1: usize,
    op2: usize,
}

/// A node of a `PolyTree64`: a polygon and the polygons it directly
/// contains (holes for outers, islands for holes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PolyPath64 {
    pub polygon: Path64,
    pub childs: Vec<PolyPath64>,
}

impl PolyPath64 {
    pub fn count(&self) -> usize {
        self.childs.len()
    }
}

/// Flat polytree used while building (`PolyPath*` pointers become indices;
/// node 0 is the root).
struct TreeBuilder {
    nodes: Vec<(Path64, Vec<usize>)>,
}

impl TreeBuilder {
    fn new() -> Self {
        TreeBuilder {
            nodes: vec![(Path64::new(), Vec::new())],
        }
    }
    fn add_child(&mut self, parent: usize, path: Path64) -> usize {
        let id = self.nodes.len();
        self.nodes.push((path, Vec::new()));
        self.nodes[parent].1.push(id);
        id
    }
    fn into_tree(mut self) -> PolyPath64 {
        fn build(nodes: &mut Vec<(Path64, Vec<usize>)>, i: usize) -> PolyPath64 {
            let polygon = std::mem::take(&mut nodes[i].0);
            let kids = std::mem::take(&mut nodes[i].1);
            PolyPath64 {
                polygon,
                childs: kids.into_iter().map(|k| build(nodes, k)).collect(),
            }
        }
        build(&mut self.nodes, 0)
    }
}

/// `Clipper64` (with its `ClipperBase`).
pub struct Clipper64 {
    cliptype: ClipType,
    fillrule: FillRule,
    bot_y: i64,
    minima_list_sorted: bool,
    using_polytree: bool,
    actives: usize,
    sel: usize,
    minima_list: Vec<LocalMinima>,
    current_locmin_iter: usize,
    vertices: Vec<Vertex>,
    scanline_list: std::collections::BinaryHeap<i64>,
    intersect_nodes: Vec<IntersectNode>,
    horz_seg_list: Vec<HorzSegment>,
    horz_join_list: Vec<HorzJoin>,
    pub preserve_collinear: bool,
    pub reverse_solution: bool,
    has_open_paths: bool,
    succeeded: bool,
    outrec_list: Vec<OutRec>,
    // arenas
    outpts: Vec<OutPt>,
    actives_arena: Vec<Active>,
    tree: Option<TreeBuilder>,
}

impl Default for Clipper64 {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// free helpers

#[inline]
fn get_dx(pt1: Point64, pt2: Point64) -> f64 {
    let dy = pt2.y.wrapping_sub(pt1.y) as f64;
    if dy != 0.0 {
        pt2.x.wrapping_sub(pt1.x) as f64 / dy
    } else if pt2.x > pt1.x {
        -f64::MAX
    } else {
        f64::MAX
    }
}

#[inline]
fn top_x(ae: &Active, current_y: i64) -> i64 {
    if current_y == ae.top.y || ae.top.x == ae.bot.x {
        ae.top.x
    } else if current_y == ae.bot.y {
        ae.bot.x
    } else {
        ae.bot.x.wrapping_add(trunc_i64(nearbyint(
            ae.dx * current_y.wrapping_sub(ae.bot.y) as f64,
        )))
    }
}

#[inline]
fn is_horizontal(e: &Active) -> bool {
    e.top.y == e.bot.y
}

#[inline]
fn is_heading_right_horz(e: &Active) -> bool {
    e.dx == -f64::MAX
}

#[inline]
fn is_heading_left_horz(e: &Active) -> bool {
    e.dx == f64::MAX
}

#[inline]
fn pts_really_close(pt1: Point64, pt2: Point64) -> bool {
    pt1.x.wrapping_sub(pt2.x).wrapping_abs() < 2 && pt1.y.wrapping_sub(pt2.y).wrapping_abs() < 2
}

#[inline]
fn area_triangle(pt1: Point64, pt2: Point64, pt3: Point64) -> f64 {
    let a1 = pt3.y.wrapping_add(pt1.y) as f64;
    let b1 = pt3.x.wrapping_sub(pt1.x) as f64;
    let a2 = pt1.y.wrapping_add(pt2.y) as f64;
    let b2 = pt1.x.wrapping_sub(pt2.x) as f64;
    let a3 = pt2.y.wrapping_add(pt3.y) as f64;
    let b3 = pt2.x.wrapping_sub(pt3.x) as f64;
    fma(a3, b3, fma(a1, b1, a2 * b2))
}

fn intersect_list_sort(a: &IntersectNode, b: &IntersectNode) -> bool {
    if a.pt.y == b.pt.y {
        a.pt.x < b.pt.x
    } else {
        a.pt.y > b.pt.y
    }
}

impl Clipper64 {
    pub fn new() -> Self {
        Clipper64 {
            cliptype: ClipType::NoClip,
            fillrule: FillRule::EvenOdd,
            bot_y: 0,
            minima_list_sorted: false,
            using_polytree: false,
            actives: NIL,
            sel: NIL,
            minima_list: Vec::new(),
            current_locmin_iter: 0,
            vertices: Vec::new(),
            scanline_list: std::collections::BinaryHeap::new(),
            intersect_nodes: Vec::new(),
            horz_seg_list: Vec::new(),
            horz_join_list: Vec::new(),
            preserve_collinear: true,
            reverse_solution: false,
            has_open_paths: false,
            succeeded: true,
            outrec_list: Vec::new(),
            outpts: Vec::new(),
            actives_arena: Vec::new(),
            tree: None,
        }
    }

    /// `succeeded_` after the last execution.
    pub fn succeeded(&self) -> bool {
        self.succeeded
    }

    // ---- small accessors ------------------------------------------------

    #[inline]
    fn ae(&self, i: usize) -> &Active {
        &self.actives_arena[i]
    }
    #[inline]
    fn ae_mut(&mut self, i: usize) -> &mut Active {
        &mut self.actives_arena[i]
    }
    #[inline]
    fn op(&self, i: usize) -> &OutPt {
        &self.outpts[i]
    }
    #[inline]
    fn op_mut(&mut self, i: usize) -> &mut OutPt {
        &mut self.outpts[i]
    }
    #[inline]
    fn or(&self, i: usize) -> &OutRec {
        &self.outrec_list[i]
    }
    #[inline]
    fn or_mut(&mut self, i: usize) -> &mut OutRec {
        &mut self.outrec_list[i]
    }

    #[inline]
    fn is_hot_edge(&self, e: usize) -> bool {
        self.ae(e).outrec != NIL
    }
    #[inline]
    fn is_open(&self, e: usize) -> bool {
        self.minima_list[self.ae(e).local_min].is_open
    }
    #[inline]
    fn is_open_end_v(&self, v: usize) -> bool {
        self.vertices[v].flags & (VF_OPEN_START | VF_OPEN_END) != 0
    }
    #[inline]
    fn is_open_end(&self, e: usize) -> bool {
        self.is_open_end_v(self.ae(e).vertex_top)
    }
    #[inline]
    fn get_poly_type(&self, e: usize) -> PathType {
        self.minima_list[self.ae(e).local_min].polytype
    }
    #[inline]
    fn is_same_poly_type(&self, e1: usize, e2: usize) -> bool {
        self.get_poly_type(e1) == self.get_poly_type(e2)
    }
    #[inline]
    fn is_front(&self, e: usize) -> bool {
        e == self.or(self.ae(e).outrec).front_edge
    }
    #[inline]
    fn is_joined(&self, e: usize) -> bool {
        self.ae(e).join_with != JoinWith::NoJoin
    }
    #[inline]
    fn is_maxima_v(&self, v: usize) -> bool {
        self.vertices[v].flags & VF_LOCAL_MAX != 0
    }
    #[inline]
    fn is_maxima(&self, e: usize) -> bool {
        self.is_maxima_v(self.ae(e).vertex_top)
    }
    #[inline]
    fn set_dx(&mut self, e: usize) {
        let a = self.ae(e);
        let dx = get_dx(a.bot, a.top);
        self.ae_mut(e).dx = dx;
    }
    #[inline]
    fn next_vertex(&self, e: usize) -> usize {
        let a = self.ae(e);
        if a.wind_dx > 0 {
            self.vertices[a.vertex_top].next
        } else {
            self.vertices[a.vertex_top].prev
        }
    }
    #[inline]
    fn prev_prev_vertex(&self, e: usize) -> usize {
        let a = self.ae(e);
        let vt = a.vertex_top;
        if a.wind_dx > 0 {
            self.vertices[self.vertices[vt].prev].prev
        } else {
            self.vertices[self.vertices[vt].next].next
        }
    }
    fn get_prev_hot_edge(&self, e: usize) -> usize {
        let mut prev = self.ae(e).prev_in_ael;
        while prev != NIL && (self.is_open(prev) || !self.is_hot_edge(prev)) {
            prev = self.ae(prev).prev_in_ael;
        }
        prev
    }

    fn extract_from_sel(&mut self, ae: usize) -> usize {
        let res = self.ae(ae).next_in_sel;
        let prev = self.ae(ae).prev_in_sel;
        if res != NIL {
            self.ae_mut(res).prev_in_sel = prev;
        }
        self.ae_mut(prev).next_in_sel = res;
        res
    }

    fn insert1_before2_in_sel(&mut self, ae1: usize, ae2: usize) {
        let p = self.ae(ae2).prev_in_sel;
        self.ae_mut(ae1).prev_in_sel = p;
        if p != NIL {
            self.ae_mut(p).next_in_sel = ae1;
        }
        self.ae_mut(ae1).next_in_sel = ae2;
        self.ae_mut(ae2).prev_in_sel = ae1;
    }

    fn get_curr_y_maxima_vertex_open(&self, e: usize) -> usize {
        let a = self.ae(e);
        let mut result = a.vertex_top;
        let v = &self.vertices;
        if a.wind_dx > 0 {
            while v[v[result].next].pt.y == v[result].pt.y
                && v[result].flags & (VF_OPEN_END | VF_LOCAL_MAX) == 0
            {
                result = v[result].next;
            }
        } else {
            while v[v[result].prev].pt.y == v[result].pt.y
                && v[result].flags & (VF_OPEN_END | VF_LOCAL_MAX) == 0
            {
                result = v[result].prev;
            }
        }
        if !self.is_maxima_v(result) {
            result = NIL;
        }
        result
    }

    fn get_curr_y_maxima_vertex(&self, e: usize) -> usize {
        let a = self.ae(e);
        let mut result = a.vertex_top;
        let v = &self.vertices;
        if a.wind_dx > 0 {
            while v[v[result].next].pt.y == v[result].pt.y {
                result = v[result].next;
            }
        } else {
            while v[v[result].prev].pt.y == v[result].pt.y {
                result = v[result].prev;
            }
        }
        if !self.is_maxima_v(result) {
            result = NIL;
        }
        result
    }

    fn get_maxima_pair(&self, e: usize) -> usize {
        let mut e2 = self.ae(e).next_in_ael;
        while e2 != NIL {
            if self.ae(e2).vertex_top == self.ae(e).vertex_top {
                return e2;
            }
            e2 = self.ae(e2).next_in_ael;
        }
        NIL
    }

    fn new_outpt(&mut self, pt: Point64, outrec: usize) -> usize {
        let id = self.outpts.len();
        self.outpts.push(OutPt {
            pt,
            next: id,
            prev: id,
            outrec,
            horz: false,
        });
        id
    }

    fn duplicate_op(&mut self, op: usize, insert_after: bool) -> usize {
        let (pt, outrec) = (self.op(op).pt, self.op(op).outrec);
        let result = self.new_outpt(pt, outrec);
        if insert_after {
            let n = self.op(op).next;
            self.op_mut(result).next = n;
            self.op_mut(n).prev = result;
            self.op_mut(result).prev = op;
            self.op_mut(op).next = result;
        } else {
            let p = self.op(op).prev;
            self.op_mut(result).prev = p;
            self.op_mut(p).next = result;
            self.op_mut(result).next = op;
            self.op_mut(op).prev = result;
        }
        result
    }

    fn dispose_out_pt(&mut self, op: usize) -> usize {
        let result = self.op(op).next;
        let (p, n) = (self.op(op).prev, self.op(op).next);
        self.op_mut(p).next = n;
        self.op_mut(n).prev = p;
        result
    }

    fn dispose_out_pts(&mut self, outrec: usize) {
        self.or_mut(outrec).pts = NIL;
    }

    #[inline]
    fn set_sides(&mut self, outrec: usize, start_edge: usize, end_edge: usize) {
        let o = self.or_mut(outrec);
        o.front_edge = start_edge;
        o.back_edge = end_edge;
    }

    fn swap_outrecs(&mut self, e1: usize, e2: usize) {
        let or1 = self.ae(e1).outrec;
        let or2 = self.ae(e2).outrec;
        if or1 == or2 {
            let o = self.or_mut(or1);
            std::mem::swap(&mut o.front_edge, &mut o.back_edge);
            return;
        }
        if or1 != NIL {
            if e1 == self.or(or1).front_edge {
                self.or_mut(or1).front_edge = e2;
            } else {
                self.or_mut(or1).back_edge = e2;
            }
        }
        if or2 != NIL {
            if e2 == self.or(or2).front_edge {
                self.or_mut(or2).front_edge = e1;
            } else {
                self.or_mut(or2).back_edge = e1;
            }
        }
        self.ae_mut(e1).outrec = or2;
        self.ae_mut(e2).outrec = or1;
    }

    fn area_op(&self, op: usize) -> f64 {
        let mut result = 0.0f64;
        let mut op2 = op;
        loop {
            let p = self.op(self.op(op2).prev).pt;
            let c = self.op(op2).pt;
            result = fma(
                p.y.wrapping_add(c.y) as f64,
                p.x.wrapping_sub(c.x) as f64,
                result,
            );
            op2 = self.op(op2).next;
            if op2 == op {
                break;
            }
        }
        result * 0.5
    }

    fn swap_front_back_sides(&mut self, outrec: usize) {
        let pts = self.or(outrec).pts;
        let next = self.op(pts).next;
        let o = self.or_mut(outrec);
        std::mem::swap(&mut o.front_edge, &mut o.back_edge);
        o.pts = next;
    }

    #[inline]
    fn get_real_outrec(&self, mut outrec: usize) -> usize {
        while outrec != NIL && self.or(outrec).pts == NIL {
            outrec = self.or(outrec).owner;
        }
        outrec
    }

    #[inline]
    fn is_valid_owner(&self, outrec: usize, mut test_owner: usize) -> bool {
        while test_owner != NIL && test_owner != outrec {
            test_owner = self.or(test_owner).owner;
        }
        test_owner == NIL
    }

    fn uncouple_outrec(&mut self, ae: usize) {
        let outrec = self.ae(ae).outrec;
        if outrec == NIL {
            return;
        }
        let (fe, be) = (self.or(outrec).front_edge, self.or(outrec).back_edge);
        self.ae_mut(fe).outrec = NIL;
        self.ae_mut(be).outrec = NIL;
        self.or_mut(outrec).front_edge = NIL;
        self.or_mut(outrec).back_edge = NIL;
    }

    fn is_very_small_triangle(&self, op: usize) -> bool {
        let o = self.op(op);
        let next = self.op(o.next);
        let prev = self.op(o.prev);
        next.next == o.prev
            && (pts_really_close(prev.pt, next.pt)
                || pts_really_close(o.pt, next.pt)
                || pts_really_close(o.pt, prev.pt))
    }

    fn is_valid_closed_path(&self, op: usize) -> bool {
        op != NIL
            && self.op(op).next != op
            && self.op(op).next != self.op(op).prev
            && !self.is_very_small_triangle(op)
    }

    #[inline]
    fn outrec_is_ascending(&self, hot_edge: usize) -> bool {
        hot_edge == self.or(self.ae(hot_edge).outrec).front_edge
    }

    fn set_owner(&mut self, outrec: usize, new_owner: usize) {
        let o = self.get_real_outrec(self.or(new_owner).owner);
        self.or_mut(new_owner).owner = o;
        let mut tmp = new_owner;
        while tmp != NIL && tmp != outrec {
            tmp = self.or(tmp).owner;
        }
        if tmp != NIL {
            let oo = self.or(outrec).owner;
            self.or_mut(new_owner).owner = oo;
        }
        self.or_mut(outrec).owner = new_owner;
    }

    fn point_in_op_polygon(&self, pt: Point64, mut op: usize) -> PointInPolygonResult {
        use PointInPolygonResult::*;
        if op == self.op(op).next || self.op(op).prev == self.op(op).next {
            return IsOutside;
        }
        let mut op2 = op;
        loop {
            if self.op(op).pt.y != pt.y {
                break;
            }
            op = self.op(op).next;
            if op == op2 {
                break;
            }
        }
        if self.op(op).pt.y == pt.y {
            return IsOutside;
        }
        let mut is_above = self.op(op).pt.y < pt.y;
        let starting_above = is_above;
        let mut val = 0;
        op2 = self.op(op).next;
        while op2 != op {
            if is_above {
                while op2 != op && self.op(op2).pt.y < pt.y {
                    op2 = self.op(op2).next;
                }
            } else {
                while op2 != op && self.op(op2).pt.y > pt.y {
                    op2 = self.op(op2).next;
                }
            }
            if op2 == op {
                break;
            }
            let o2 = self.op(op2).pt;
            let p2 = self.op(self.op(op2).prev).pt;
            if o2.y == pt.y {
                if o2.x == pt.x || (o2.y == p2.y && (pt.x < p2.x) != (pt.x < o2.x)) {
                    return IsOn;
                }
                op2 = self.op(op2).next;
                if op2 == op {
                    break;
                }
                continue;
            }
            if pt.x < o2.x && pt.x < p2.x {
                // only interested in edges crossing on the left
            } else if pt.x > p2.x && pt.x > o2.x {
                val = 1 - val;
            } else {
                let i = cross_product_sign(p2, o2, pt);
                if i == 0 {
                    return IsOn;
                }
                if (i < 0) == is_above {
                    val = 1 - val;
                }
            }
            is_above = !is_above;
            op2 = self.op(op2).next;
        }
        if is_above != starting_above {
            let i = cross_product_sign(self.op(self.op(op2).prev).pt, self.op(op2).pt, pt);
            if i == 0 {
                return IsOn;
            }
            if (i < 0) == is_above {
                val = 1 - val;
            }
        }
        if val == 0 {
            IsOutside
        } else {
            IsInside
        }
    }

    fn get_clean_path(&self, op: usize) -> Path64 {
        let mut result = Path64::new();
        let mut op2 = op;
        loop {
            let o = self.op(op2);
            let n = self.op(o.next).pt;
            let p = self.op(o.prev).pt;
            if o.next != op
                && ((o.pt.x == n.x && o.pt.x == p.x) || (o.pt.y == n.y && o.pt.y == p.y))
            {
                op2 = o.next;
            } else {
                break;
            }
        }
        result.push(self.op(op2).pt);
        let mut prev_op = op2;
        op2 = self.op(op2).next;
        while op2 != op {
            let o = self.op(op2);
            let n = self.op(o.next).pt;
            let pp = self.op(prev_op).pt;
            if (o.pt.x != n.x || o.pt.x != pp.x) && (o.pt.y != n.y || o.pt.y != pp.y) {
                result.push(o.pt);
                prev_op = op2;
            }
            op2 = self.op(op2).next;
        }
        result
    }

    fn path2_contains_path1_op(&self, op1: usize, op2: usize) -> bool {
        use PointInPolygonResult::*;
        let mut pip = IsOn;
        let mut op = op1;
        loop {
            match self.point_in_op_polygon(self.op(op).pt, op2) {
                IsOutside => {
                    if pip == IsOutside {
                        return false;
                    }
                    pip = IsOutside;
                }
                IsInside => {
                    if pip == IsInside {
                        return true;
                    }
                    pip = IsInside;
                }
                IsOn => {}
            }
            op = self.op(op).next;
            if op == op1 {
                break;
            }
        }
        path2_contains_path1(&self.get_clean_path(op1), &self.get_clean_path(op2))
    }

    // ---- adding paths ---------------------------------------------------

    fn add_loc_min(&mut self, vert: usize, polytype: PathType, is_open: bool) {
        if self.vertices[vert].flags & VF_LOCAL_MIN != 0 {
            return;
        }
        self.vertices[vert].flags |= VF_LOCAL_MIN;
        self.minima_list.push(LocalMinima {
            vertex: vert,
            polytype,
            is_open,
        });
    }

    /// `AddPaths_`.
    fn add_paths_internal(&mut self, paths: &[Path64], polytype: PathType, is_open: bool) {
        let total_vertex_count: usize = paths.iter().map(|p| p.len()).sum();
        if total_vertex_count == 0 {
            return;
        }
        let base = self.vertices.len();
        self.vertices.resize(
            base + total_vertex_count,
            Vertex {
                pt: Point64::default(),
                next: NIL,
                prev: NIL,
                flags: 0,
            },
        );
        let mut v = base;
        for path in paths {
            let v0 = v;
            let mut curr_v = v;
            let mut prev_v = NIL;
            if path.is_empty() {
                continue;
            }
            self.vertices[v].prev = NIL;
            let mut cnt = 0;
            for &pt in path {
                if prev_v != NIL {
                    if self.vertices[prev_v].pt == pt {
                        continue;
                    }
                    self.vertices[prev_v].next = curr_v;
                }
                self.vertices[curr_v].prev = prev_v;
                self.vertices[curr_v].pt = pt;
                self.vertices[curr_v].flags = 0;
                prev_v = curr_v;
                curr_v += 1;
                cnt += 1;
            }
            if prev_v == NIL || self.vertices[prev_v].prev == NIL {
                continue;
            }
            if !is_open && self.vertices[prev_v].pt == self.vertices[v0].pt {
                prev_v = self.vertices[prev_v].prev;
            }
            self.vertices[prev_v].next = v0;
            self.vertices[v0].prev = prev_v;
            v = curr_v;
            if cnt < 2 || (cnt == 2 && !is_open) {
                continue;
            }
            let mut going_up;
            if is_open {
                curr_v = self.vertices[v0].next;
                while curr_v != v0 && self.vertices[curr_v].pt.y == self.vertices[v0].pt.y {
                    curr_v = self.vertices[curr_v].next;
                }
                going_up = self.vertices[curr_v].pt.y <= self.vertices[v0].pt.y;
                if going_up {
                    self.vertices[v0].flags = VF_OPEN_START;
                    self.add_loc_min(v0, polytype, true);
                } else {
                    self.vertices[v0].flags = VF_OPEN_START | VF_LOCAL_MAX;
                }
            } else {
                prev_v = self.vertices[v0].prev;
                while prev_v != v0 && self.vertices[prev_v].pt.y == self.vertices[v0].pt.y {
                    prev_v = self.vertices[prev_v].prev;
                }
                if prev_v == v0 {
                    continue;
                }
                going_up = self.vertices[prev_v].pt.y > self.vertices[v0].pt.y;
            }
            let going_up0 = going_up;
            prev_v = v0;
            curr_v = self.vertices[v0].next;
            while curr_v != v0 {
                if self.vertices[curr_v].pt.y > self.vertices[prev_v].pt.y && going_up {
                    self.vertices[prev_v].flags |= VF_LOCAL_MAX;
                    going_up = false;
                } else if self.vertices[curr_v].pt.y < self.vertices[prev_v].pt.y && !going_up {
                    going_up = true;
                    self.add_loc_min(prev_v, polytype, is_open);
                }
                prev_v = curr_v;
                curr_v = self.vertices[curr_v].next;
            }
            if is_open {
                self.vertices[prev_v].flags |= VF_OPEN_END;
                if going_up {
                    self.vertices[prev_v].flags |= VF_LOCAL_MAX;
                } else {
                    self.add_loc_min(prev_v, polytype, is_open);
                }
            } else if going_up != going_up0 {
                if going_up0 {
                    self.add_loc_min(prev_v, polytype, false);
                } else {
                    self.vertices[prev_v].flags |= VF_LOCAL_MAX;
                }
            }
        }
    }

    /// `ClipperBase::AddPaths`.
    pub fn add_paths(&mut self, paths: &[Path64], polytype: PathType, is_open: bool) {
        if is_open {
            self.has_open_paths = true;
        }
        self.minima_list_sorted = false;
        self.add_paths_internal(paths, polytype, is_open);
    }

    /// `ClipperBase::AddPath`.
    pub fn add_path(&mut self, path: &[Point64], polytype: PathType, is_open: bool) {
        self.add_paths(&[path.to_vec()], polytype, is_open);
    }

    /// `ClipperBase::CleanUp` (keeps added paths).
    pub fn clean_up(&mut self) {
        self.actives = NIL;
        self.actives_arena.clear();
        self.scanline_list.clear();
        self.intersect_nodes.clear();
        self.outrec_list.clear();
        self.outpts.clear();
        self.horz_seg_list.clear();
        self.horz_join_list.clear();
    }

    /// `ClipperBase::Clear`.
    pub fn clear(&mut self) {
        self.clean_up();
        self.minima_list.clear();
        self.vertices.clear();
        self.current_locmin_iter = 0;
        self.minima_list_sorted = false;
        self.has_open_paths = false;
    }

    fn reset(&mut self) {
        if !self.minima_list_sorted {
            // LocMinSorter, std::stable_sort
            let verts = &self.vertices;
            self.minima_list.sort_by(|m1, m2| {
                let less = |a: &LocalMinima, b: &LocalMinima| {
                    let (pa, pb) = (verts[a.vertex].pt, verts[b.vertex].pt);
                    if pb.y != pa.y {
                        pb.y < pa.y
                    } else {
                        pb.x > pa.x
                    }
                };
                if less(m1, m2) {
                    std::cmp::Ordering::Less
                } else if less(m2, m1) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            });
            self.minima_list_sorted = true;
        }
        for i in (0..self.minima_list.len()).rev() {
            let y = self.vertices[self.minima_list[i].vertex].pt.y;
            self.insert_scanline(y);
        }
        self.current_locmin_iter = 0;
        self.actives = NIL;
        self.sel = NIL;
        self.succeeded = true;
    }

    #[inline]
    fn insert_scanline(&mut self, y: i64) {
        self.scanline_list.push(y);
    }

    fn pop_scanline(&mut self) -> Option<i64> {
        let y = self.scanline_list.pop()?;
        while self.scanline_list.peek() == Some(&y) {
            self.scanline_list.pop();
        }
        Some(y)
    }

    fn pop_local_minima(&mut self, y: i64) -> Option<usize> {
        if self.current_locmin_iter == self.minima_list.len()
            || self.vertices[self.minima_list[self.current_locmin_iter].vertex]
                .pt
                .y
                != y
        {
            return None;
        }
        let r = self.current_locmin_iter;
        self.current_locmin_iter += 1;
        Some(r)
    }

    // ---- winding --------------------------------------------------------

    fn is_contributing_closed(&self, e: usize) -> bool {
        let a = self.ae(e);
        match self.fillrule {
            FillRule::EvenOdd => {}
            FillRule::NonZero => {
                if a.wind_cnt.wrapping_abs() != 1 {
                    return false;
                }
            }
            FillRule::Positive => {
                if a.wind_cnt != 1 {
                    return false;
                }
            }
            FillRule::Negative => {
                if a.wind_cnt != -1 {
                    return false;
                }
            }
        }
        match self.cliptype {
            ClipType::NoClip => false,
            ClipType::Intersection => match self.fillrule {
                FillRule::Positive => a.wind_cnt2 > 0,
                FillRule::Negative => a.wind_cnt2 < 0,
                _ => a.wind_cnt2 != 0,
            },
            ClipType::Union => match self.fillrule {
                FillRule::Positive => a.wind_cnt2 <= 0,
                FillRule::Negative => a.wind_cnt2 >= 0,
                _ => a.wind_cnt2 == 0,
            },
            ClipType::Difference => {
                let result = match self.fillrule {
                    FillRule::Positive => a.wind_cnt2 <= 0,
                    FillRule::Negative => a.wind_cnt2 >= 0,
                    _ => a.wind_cnt2 == 0,
                };
                if self.get_poly_type(e) == PathType::Subject {
                    result
                } else {
                    !result
                }
            }
            ClipType::Xor => true,
            ClipType::Other(_) => false,
        }
    }

    fn is_contributing_open(&self, e: usize) -> bool {
        let a = self.ae(e);
        let (is_in_clip, is_in_subj) = match self.fillrule {
            FillRule::Positive => (a.wind_cnt2 > 0, a.wind_cnt > 0),
            FillRule::Negative => (a.wind_cnt2 < 0, a.wind_cnt < 0),
            _ => (a.wind_cnt2 != 0, a.wind_cnt != 0),
        };
        match self.cliptype {
            ClipType::Intersection => is_in_clip,
            ClipType::Union => !is_in_subj && !is_in_clip,
            _ => !is_in_clip,
        }
    }

    fn set_wind_count_for_closed_path_edge(&mut self, e: usize) {
        let mut e2 = self.ae(e).prev_in_ael;
        let pt = self.get_poly_type(e);
        while e2 != NIL && (self.get_poly_type(e2) != pt || self.is_open(e2)) {
            e2 = self.ae(e2).prev_in_ael;
        }
        if e2 == NIL {
            let wdx = self.ae(e).wind_dx;
            self.ae_mut(e).wind_cnt = wdx;
            e2 = self.actives;
        } else if self.fillrule == FillRule::EvenOdd {
            let wdx = self.ae(e).wind_dx;
            let wc2 = self.ae(e2).wind_cnt2;
            self.ae_mut(e).wind_cnt = wdx;
            self.ae_mut(e).wind_cnt2 = wc2;
            e2 = self.ae(e2).next_in_ael;
        } else {
            let a2 = *self.ae(e2);
            let wdx = self.ae(e).wind_dx;
            let new_wc = if a2.wind_cnt.wrapping_mul(a2.wind_dx) < 0 {
                if a2.wind_cnt.wrapping_abs() > 1 {
                    if a2.wind_dx.wrapping_mul(wdx) < 0 {
                        a2.wind_cnt
                    } else {
                        a2.wind_cnt.wrapping_add(wdx)
                    }
                } else if self.is_open(e) {
                    1
                } else {
                    wdx
                }
            } else if a2.wind_dx.wrapping_mul(wdx) < 0 {
                a2.wind_cnt
            } else {
                a2.wind_cnt.wrapping_add(wdx)
            };
            self.ae_mut(e).wind_cnt = new_wc;
            self.ae_mut(e).wind_cnt2 = a2.wind_cnt2;
            e2 = a2.next_in_ael;
        }
        if self.fillrule == FillRule::EvenOdd {
            while e2 != e {
                if self.get_poly_type(e2) != pt && !self.is_open(e2) {
                    let w = self.ae(e).wind_cnt2;
                    self.ae_mut(e).wind_cnt2 = if w == 0 { 1 } else { 0 };
                }
                e2 = self.ae(e2).next_in_ael;
            }
        } else {
            while e2 != e {
                if self.get_poly_type(e2) != pt && !self.is_open(e2) {
                    let d = self.ae(e2).wind_dx;
                    let w = self.ae(e).wind_cnt2;
                    self.ae_mut(e).wind_cnt2 = w.wrapping_add(d);
                }
                e2 = self.ae(e2).next_in_ael;
            }
        }
    }

    fn set_wind_count_for_open_path_edge(&mut self, e: usize) {
        let mut e2 = self.actives;
        if self.fillrule == FillRule::EvenOdd {
            let mut cnt1 = 0i32;
            let mut cnt2 = 0i32;
            while e2 != e {
                if self.get_poly_type(e2) == PathType::Clip {
                    cnt2 += 1;
                } else if !self.is_open(e2) {
                    cnt1 += 1;
                }
                e2 = self.ae(e2).next_in_ael;
            }
            self.ae_mut(e).wind_cnt = if cnt1 & 1 != 0 { 1 } else { 0 };
            self.ae_mut(e).wind_cnt2 = if cnt2 & 1 != 0 { 1 } else { 0 };
        } else {
            while e2 != e {
                let d = self.ae(e2).wind_dx;
                if self.get_poly_type(e2) == PathType::Clip {
                    let w = self.ae(e).wind_cnt2;
                    self.ae_mut(e).wind_cnt2 = w.wrapping_add(d);
                } else if !self.is_open(e2) {
                    let w = self.ae(e).wind_cnt;
                    self.ae_mut(e).wind_cnt = w.wrapping_add(d);
                }
                e2 = self.ae(e2).next_in_ael;
            }
        }
    }

    fn is_valid_ael_order(&self, resident: usize, newcomer: usize) -> bool {
        let r = *self.ae(resident);
        let n = *self.ae(newcomer);
        if n.curr_x != r.curr_x {
            return n.curr_x > r.curr_x;
        }
        let i = cross_product_sign(r.top, n.bot, n.top);
        if i != 0 {
            return i < 0;
        }
        if !self.is_maxima(resident) && r.top.y > n.top.y {
            return cross_product_sign(n.bot, r.top, self.vertices[self.next_vertex(resident)].pt)
                <= 0;
        } else if !self.is_maxima(newcomer) && n.top.y > r.top.y {
            return cross_product_sign(n.bot, n.top, self.vertices[self.next_vertex(newcomer)].pt)
                >= 0;
        }
        let y = n.bot.y;
        let newcomer_is_left = n.is_left_bound;
        if r.bot.y != y || self.vertices[self.minima_list[r.local_min].vertex].pt.y != y {
            n.is_left_bound
        } else if r.is_left_bound != newcomer_is_left {
            newcomer_is_left
        } else if is_collinear(
            self.vertices[self.prev_prev_vertex(resident)].pt,
            r.bot,
            r.top,
        ) {
            true
        } else {
            (cross_product_sign(
                self.vertices[self.prev_prev_vertex(resident)].pt,
                n.bot,
                self.vertices[self.prev_prev_vertex(newcomer)].pt,
            ) > 0)
                == newcomer_is_left
        }
    }

    fn insert_left_edge(&mut self, e: usize) {
        if self.actives == NIL {
            self.ae_mut(e).prev_in_ael = NIL;
            self.ae_mut(e).next_in_ael = NIL;
            self.actives = e;
        } else if !self.is_valid_ael_order(self.actives, e) {
            let a = self.actives;
            self.ae_mut(e).prev_in_ael = NIL;
            self.ae_mut(e).next_in_ael = a;
            self.ae_mut(a).prev_in_ael = e;
            self.actives = e;
        } else {
            let mut e2 = self.actives;
            while self.ae(e2).next_in_ael != NIL
                && self.is_valid_ael_order(self.ae(e2).next_in_ael, e)
            {
                e2 = self.ae(e2).next_in_ael;
            }
            if self.ae(e2).join_with == JoinWith::Right {
                e2 = self.ae(e2).next_in_ael;
            }
            if e2 == NIL {
                return;
            }
            let n = self.ae(e2).next_in_ael;
            self.ae_mut(e).next_in_ael = n;
            if n != NIL {
                self.ae_mut(n).prev_in_ael = e;
            }
            self.ae_mut(e).prev_in_ael = e2;
            self.ae_mut(e2).next_in_ael = e;
        }
    }

    fn insert_right_edge(&mut self, e: usize, e2: usize) {
        let n = self.ae(e).next_in_ael;
        self.ae_mut(e2).next_in_ael = n;
        if n != NIL {
            self.ae_mut(n).prev_in_ael = e2;
        }
        self.ae_mut(e2).prev_in_ael = e;
        self.ae_mut(e).next_in_ael = e2;
    }

    fn new_active(&mut self, a: Active) -> usize {
        let id = self.actives_arena.len();
        self.actives_arena.push(a);
        id
    }

    fn insert_local_minima_into_ael(&mut self, bot_y: i64) {
        while let Some(lm) = self.pop_local_minima(bot_y) {
            let lmv = self.minima_list[lm].vertex;
            let vflags = self.vertices[lmv].flags;
            let mut left_bound = if vflags & VF_OPEN_START != 0 {
                NIL
            } else {
                let bot = self.vertices[lmv].pt;
                let vt = self.vertices[lmv].prev;
                let a = Active {
                    bot,
                    curr_x: bot.x,
                    wind_dx: -1,
                    vertex_top: vt,
                    top: self.vertices[vt].pt,
                    local_min: lm,
                    ..Active::default()
                };
                let id = self.new_active(a);
                self.set_dx(id);
                id
            };
            let mut right_bound = if vflags & VF_OPEN_END != 0 {
                NIL
            } else {
                let bot = self.vertices[lmv].pt;
                let vt = self.vertices[lmv].next;
                let a = Active {
                    bot,
                    curr_x: bot.x,
                    wind_dx: 1,
                    vertex_top: vt,
                    top: self.vertices[vt].pt,
                    local_min: lm,
                    ..Active::default()
                };
                let id = self.new_active(a);
                self.set_dx(id);
                id
            };

            if left_bound != NIL && right_bound != NIL {
                if is_horizontal(self.ae(left_bound)) {
                    if is_heading_right_horz(self.ae(left_bound)) {
                        std::mem::swap(&mut left_bound, &mut right_bound);
                    }
                } else if is_horizontal(self.ae(right_bound)) {
                    if is_heading_left_horz(self.ae(right_bound)) {
                        std::mem::swap(&mut left_bound, &mut right_bound);
                    }
                } else if self.ae(left_bound).dx < self.ae(right_bound).dx {
                    std::mem::swap(&mut left_bound, &mut right_bound);
                }
            } else if left_bound == NIL {
                left_bound = right_bound;
                right_bound = NIL;
            }

            self.ae_mut(left_bound).is_left_bound = true;
            self.insert_left_edge(left_bound);

            let contributing = if self.is_open(left_bound) {
                self.set_wind_count_for_open_path_edge(left_bound);
                self.is_contributing_open(left_bound)
            } else {
                self.set_wind_count_for_closed_path_edge(left_bound);
                self.is_contributing_closed(left_bound)
            };

            if right_bound != NIL {
                let (wc, wc2) = (self.ae(left_bound).wind_cnt, self.ae(left_bound).wind_cnt2);
                {
                    let r = self.ae_mut(right_bound);
                    r.is_left_bound = false;
                    r.wind_cnt = wc;
                    r.wind_cnt2 = wc2;
                }
                self.insert_right_edge(left_bound, right_bound);
                if contributing {
                    let bot = self.ae(left_bound).bot;
                    self.add_local_min_poly(left_bound, right_bound, bot, true);
                    if !is_horizontal(self.ae(left_bound)) {
                        self.check_join_left(left_bound, bot, false);
                    }
                }
                while self.ae(right_bound).next_in_ael != NIL
                    && self.is_valid_ael_order(self.ae(right_bound).next_in_ael, right_bound)
                {
                    let n = self.ae(right_bound).next_in_ael;
                    let bot = self.ae(right_bound).bot;
                    self.intersect_edges(right_bound, n, bot);
                    let n = self.ae(right_bound).next_in_ael;
                    self.swap_positions_in_ael(right_bound, n);
                }
                if is_horizontal(self.ae(right_bound)) {
                    self.push_horz(right_bound);
                } else {
                    let bot = self.ae(right_bound).bot;
                    self.check_join_right(right_bound, bot, false);
                    let ty = self.ae(right_bound).top.y;
                    self.insert_scanline(ty);
                }
            } else if contributing {
                let bot = self.ae(left_bound).bot;
                self.start_open_path(left_bound, bot);
            }

            if is_horizontal(self.ae(left_bound)) {
                self.push_horz(left_bound);
            } else {
                let ty = self.ae(left_bound).top.y;
                self.insert_scanline(ty);
            }
        }
    }

    #[inline]
    fn push_horz(&mut self, e: usize) {
        let s = self.sel;
        self.ae_mut(e).next_in_sel = s;
        self.sel = e;
    }

    #[inline]
    fn pop_horz(&mut self) -> Option<usize> {
        let e = self.sel;
        if e == NIL {
            return None;
        }
        self.sel = self.ae(e).next_in_sel;
        Some(e)
    }

    // ---- output construction --------------------------------------------

    fn add_local_min_poly(&mut self, e1: usize, e2: usize, pt: Point64, is_new: bool) -> usize {
        let outrec = self.new_outrec();
        self.ae_mut(e1).outrec = outrec;
        self.ae_mut(e2).outrec = outrec;
        if self.is_open(e1) {
            self.or_mut(outrec).owner = NIL;
            self.or_mut(outrec).is_open = true;
            if self.ae(e1).wind_dx > 0 {
                self.set_sides(outrec, e1, e2);
            } else {
                self.set_sides(outrec, e2, e1);
            }
        } else {
            let prev_hot_edge = self.get_prev_hot_edge(e1);
            if prev_hot_edge != NIL {
                if self.using_polytree {
                    let pho = self.ae(prev_hot_edge).outrec;
                    self.set_owner(outrec, pho);
                }
                if self.outrec_is_ascending(prev_hot_edge) == is_new {
                    self.set_sides(outrec, e2, e1);
                } else {
                    self.set_sides(outrec, e1, e2);
                }
            } else {
                self.or_mut(outrec).owner = NIL;
                if is_new {
                    self.set_sides(outrec, e1, e2);
                } else {
                    self.set_sides(outrec, e2, e1);
                }
            }
        }
        let op = self.new_outpt(pt, outrec);
        self.or_mut(outrec).pts = op;
        op
    }

    fn add_local_max_poly(&mut self, e1: usize, e2: usize, pt: Point64) -> usize {
        if self.is_joined(e1) {
            self.split(e1, pt);
        }
        if self.is_joined(e2) {
            self.split(e2, pt);
        }
        // The reference dereferences a null `outrec` here (and crashes) when
        // one edge is not hot; this only happens with clip types outside
        // the enum. Report a failed sweep instead.
        if self.ae(e1).outrec == NIL || self.ae(e2).outrec == NIL {
            self.succeeded = false;
            return NIL;
        }
        if self.is_front(e1) == self.is_front(e2) {
            if self.is_open_end(e1) {
                let o = self.ae(e1).outrec;
                self.swap_front_back_sides(o);
            } else if self.is_open_end(e2) {
                let o = self.ae(e2).outrec;
                self.swap_front_back_sides(o);
            } else {
                self.succeeded = false;
                return NIL;
            }
        }
        let mut result = self.add_out_pt(e1, pt);
        if self.ae(e1).outrec == self.ae(e2).outrec {
            let outrec = self.ae(e1).outrec;
            self.or_mut(outrec).pts = result;
            if self.using_polytree {
                let e = self.get_prev_hot_edge(e1);
                if e == NIL {
                    self.or_mut(outrec).owner = NIL;
                } else {
                    let eo = self.ae(e).outrec;
                    self.set_owner(outrec, eo);
                }
            }
            self.uncouple_outrec(e1);
            result = self.or(outrec).pts;
            let owner = self.or(outrec).owner;
            if owner != NIL && self.or(owner).front_edge == NIL {
                let ro = self.get_real_outrec(owner);
                self.or_mut(outrec).owner = ro;
            }
        } else if self.is_open(e1) {
            if self.ae(e1).wind_dx < 0 {
                self.join_outrec_paths(e1, e2);
            } else {
                self.join_outrec_paths(e2, e1);
            }
        } else if self.or(self.ae(e1).outrec).idx < self.or(self.ae(e2).outrec).idx {
            self.join_outrec_paths(e1, e2);
        } else {
            self.join_outrec_paths(e2, e1);
        }
        result
    }

    fn join_outrec_paths(&mut self, e1: usize, e2: usize) {
        let or1 = self.ae(e1).outrec;
        let or2 = self.ae(e2).outrec;
        let p1_st = self.or(or1).pts;
        let p2_st = self.or(or2).pts;
        let p1_end = self.op(p1_st).next;
        let p2_end = self.op(p2_st).next;
        if self.is_front(e1) {
            self.op_mut(p2_end).prev = p1_st;
            self.op_mut(p1_st).next = p2_end;
            self.op_mut(p2_st).next = p1_end;
            self.op_mut(p1_end).prev = p2_st;
            self.or_mut(or1).pts = p2_st;
            let fe = self.or(or2).front_edge;
            self.or_mut(or1).front_edge = fe;
            if fe != NIL {
                self.ae_mut(fe).outrec = or1;
            }
        } else {
            self.op_mut(p1_end).prev = p2_st;
            self.op_mut(p2_st).next = p1_end;
            self.op_mut(p1_st).next = p2_end;
            self.op_mut(p2_end).prev = p1_st;
            let be = self.or(or2).back_edge;
            self.or_mut(or1).back_edge = be;
            if be != NIL {
                self.ae_mut(be).outrec = or1;
            }
        }
        self.or_mut(or2).front_edge = NIL;
        self.or_mut(or2).back_edge = NIL;
        self.or_mut(or2).pts = NIL;
        if self.is_open_end(e1) {
            let p = self.or(or1).pts;
            self.or_mut(or2).pts = p;
            self.or_mut(or1).pts = NIL;
        } else {
            self.set_owner(or2, or1);
        }
        self.ae_mut(e1).outrec = NIL;
        self.ae_mut(e2).outrec = NIL;
    }

    fn new_outrec(&mut self) -> usize {
        let idx = self.outrec_list.len();
        self.outrec_list.push(OutRec {
            idx,
            owner: NIL,
            front_edge: NIL,
            back_edge: NIL,
            pts: NIL,
            polypath: NIL,
            splits: None,
            recursive_split: NIL,
            bounds: Rect64::default(),
            path: Path64::new(),
            is_open: false,
        });
        idx
    }

    fn add_out_pt(&mut self, e: usize, pt: Point64) -> usize {
        let outrec = self.ae(e).outrec;
        let to_front = self.is_front(e);
        let op_front = self.or(outrec).pts;
        let op_back = self.op(op_front).next;
        if to_front {
            if pt == self.op(op_front).pt {
                return op_front;
            }
        } else if pt == self.op(op_back).pt {
            return op_back;
        }
        let new_op = self.new_outpt(pt, outrec);
        self.op_mut(op_back).prev = new_op;
        self.op_mut(new_op).prev = op_front;
        self.op_mut(new_op).next = op_back;
        self.op_mut(op_front).next = new_op;
        if to_front {
            self.or_mut(outrec).pts = new_op;
        }
        new_op
    }

    fn clean_collinear(&mut self, outrec: usize) {
        let outrec = self.get_real_outrec(outrec);
        if outrec == NIL || self.or(outrec).is_open {
            return;
        }
        if !self.is_valid_closed_path(self.or(outrec).pts) {
            self.dispose_out_pts(outrec);
            return;
        }
        let mut start_op = self.or(outrec).pts;
        let mut op2 = start_op;
        loop {
            let o = *self.op(op2);
            let pp = self.op(o.prev).pt;
            let np = self.op(o.next).pt;
            if is_collinear(pp, o.pt, np)
                && (o.pt == pp
                    || o.pt == np
                    || !self.preserve_collinear
                    || dot_product3(pp, o.pt, np) < 0.0)
            {
                if op2 == self.or(outrec).pts {
                    self.or_mut(outrec).pts = o.prev;
                }
                op2 = self.dispose_out_pt(op2);
                if !self.is_valid_closed_path(op2) {
                    self.dispose_out_pts(outrec);
                    return;
                }
                start_op = op2;
                continue;
            }
            op2 = self.op(op2).next;
            if op2 == start_op {
                break;
            }
        }
        self.fix_self_intersects(outrec);
    }

    fn do_split_op(&mut self, outrec: usize, split_op: usize) {
        let prev_op = self.op(split_op).prev;
        let split_next = self.op(split_op).next;
        let next_next_op = self.op(split_next).next;
        self.or_mut(outrec).pts = prev_op;

        let mut ip = Point64::default();
        get_segment_intersect_pt(
            self.op(prev_op).pt,
            self.op(split_op).pt,
            self.op(split_next).pt,
            self.op(next_next_op).pt,
            &mut ip,
        );

        let area1 = self.area_op(self.or(outrec).pts);
        let abs_area1 = area1.abs();
        if abs_area1 < 2.0 {
            self.dispose_out_pts(outrec);
            return;
        }

        let area2 = area_triangle(ip, self.op(split_op).pt, self.op(split_next).pt);
        let abs_area2 = area2.abs();

        if ip == self.op(prev_op).pt || ip == self.op(next_next_op).pt {
            self.op_mut(next_next_op).prev = prev_op;
            self.op_mut(prev_op).next = next_next_op;
        } else {
            let o = self.op(prev_op).outrec;
            let new_op2 = self.new_outpt(ip, o);
            self.op_mut(new_op2).prev = prev_op;
            self.op_mut(new_op2).next = next_next_op;
            self.op_mut(next_next_op).prev = new_op2;
            self.op_mut(prev_op).next = new_op2;
        }

        if abs_area2 >= 1.0 && (abs_area2 > abs_area1 || (area2 > 0.0) == (area1 > 0.0)) {
            let new_or = self.new_outrec();
            let owner = self.or(outrec).owner;
            self.or_mut(new_or).owner = owner;
            self.op_mut(split_op).outrec = new_or;
            self.op_mut(split_next).outrec = new_or;
            let new_op = self.new_outpt(ip, new_or);
            self.op_mut(new_op).prev = split_next;
            self.op_mut(new_op).next = split_op;
            self.or_mut(new_or).pts = new_op;
            self.op_mut(split_op).prev = new_op;
            self.op_mut(split_next).next = new_op;

            if self.using_polytree {
                if self.path2_contains_path1_op(prev_op, new_op) {
                    self.or_mut(new_or).splits = Some(vec![outrec]);
                } else {
                    self.or_mut(outrec)
                        .splits
                        .get_or_insert_with(Vec::new)
                        .push(new_or);
                }
            }
        }
        // else: splitOp and its next are deleted (left unreferenced)
    }

    fn fix_self_intersects(&mut self, outrec: usize) {
        let mut op2 = self.or(outrec).pts;
        if self.op(op2).prev == self.op(self.op(op2).next).next {
            return;
        }
        loop {
            let o = *self.op(op2);
            let n = self.op(o.next).next;
            if segments_intersect(self.op(o.prev).pt, o.pt, self.op(o.next).pt, self.op(n).pt) {
                let nnn = self.op(n).next;
                if segments_intersect(self.op(o.prev).pt, o.pt, self.op(n).pt, self.op(nnn).pt) {
                    op2 = self.duplicate_op(op2, false);
                    let p = self.op(self.op(self.op(self.op(op2).next).next).next).pt;
                    self.op_mut(op2).pt = p;
                    op2 = self.op(op2).next;
                } else {
                    let pts = self.or(outrec).pts;
                    if op2 == pts || self.op(op2).next == pts {
                        let pp = self.op(pts).prev;
                        self.or_mut(outrec).pts = pp;
                    }
                    self.do_split_op(outrec, op2);
                    if self.or(outrec).pts == NIL {
                        break;
                    }
                    op2 = self.or(outrec).pts;
                    if self.op(op2).prev == self.op(self.op(op2).next).next {
                        break;
                    }
                    continue;
                }
            } else {
                op2 = self.op(op2).next;
            }
            if op2 == self.or(outrec).pts {
                break;
            }
        }
    }

    fn start_open_path(&mut self, e: usize, pt: Point64) -> usize {
        let outrec = self.new_outrec();
        self.or_mut(outrec).is_open = true;
        if self.ae(e).wind_dx > 0 {
            self.or_mut(outrec).front_edge = e;
            self.or_mut(outrec).back_edge = NIL;
        } else {
            self.or_mut(outrec).front_edge = NIL;
            self.or_mut(outrec).back_edge = e;
        }
        self.ae_mut(e).outrec = outrec;
        let op = self.new_outpt(pt, outrec);
        self.or_mut(outrec).pts = op;
        op
    }

    fn trim_horz(&mut self, horz_edge: usize, preserve_collinear: bool) {
        let mut was_trimmed = false;
        let mut pt = self.vertices[self.next_vertex(horz_edge)].pt;
        while pt.y == self.ae(horz_edge).top.y {
            let a = self.ae(horz_edge);
            if preserve_collinear && ((pt.x < a.top.x) != (a.bot.x < a.top.x)) {
                break;
            }
            let nv = self.next_vertex(horz_edge);
            self.ae_mut(horz_edge).vertex_top = nv;
            self.ae_mut(horz_edge).top = pt;
            was_trimmed = true;
            if self.is_maxima(horz_edge) {
                break;
            }
            pt = self.vertices[self.next_vertex(horz_edge)].pt;
        }
        if was_trimmed {
            self.set_dx(horz_edge);
        }
    }

    fn update_edge_into_ael(&mut self, e: usize) {
        let top = self.ae(e).top;
        self.ae_mut(e).bot = top;
        let nv = self.next_vertex(e);
        self.ae_mut(e).vertex_top = nv;
        self.ae_mut(e).top = self.vertices[nv].pt;
        self.ae_mut(e).curr_x = top.x;
        self.set_dx(e);
        if self.is_joined(e) {
            self.split(e, top);
        }
        if is_horizontal(self.ae(e)) {
            if !self.is_open(e) {
                let pc = self.preserve_collinear;
                self.trim_horz(e, pc);
            }
            return;
        }
        let ty = self.ae(e).top.y;
        self.insert_scanline(ty);
        let bot = self.ae(e).bot;
        self.check_join_left(e, bot, false);
        self.check_join_right(e, bot, true);
    }

    fn find_edge_with_matching_loc_min(&self, e: usize) -> usize {
        let mut result = self.ae(e).next_in_ael;
        while result != NIL {
            if self.ae(result).local_min == self.ae(e).local_min {
                return result;
            } else if !is_horizontal(self.ae(result)) && self.ae(e).bot != self.ae(result).bot {
                result = NIL;
            } else {
                result = self.ae(result).next_in_ael;
            }
        }
        result = self.ae(e).prev_in_ael;
        while result != NIL {
            if self.ae(result).local_min == self.ae(e).local_min {
                return result;
            } else if !is_horizontal(self.ae(result)) && self.ae(e).bot != self.ae(result).bot {
                return NIL;
            } else {
                result = self.ae(result).prev_in_ael;
            }
        }
        result
    }

    fn intersect_edges(&mut self, e1: usize, e2: usize, pt: Point64) {
        if self.has_open_paths && (self.is_open(e1) || self.is_open(e2)) {
            if self.is_open(e1) && self.is_open(e2) {
                return;
            }
            let (edge_o, edge_c) = if self.is_open(e1) { (e1, e2) } else { (e2, e1) };
            if self.is_joined(edge_c) {
                self.split(edge_c, pt);
            }
            if self.ae(edge_c).wind_cnt.wrapping_abs() != 1 {
                return;
            }
            match self.cliptype {
                ClipType::Union => {
                    if !self.is_hot_edge(edge_c) {
                        return;
                    }
                }
                _ => {
                    if self.minima_list[self.ae(edge_c).local_min].polytype == PathType::Subject {
                        return;
                    }
                }
            }
            match self.fillrule {
                FillRule::Positive => {
                    if self.ae(edge_c).wind_cnt != 1 {
                        return;
                    }
                }
                FillRule::Negative => {
                    if self.ae(edge_c).wind_cnt != -1 {
                        return;
                    }
                }
                _ => {
                    if self.ae(edge_c).wind_cnt.wrapping_abs() != 1 {
                        return;
                    }
                }
            }
            if self.is_hot_edge(edge_o) {
                self.add_out_pt(edge_o, pt);
                let o = self.ae(edge_o).outrec;
                if self.is_front(edge_o) {
                    self.or_mut(o).front_edge = NIL;
                } else {
                    self.or_mut(o).back_edge = NIL;
                }
                self.ae_mut(edge_o).outrec = NIL;
            } else if pt == self.vertices[self.minima_list[self.ae(edge_o).local_min].vertex].pt
                && !self.is_open_end_v(self.minima_list[self.ae(edge_o).local_min].vertex)
            {
                let e3 = self.find_edge_with_matching_loc_min(edge_o);
                if e3 != NIL && self.is_hot_edge(e3) {
                    let e3o = self.ae(e3).outrec;
                    self.ae_mut(edge_o).outrec = e3o;
                    if self.ae(edge_o).wind_dx > 0 {
                        self.set_sides(e3o, edge_o, e3);
                    } else {
                        self.set_sides(e3o, e3, edge_o);
                    }
                    return;
                } else {
                    self.start_open_path(edge_o, pt);
                }
            } else {
                self.start_open_path(edge_o, pt);
            }
            return;
        }

        if self.is_joined(e1) {
            self.split(e1, pt);
        }
        if self.is_joined(e2) {
            self.split(e2, pt);
        }

        let fillrule = self.fillrule;
        if self.get_poly_type(e1) == self.get_poly_type(e2) {
            if fillrule == FillRule::EvenOdd {
                let a = self.ae(e1).wind_cnt;
                let b = self.ae(e2).wind_cnt;
                self.ae_mut(e1).wind_cnt = b;
                self.ae_mut(e2).wind_cnt = a;
            } else {
                let (w1, d1) = (self.ae(e1).wind_cnt, self.ae(e1).wind_dx);
                let d2 = self.ae(e2).wind_dx;
                let nw1 = if w1.wrapping_add(d2) == 0 {
                    w1.wrapping_neg()
                } else {
                    w1.wrapping_add(d2)
                };
                self.ae_mut(e1).wind_cnt = nw1;
                let w2 = self.ae(e2).wind_cnt;
                let nw2 = if w2.wrapping_sub(d1) == 0 {
                    w2.wrapping_neg()
                } else {
                    w2.wrapping_sub(d1)
                };
                self.ae_mut(e2).wind_cnt = nw2;
            }
        } else if fillrule != FillRule::EvenOdd {
            let d2 = self.ae(e2).wind_dx;
            let d1 = self.ae(e1).wind_dx;
            let a = self.ae(e1).wind_cnt2.wrapping_add(d2);
            self.ae_mut(e1).wind_cnt2 = a;
            let b = self.ae(e2).wind_cnt2.wrapping_sub(d1);
            self.ae_mut(e2).wind_cnt2 = b;
        } else {
            let a = self.ae(e1).wind_cnt2;
            let b = self.ae(e2).wind_cnt2;
            self.ae_mut(e1).wind_cnt2 = if a == 0 { 1 } else { 0 };
            self.ae_mut(e2).wind_cnt2 = if b == 0 { 1 } else { 0 };
        }

        let (old_e1_windcnt, old_e2_windcnt) = match fillrule {
            FillRule::EvenOdd | FillRule::NonZero => (
                self.ae(e1).wind_cnt.wrapping_abs(),
                self.ae(e2).wind_cnt.wrapping_abs(),
            ),
            _ => {
                if fillrule == FillRule::Positive {
                    (self.ae(e1).wind_cnt, self.ae(e2).wind_cnt)
                } else {
                    (
                        self.ae(e1).wind_cnt.wrapping_neg(),
                        self.ae(e2).wind_cnt.wrapping_neg(),
                    )
                }
            }
        };

        let e1_windcnt_in_01 = old_e1_windcnt == 0 || old_e1_windcnt == 1;
        let e2_windcnt_in_01 = old_e2_windcnt == 0 || old_e2_windcnt == 1;

        if (!self.is_hot_edge(e1) && !e1_windcnt_in_01)
            || (!self.is_hot_edge(e2) && !e2_windcnt_in_01)
        {
            return;
        }

        if self.is_hot_edge(e1) && self.is_hot_edge(e2) {
            if (old_e1_windcnt != 0 && old_e1_windcnt != 1)
                || (old_e2_windcnt != 0 && old_e2_windcnt != 1)
                || (self.get_poly_type(e1) != self.get_poly_type(e2)
                    && self.cliptype != ClipType::Xor)
            {
                self.add_local_max_poly(e1, e2, pt);
            } else if self.is_front(e1) || self.ae(e1).outrec == self.ae(e2).outrec {
                self.add_local_max_poly(e1, e2, pt);
                self.add_local_min_poly(e1, e2, pt, false);
            } else {
                self.add_out_pt(e1, pt);
                self.add_out_pt(e2, pt);
                self.swap_outrecs(e1, e2);
            }
        } else if self.is_hot_edge(e1) {
            self.add_out_pt(e1, pt);
            self.swap_outrecs(e1, e2);
        } else if self.is_hot_edge(e2) {
            self.add_out_pt(e2, pt);
            self.swap_outrecs(e1, e2);
        } else {
            let (e1_wc2, e2_wc2): (i64, i64) = match fillrule {
                FillRule::EvenOdd | FillRule::NonZero => (
                    self.ae(e1).wind_cnt2.wrapping_abs() as i64,
                    self.ae(e2).wind_cnt2.wrapping_abs() as i64,
                ),
                _ => {
                    if fillrule == FillRule::Positive {
                        (self.ae(e1).wind_cnt2 as i64, self.ae(e2).wind_cnt2 as i64)
                    } else {
                        (
                            self.ae(e1).wind_cnt2.wrapping_neg() as i64,
                            self.ae(e2).wind_cnt2.wrapping_neg() as i64,
                        )
                    }
                }
            };

            if !self.is_same_poly_type(e1, e2) {
                self.add_local_min_poly(e1, e2, pt, false);
            } else if old_e1_windcnt == 1 && old_e2_windcnt == 1 {
                match self.cliptype {
                    ClipType::Union => {
                        if e1_wc2 <= 0 && e2_wc2 <= 0 {
                            self.add_local_min_poly(e1, e2, pt, false);
                        }
                    }
                    ClipType::Difference => {
                        if (self.get_poly_type(e1) == PathType::Clip && e1_wc2 > 0 && e2_wc2 > 0)
                            || (self.get_poly_type(e1) == PathType::Subject
                                && e1_wc2 <= 0
                                && e2_wc2 <= 0)
                        {
                            self.add_local_min_poly(e1, e2, pt, false);
                        }
                    }
                    ClipType::Xor => {
                        self.add_local_min_poly(e1, e2, pt, false);
                    }
                    _ => {
                        if e1_wc2 > 0 && e2_wc2 > 0 {
                            self.add_local_min_poly(e1, e2, pt, false);
                        }
                    }
                }
            }
        }
    }

    fn delete_from_ael(&mut self, e: usize) {
        let prev = self.ae(e).prev_in_ael;
        let next = self.ae(e).next_in_ael;
        if prev == NIL && next == NIL && e != self.actives {
            return;
        }
        if prev != NIL {
            self.ae_mut(prev).next_in_ael = next;
        } else {
            self.actives = next;
        }
        if next != NIL {
            self.ae_mut(next).prev_in_ael = prev;
        }
        // `delete &e`: the arena slot is simply abandoned.
    }

    fn adjust_curr_x_and_copy_to_sel(&mut self, top_y: i64) {
        let mut e = self.actives;
        self.sel = e;
        while e != NIL {
            let a = self.ae(e);
            let (p, n) = (a.prev_in_ael, a.next_in_ael);
            let cx = top_x(a, top_y);
            let am = self.ae_mut(e);
            am.prev_in_sel = p;
            am.next_in_sel = n;
            am.jump = n;
            am.curr_x = cx;
            e = n;
        }
    }

    /// `ClipperBase::ExecuteInternal`.
    pub fn execute_internal(
        &mut self,
        ct: ClipType,
        fillrule: FillRule,
        use_polytrees: bool,
    ) -> bool {
        self.cliptype = ct;
        self.fillrule = fillrule;
        self.using_polytree = use_polytrees;
        self.reset();
        if ct == ClipType::NoClip {
            return true;
        }
        let Some(mut y) = self.pop_scanline() else {
            return true;
        };
        while self.succeeded {
            self.insert_local_minima_into_ael(y);
            while let Some(e) = self.pop_horz() {
                self.do_horizontal(e);
            }
            if !self.horz_seg_list.is_empty() {
                self.convert_horz_segs_to_joins();
                self.horz_seg_list.clear();
            }
            self.bot_y = y;
            match self.pop_scanline() {
                Some(ny) => y = ny,
                None => break,
            }
            self.do_intersections(y);
            self.do_top_of_scanbeam(y);
            while let Some(e) = self.pop_horz() {
                self.do_horizontal(e);
            }
        }
        if self.succeeded {
            self.process_horz_joins();
        }
        self.succeeded
    }

    fn fix_outrec_pts(&mut self, outrec: usize) {
        let start = self.or(outrec).pts;
        let mut op = start;
        loop {
            self.op_mut(op).outrec = outrec;
            op = self.op(op).next;
            if op == start {
                break;
            }
        }
    }

    fn set_horz_seg_heading_forward(&self, hs: &mut HorzSegment, op_p: usize, op_n: usize) -> bool {
        let (px, nx) = (self.op(op_p).pt.x, self.op(op_n).pt.x);
        if px == nx {
            return false;
        }
        if px < nx {
            hs.left_op = op_p;
            hs.right_op = op_n;
            hs.left_to_right = true;
        } else {
            hs.left_op = op_n;
            hs.right_op = op_p;
            hs.left_to_right = false;
        }
        true
    }

    fn update_horz_segment(&mut self, hs: &mut HorzSegment) -> bool {
        let op = hs.left_op;
        let outrec = self.get_real_outrec(self.op(op).outrec);
        let outrec_has_edges = self.or(outrec).front_edge != NIL;
        let curr_y = self.op(op).pt.y;
        let mut op_p = op;
        let mut op_n = op;
        if outrec_has_edges {
            let op_a = self.or(outrec).pts;
            let op_z = self.op(op_a).next;
            while op_p != op_z && self.op(self.op(op_p).prev).pt.y == curr_y {
                op_p = self.op(op_p).prev;
            }
            while op_n != op_a && self.op(self.op(op_n).next).pt.y == curr_y {
                op_n = self.op(op_n).next;
            }
        } else {
            while self.op(op_p).prev != op_n && self.op(self.op(op_p).prev).pt.y == curr_y {
                op_p = self.op(op_p).prev;
            }
            while self.op(op_n).next != op_p && self.op(self.op(op_n).next).pt.y == curr_y {
                op_n = self.op(op_n).next;
            }
        }
        let result = self.set_horz_seg_heading_forward(hs, op_p, op_n) && !self.op(hs.left_op).horz;
        if result {
            let l = hs.left_op;
            self.op_mut(l).horz = true;
        } else {
            hs.right_op = NIL;
        }
        result
    }

    fn convert_horz_segs_to_joins(&mut self) {
        let mut list = std::mem::take(&mut self.horz_seg_list);
        let mut j = 0usize;
        for hs in list.iter_mut() {
            if self.update_horz_segment(hs) {
                j += 1;
            }
        }
        if j < 2 {
            self.horz_seg_list = list;
            return;
        }
        // HorzSegSorter, std::stable_sort
        list.sort_by(|hs1, hs2| {
            let less = |a: &HorzSegment, b: &HorzSegment| {
                if a.right_op == NIL || b.right_op == NIL {
                    a.right_op != NIL
                } else {
                    self.op(b.left_op).pt.x > self.op(a.left_op).pt.x
                }
            };
            if less(hs1, hs2) {
                std::cmp::Ordering::Less
            } else if less(hs2, hs1) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
        let hs_end = j;
        let hs_end1 = hs_end - 1;
        let mut i1 = 0usize;
        while i1 != hs_end1 {
            let mut i2 = i1 + 1;
            while i2 != hs_end {
                let (h1, h2) = (list[i1], list[i2]);
                if self.op(h2.left_op).pt.x >= self.op(h1.right_op).pt.x
                    || h2.left_to_right == h1.left_to_right
                    || self.op(h2.right_op).pt.x <= self.op(h1.left_op).pt.x
                {
                    i2 += 1;
                    continue;
                }
                let curr_y = self.op(h1.left_op).pt.y;
                if h1.left_to_right {
                    while self.op(self.op(list[i1].left_op).next).pt.y == curr_y
                        && self.op(self.op(list[i1].left_op).next).pt.x
                            <= self.op(list[i2].left_op).pt.x
                    {
                        list[i1].left_op = self.op(list[i1].left_op).next;
                    }
                    while self.op(self.op(list[i2].left_op).prev).pt.y == curr_y
                        && self.op(self.op(list[i2].left_op).prev).pt.x
                            <= self.op(list[i1].left_op).pt.x
                    {
                        list[i2].left_op = self.op(list[i2].left_op).prev;
                    }
                    let op1 = self.duplicate_op(list[i1].left_op, true);
                    let op2 = self.duplicate_op(list[i2].left_op, false);
                    self.horz_join_list.push(HorzJoin { op1, op2 });
                } else {
                    while self.op(self.op(list[i1].left_op).prev).pt.y == curr_y
                        && self.op(self.op(list[i1].left_op).prev).pt.x
                            <= self.op(list[i2].left_op).pt.x
                    {
                        list[i1].left_op = self.op(list[i1].left_op).prev;
                    }
                    while self.op(self.op(list[i2].left_op).next).pt.y == curr_y
                        && self.op(self.op(list[i2].left_op).next).pt.x
                            <= self.op(list[i1].left_op).pt.x
                    {
                        list[i2].left_op = self.op(list[i2].left_op).next;
                    }
                    let op1 = self.duplicate_op(list[i2].left_op, true);
                    let op2 = self.duplicate_op(list[i1].left_op, false);
                    self.horz_join_list.push(HorzJoin { op1, op2 });
                }
                i2 += 1;
            }
            i1 += 1;
        }
        self.horz_seg_list = list;
    }

    fn move_splits(&mut self, from_or: usize, to_or: usize) {
        let moved: Vec<usize> = self
            .or_mut(from_or)
            .splits
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        self.or_mut(to_or)
            .splits
            .get_or_insert_with(Vec::new)
            .extend(moved);
    }

    fn process_horz_joins(&mut self) {
        let joins = std::mem::take(&mut self.horz_join_list);
        for j in &joins {
            let or1 = self.get_real_outrec(self.op(j.op1).outrec);
            let mut or2 = self.get_real_outrec(self.op(j.op2).outrec);

            let op1b = self.op(j.op1).next;
            let op2b = self.op(j.op2).prev;
            self.op_mut(j.op1).next = j.op2;
            self.op_mut(j.op2).prev = j.op1;
            self.op_mut(op1b).prev = op2b;
            self.op_mut(op2b).next = op1b;

            if or1 == or2 {
                or2 = self.new_outrec();
                self.or_mut(or2).pts = op1b;
                self.fix_outrec_pts(or2);

                let or1_pts = self.or(or1).pts;
                if self.op(or1_pts).outrec == or2 {
                    self.or_mut(or1).pts = j.op1;
                    self.op_mut(j.op1).outrec = or1;
                }

                if self.using_polytree {
                    if self.path2_contains_path1_op(self.or(or1).pts, self.or(or2).pts) {
                        let tmp = self.or(or1).pts;
                        let p2 = self.or(or2).pts;
                        self.or_mut(or1).pts = p2;
                        self.or_mut(or2).pts = tmp;
                        self.fix_outrec_pts(or1);
                        self.fix_outrec_pts(or2);
                        self.or_mut(or2).owner = or1;
                    } else if self.path2_contains_path1_op(self.or(or2).pts, self.or(or1).pts) {
                        self.or_mut(or2).owner = or1;
                    } else {
                        let o = self.or(or1).owner;
                        self.or_mut(or2).owner = o;
                    }
                    self.or_mut(or1)
                        .splits
                        .get_or_insert_with(Vec::new)
                        .push(or2);
                } else {
                    self.or_mut(or2).owner = or1;
                }
            } else {
                self.or_mut(or2).pts = NIL;
                if self.using_polytree {
                    self.set_owner(or2, or1);
                    if self.or(or2).splits.is_some() {
                        self.move_splits(or2, or1);
                    }
                } else {
                    self.or_mut(or2).owner = or1;
                }
            }
        }
        self.horz_join_list = joins;
    }

    fn do_intersections(&mut self, top_y: i64) {
        if self.build_intersect_list(top_y) {
            self.process_intersect_list();
            self.intersect_nodes.clear();
        }
    }

    fn add_new_intersect_node(&mut self, e1: usize, e2: usize, top_y: i64) {
        let a1 = *self.ae(e1);
        let a2 = *self.ae(e2);
        let mut ip = Point64::default();
        if !get_segment_intersect_pt(a1.bot, a1.top, a2.bot, a2.top, &mut ip) {
            ip = Point64::new(a1.curr_x, top_y);
        }
        if ip.y > self.bot_y || ip.y < top_y {
            let abs_dx1 = a1.dx.abs();
            let abs_dx2 = a2.dx.abs();
            if abs_dx1 > 100.0 && abs_dx2 > 100.0 {
                if abs_dx1 > abs_dx2 {
                    ip = get_closest_point_on_segment(ip, a1.bot, a1.top);
                } else {
                    ip = get_closest_point_on_segment(ip, a2.bot, a2.top);
                }
            } else if abs_dx1 > 100.0 {
                ip = get_closest_point_on_segment(ip, a1.bot, a1.top);
            } else if abs_dx2 > 100.0 {
                ip = get_closest_point_on_segment(ip, a2.bot, a2.top);
            } else {
                if ip.y < top_y {
                    ip.y = top_y;
                } else {
                    ip.y = self.bot_y;
                }
                if abs_dx1 < abs_dx2 {
                    ip.x = top_x(&a1, ip.y);
                } else {
                    ip.x = top_x(&a2, ip.y);
                }
            }
        }
        self.intersect_nodes.push(IntersectNode {
            pt: ip,
            edge1: e1,
            edge2: e2,
        });
    }

    fn build_intersect_list(&mut self, top_y: i64) -> bool {
        if self.actives == NIL || self.ae(self.actives).next_in_ael == NIL {
            return false;
        }
        self.adjust_curr_x_and_copy_to_sel(top_y);
        let mut left = self.sel;
        while left != NIL && self.ae(left).jump != NIL {
            let mut prev_base = NIL;
            while left != NIL && self.ae(left).jump != NIL {
                let mut curr_base = left;
                let mut right = self.ae(left).jump;
                let mut l_end = right;
                let r_end = self.ae(right).jump;
                self.ae_mut(left).jump = r_end;
                while left != l_end && right != r_end {
                    if self.ae(right).curr_x < self.ae(left).curr_x {
                        let mut tmp = self.ae(right).prev_in_sel;
                        loop {
                            self.add_new_intersect_node(tmp, right, top_y);
                            if tmp == left {
                                break;
                            }
                            tmp = self.ae(tmp).prev_in_sel;
                        }
                        tmp = right;
                        right = self.extract_from_sel(tmp);
                        l_end = right;
                        self.insert1_before2_in_sel(tmp, left);
                        if left == curr_base {
                            curr_base = tmp;
                            self.ae_mut(curr_base).jump = r_end;
                            if prev_base == NIL {
                                self.sel = curr_base;
                            } else {
                                self.ae_mut(prev_base).jump = curr_base;
                            }
                        }
                    } else {
                        left = self.ae(left).next_in_sel;
                    }
                }
                prev_base = curr_base;
                left = r_end;
            }
            left = self.sel;
        }
        !self.intersect_nodes.is_empty()
    }

    fn edges_adjacent_in_ael(&self, inode: &IntersectNode) -> bool {
        self.ae(inode.edge1).next_in_ael == inode.edge2
            || self.ae(inode.edge1).prev_in_ael == inode.edge2
    }

    fn process_intersect_list(&mut self) {
        cxxsort::sort(&mut self.intersect_nodes, intersect_list_sort);
        let n = self.intersect_nodes.len();
        let mut i = 0;
        while i != n {
            if !self.edges_adjacent_in_ael(&self.intersect_nodes[i]) {
                let mut i2 = i + 1;
                while !self.edges_adjacent_in_ael(&self.intersect_nodes[i2]) {
                    i2 += 1;
                }
                self.intersect_nodes.swap(i, i2);
            }
            let node = self.intersect_nodes[i];
            self.intersect_edges(node.edge1, node.edge2, node.pt);
            self.swap_positions_in_ael(node.edge1, node.edge2);
            self.ae_mut(node.edge1).curr_x = node.pt.x;
            self.ae_mut(node.edge2).curr_x = node.pt.x;
            self.check_join_left(node.edge2, node.pt, true);
            self.check_join_right(node.edge1, node.pt, true);
            i += 1;
        }
    }

    fn swap_positions_in_ael(&mut self, e1: usize, e2: usize) {
        let next = self.ae(e2).next_in_ael;
        if next != NIL {
            self.ae_mut(next).prev_in_ael = e1;
        }
        let prev = self.ae(e1).prev_in_ael;
        if prev != NIL {
            self.ae_mut(prev).next_in_ael = e2;
        }
        self.ae_mut(e2).prev_in_ael = prev;
        self.ae_mut(e2).next_in_ael = e1;
        self.ae_mut(e1).prev_in_ael = e2;
        self.ae_mut(e1).next_in_ael = next;
        if self.ae(e2).prev_in_ael == NIL {
            self.actives = e2;
        }
    }

    fn get_last_op(&self, hot_edge: usize) -> usize {
        let outrec = self.ae(hot_edge).outrec;
        let mut result = self.or(outrec).pts;
        if hot_edge != self.or(outrec).front_edge {
            result = self.op(result).next;
        }
        result
    }

    fn add_trial_horz_join(&mut self, op: usize) {
        if self.or(self.op(op).outrec).is_open {
            return;
        }
        self.horz_seg_list.push(HorzSegment {
            left_op: op,
            right_op: NIL,
            left_to_right: true,
        });
    }

    fn reset_horz_direction(
        &self,
        horz: usize,
        max_vertex: usize,
        horz_left: &mut i64,
        horz_right: &mut i64,
    ) -> bool {
        let h = self.ae(horz);
        if h.bot.x == h.top.x {
            *horz_left = h.curr_x;
            *horz_right = h.curr_x;
            let mut e = h.next_in_ael;
            while e != NIL && self.ae(e).vertex_top != max_vertex {
                e = self.ae(e).next_in_ael;
            }
            e != NIL
        } else if h.curr_x < h.top.x {
            *horz_left = h.curr_x;
            *horz_right = h.top.x;
            true
        } else {
            *horz_left = h.top.x;
            *horz_right = h.curr_x;
            false
        }
    }

    fn do_horizontal(&mut self, horz: usize) {
        let horz_is_open = self.is_open(horz);
        let y = self.ae(horz).bot.y;
        let vertex_max = if horz_is_open {
            self.get_curr_y_maxima_vertex_open(horz)
        } else {
            self.get_curr_y_maxima_vertex(horz)
        };
        let mut horz_left = 0i64;
        let mut horz_right = 0i64;
        let mut is_left_to_right =
            self.reset_horz_direction(horz, vertex_max, &mut horz_left, &mut horz_right);

        if self.is_hot_edge(horz) {
            let cx = self.ae(horz).curr_x;
            let op = self.add_out_pt(horz, Point64::new(cx, y));
            self.add_trial_horz_join(op);
        }

        loop {
            let mut e = if is_left_to_right {
                self.ae(horz).next_in_ael
            } else {
                self.ae(horz).prev_in_ael
            };
            while e != NIL {
                if self.ae(e).vertex_top == vertex_max {
                    if self.is_hot_edge(horz) && self.is_joined(e) {
                        let t = self.ae(e).top;
                        self.split(e, t);
                    }
                    if self.is_hot_edge(horz) {
                        while self.ae(horz).vertex_top != vertex_max {
                            let t = self.ae(horz).top;
                            self.add_out_pt(horz, t);
                            self.update_edge_into_ael(horz);
                        }
                        let t = self.ae(horz).top;
                        if is_left_to_right {
                            self.add_local_max_poly(horz, e, t);
                        } else {
                            self.add_local_max_poly(e, horz, t);
                        }
                    }
                    self.delete_from_ael(e);
                    self.delete_from_ael(horz);
                    return;
                }

                if vertex_max != self.ae(horz).vertex_top || self.is_open_end(horz) {
                    let ecx = self.ae(e).curr_x;
                    if (is_left_to_right && ecx > horz_right)
                        || (!is_left_to_right && ecx < horz_left)
                    {
                        break;
                    }
                    if ecx == self.ae(horz).top.x && !is_horizontal(self.ae(e)) {
                        let pt = self.vertices[self.next_vertex(horz)].pt;
                        let special = self.is_open(e)
                            && !self.is_same_poly_type(e, horz)
                            && !self.is_hot_edge(e);
                        let tx = top_x(self.ae(e), pt.y);
                        if is_left_to_right {
                            if special {
                                if tx > pt.x {
                                    break;
                                }
                            } else if tx >= pt.x {
                                break;
                            }
                        } else if special {
                            if tx < pt.x {
                                break;
                            }
                        } else if tx <= pt.x {
                            break;
                        }
                    }
                }

                let pt = Point64::new(self.ae(e).curr_x, self.ae(horz).bot.y);
                if is_left_to_right {
                    self.intersect_edges(horz, e, pt);
                    self.swap_positions_in_ael(horz, e);
                    self.check_join_left(e, pt, false);
                    let ecx = self.ae(e).curr_x;
                    self.ae_mut(horz).curr_x = ecx;
                    e = self.ae(horz).next_in_ael;
                } else {
                    self.intersect_edges(e, horz, pt);
                    self.swap_positions_in_ael(e, horz);
                    self.check_join_right(e, pt, false);
                    let ecx = self.ae(e).curr_x;
                    self.ae_mut(horz).curr_x = ecx;
                    e = self.ae(horz).prev_in_ael;
                }

                if self.ae(horz).outrec != NIL {
                    let lo = self.get_last_op(horz);
                    self.add_trial_horz_join(lo);
                }
            }

            if horz_is_open && self.is_open_end(horz) {
                if self.is_hot_edge(horz) {
                    let t = self.ae(horz).top;
                    self.add_out_pt(horz, t);
                    let o = self.ae(horz).outrec;
                    if self.is_front(horz) {
                        self.or_mut(o).front_edge = NIL;
                    } else {
                        self.or_mut(o).back_edge = NIL;
                    }
                    self.ae_mut(horz).outrec = NIL;
                }
                self.delete_from_ael(horz);
                return;
            } else if self.vertices[self.next_vertex(horz)].pt.y != self.ae(horz).top.y {
                break;
            }

            if self.is_hot_edge(horz) {
                let t = self.ae(horz).top;
                self.add_out_pt(horz, t);
            }
            self.update_edge_into_ael(horz);
            is_left_to_right =
                self.reset_horz_direction(horz, vertex_max, &mut horz_left, &mut horz_right);
        }

        if self.is_hot_edge(horz) {
            let t = self.ae(horz).top;
            let op = self.add_out_pt(horz, t);
            self.add_trial_horz_join(op);
        }
        self.update_edge_into_ael(horz);
    }

    fn do_top_of_scanbeam(&mut self, y: i64) {
        self.sel = NIL;
        let mut e = self.actives;
        while e != NIL {
            if self.ae(e).top.y == y {
                let tx = self.ae(e).top.x;
                self.ae_mut(e).curr_x = tx;
                if self.is_maxima(e) {
                    e = self.do_maxima(e);
                    continue;
                } else {
                    if self.is_hot_edge(e) {
                        let t = self.ae(e).top;
                        self.add_out_pt(e, t);
                    }
                    self.update_edge_into_ael(e);
                    if is_horizontal(self.ae(e)) {
                        self.push_horz(e);
                    }
                }
            } else {
                let cx = top_x(self.ae(e), y);
                self.ae_mut(e).curr_x = cx;
            }
            e = self.ae(e).next_in_ael;
        }
    }

    fn do_maxima(&mut self, e: usize) -> usize {
        let prev_e = self.ae(e).prev_in_ael;
        let mut next_e = self.ae(e).next_in_ael;

        if self.is_open_end(e) {
            if self.is_hot_edge(e) {
                let t = self.ae(e).top;
                self.add_out_pt(e, t);
            }
            if !is_horizontal(self.ae(e)) {
                if self.is_hot_edge(e) {
                    let o = self.ae(e).outrec;
                    if self.is_front(e) {
                        self.or_mut(o).front_edge = NIL;
                    } else {
                        self.or_mut(o).back_edge = NIL;
                    }
                    self.ae_mut(e).outrec = NIL;
                }
                self.delete_from_ael(e);
            }
            return next_e;
        }

        let max_pair = self.get_maxima_pair(e);
        if max_pair == NIL {
            return next_e;
        }

        if self.is_joined(e) {
            let t = self.ae(e).top;
            self.split(e, t);
        }
        if self.is_joined(max_pair) {
            let t = self.ae(max_pair).top;
            self.split(max_pair, t);
        }

        while next_e != max_pair {
            let t = self.ae(e).top;
            self.intersect_edges(e, next_e, t);
            self.swap_positions_in_ael(e, next_e);
            next_e = self.ae(e).next_in_ael;
        }

        if self.is_open(e) {
            if self.is_hot_edge(e) {
                let t = self.ae(e).top;
                self.add_local_max_poly(e, max_pair, t);
            }
            self.delete_from_ael(max_pair);
            self.delete_from_ael(e);
            return if prev_e != NIL {
                self.ae(prev_e).next_in_ael
            } else {
                self.actives
            };
        }

        if self.is_hot_edge(e) {
            let t = self.ae(e).top;
            self.add_local_max_poly(e, max_pair, t);
        }
        self.delete_from_ael(e);
        self.delete_from_ael(max_pair);
        if prev_e != NIL {
            self.ae(prev_e).next_in_ael
        } else {
            self.actives
        }
    }

    fn split(&mut self, e: usize, pt: Point64) {
        if self.ae(e).join_with == JoinWith::Right {
            self.ae_mut(e).join_with = JoinWith::NoJoin;
            let n = self.ae(e).next_in_ael;
            self.ae_mut(n).join_with = JoinWith::NoJoin;
            self.add_local_min_poly(e, n, pt, true);
        } else {
            self.ae_mut(e).join_with = JoinWith::NoJoin;
            let p = self.ae(e).prev_in_ael;
            self.ae_mut(p).join_with = JoinWith::NoJoin;
            self.add_local_min_poly(p, e, pt, true);
        }
    }

    fn check_join_left(&mut self, e: usize, pt: Point64, check_curr_x: bool) {
        let prev = self.ae(e).prev_in_ael;
        if prev == NIL
            || !self.is_hot_edge(e)
            || !self.is_hot_edge(prev)
            || is_horizontal(self.ae(e))
            || is_horizontal(self.ae(prev))
            || self.is_open(e)
            || self.is_open(prev)
        {
            return;
        }
        let a = *self.ae(e);
        let p = *self.ae(prev);
        if (pt.y < a.top.y.wrapping_add(2) || pt.y < p.top.y.wrapping_add(2))
            && (a.bot.y > pt.y || p.bot.y > pt.y)
        {
            return;
        }
        if check_curr_x {
            if perpendic_dist_from_line_sqrd(pt, p.bot, p.top) > 0.25 {
                return;
            }
        } else if a.curr_x != p.curr_x {
            return;
        }
        if !is_collinear(a.top, pt, p.top) {
            return;
        }
        let (ei, pi) = (self.or(a.outrec).idx, self.or(p.outrec).idx);
        if ei == pi {
            self.add_local_max_poly(prev, e, pt);
        } else if ei < pi {
            self.join_outrec_paths(e, prev);
        } else {
            self.join_outrec_paths(prev, e);
        }
        self.ae_mut(prev).join_with = JoinWith::Right;
        self.ae_mut(e).join_with = JoinWith::Left;
    }

    fn check_join_right(&mut self, e: usize, pt: Point64, check_curr_x: bool) {
        let next = self.ae(e).next_in_ael;
        if next == NIL
            || !self.is_hot_edge(e)
            || !self.is_hot_edge(next)
            || is_horizontal(self.ae(e))
            || is_horizontal(self.ae(next))
            || self.is_open(e)
            || self.is_open(next)
        {
            return;
        }
        let a = *self.ae(e);
        let n = *self.ae(next);
        if (pt.y < a.top.y.wrapping_add(2) || pt.y < n.top.y.wrapping_add(2))
            && (a.bot.y > pt.y || n.bot.y > pt.y)
        {
            return;
        }
        if check_curr_x {
            if perpendic_dist_from_line_sqrd(pt, n.bot, n.top) > 0.35 {
                return;
            }
        } else if a.curr_x != n.curr_x {
            return;
        }
        if !is_collinear(a.top, pt, n.top) {
            return;
        }
        let (ei, ni) = (self.or(a.outrec).idx, self.or(n.outrec).idx);
        if ei == ni {
            self.add_local_max_poly(e, next, pt);
        } else if ei < ni {
            self.join_outrec_paths(e, next);
        } else {
            self.join_outrec_paths(next, e);
        }
        self.ae_mut(e).join_with = JoinWith::Right;
        self.ae_mut(next).join_with = JoinWith::Left;
    }

    // ---- building the solution ------------------------------------------

    fn build_path64(&self, mut op: usize, reverse: bool, is_open: bool, path: &mut Path64) -> bool {
        if op == NIL || self.op(op).next == op || (!is_open && self.op(op).next == self.op(op).prev)
        {
            return false;
        }
        path.clear();
        let mut last_pt;
        let mut op2;
        if reverse {
            last_pt = self.op(op).pt;
            op2 = self.op(op).prev;
        } else {
            op = self.op(op).next;
            last_pt = self.op(op).pt;
            op2 = self.op(op).next;
        }
        path.push(last_pt);
        while op2 != op {
            if self.op(op2).pt != last_pt {
                last_pt = self.op(op2).pt;
                path.push(last_pt);
            }
            op2 = if reverse {
                self.op(op2).prev
            } else {
                self.op(op2).next
            };
        }
        !(!is_open && path.len() == 3 && self.is_very_small_triangle(op2))
    }

    fn check_bounds(&mut self, outrec: usize) -> bool {
        if self.or(outrec).pts == NIL {
            return false;
        }
        if !self.or(outrec).bounds.is_empty() {
            return true;
        }
        self.clean_collinear(outrec);
        if self.or(outrec).pts == NIL {
            return false;
        }
        let mut path = std::mem::take(&mut self.or_mut(outrec).path);
        let ok = self.build_path64(self.or(outrec).pts, self.reverse_solution, false, &mut path);
        self.or_mut(outrec).path = path;
        if !ok {
            return false;
        }
        let b = get_bounds(&self.or(outrec).path);
        self.or_mut(outrec).bounds = b;
        true
    }

    fn check_split_owner(&mut self, outrec: usize, splits: &[usize]) -> bool {
        for &split0 in splits {
            if self.or(split0).pts == NIL {
                if let Some(s) = self.or(split0).splits.clone() {
                    if self.check_split_owner(outrec, &s) {
                        return true;
                    }
                }
            }
            let split = self.get_real_outrec(split0);
            if split == NIL || split == outrec || self.or(split).recursive_split == outrec {
                continue;
            }
            self.or_mut(split).recursive_split = outrec;
            if let Some(s) = self.or(split).splits.clone() {
                if self.check_split_owner(outrec, &s) {
                    return true;
                }
            }
            if !self.check_bounds(split)
                || !self.or(split).bounds.contains_rect(&self.or(outrec).bounds)
                || !self.path2_contains_path1_op(self.or(outrec).pts, self.or(split).pts)
            {
                continue;
            }
            if !self.is_valid_owner(outrec, split) {
                let o = self.or(outrec).owner;
                self.or_mut(split).owner = o;
            }
            self.or_mut(outrec).owner = split;
            return true;
        }
        false
    }

    fn recursive_check_owners(&mut self, outrec: usize, polypath: usize) {
        if self.or(outrec).polypath != NIL || self.or(outrec).bounds.is_empty() {
            return;
        }
        while self.or(outrec).owner != NIL {
            let owner = self.or(outrec).owner;
            if let Some(s) = self.or(owner).splits.clone() {
                if self.check_split_owner(outrec, &s) {
                    break;
                }
            }
            let owner = self.or(outrec).owner;
            if self.or(owner).pts != NIL
                && self.check_bounds(owner)
                && self.or(owner).bounds.contains_rect(&self.or(outrec).bounds)
                && self.path2_contains_path1_op(self.or(outrec).pts, self.or(owner).pts)
            {
                break;
            }
            let oo = self.or(owner).owner;
            self.or_mut(outrec).owner = oo;
        }
        let owner = self.or(outrec).owner;
        let path = self.or(outrec).path.clone();
        if owner != NIL {
            if self.or(owner).polypath == NIL {
                self.recursive_check_owners(owner, polypath);
            }
            let parent = self.or(owner).polypath;
            let pp = self.tree.as_mut().expect("tree").add_child(parent, path);
            self.or_mut(outrec).polypath = pp;
        } else {
            let pp = self.tree.as_mut().expect("tree").add_child(polypath, path);
            self.or_mut(outrec).polypath = pp;
        }
    }

    /// `Clipper64::BuildTree64`.
    fn build_tree64(&mut self, open_paths: &mut Paths64) -> PolyPath64 {
        self.tree = Some(TreeBuilder::new());
        open_paths.clear();
        // `outrec_list_.size()` is re-read every iteration: outrecs split off
        // while cleaning earlier ones are visited too.
        let mut i = 0;
        while i < self.outrec_list.len() {
            let cur = i;
            i += 1;
            let i = cur;
            if self.or(i).pts == NIL {
                continue;
            }
            if self.or(i).is_open {
                let mut path = Path64::new();
                if self.build_path64(self.or(i).pts, self.reverse_solution, true, &mut path) {
                    open_paths.push(path);
                }
                continue;
            }
            if self.check_bounds(i) {
                self.recursive_check_owners(i, 0);
            }
        }
        self.tree.take().expect("tree").into_tree()
    }

    /// Runs `ExecuteInternal(ct, fr, true)`; on success builds the poly tree
    /// (`BuildTree64`), then `CleanUp()`. Returns `None` when the sweep
    /// failed (the reference then leaves its previous open paths alone).
    pub fn execute_tree(&mut self, ct: ClipType, fr: FillRule) -> Option<(PolyPath64, Paths64)> {
        let result = if self.execute_internal(ct, fr, true) {
            let mut open = Paths64::new();
            let tree = self.build_tree64(&mut open);
            Some((tree, open))
        } else {
            None
        };
        self.clean_up();
        result
    }

    /// `Clipper64::BuildPaths64` + `Execute(ct, fr, Paths64&)`.
    pub fn execute_paths(&mut self, ct: ClipType, fr: FillRule) -> Option<(Paths64, Paths64)> {
        let result = if self.execute_internal(ct, fr, false) {
            let mut closed = Paths64::new();
            let mut open = Paths64::new();
            let mut i = 0;
            while i < self.outrec_list.len() {
                let cur = i;
                i += 1;
                let i = cur;
                if self.or(i).pts == NIL {
                    continue;
                }
                let mut path = Path64::new();
                if self.or(i).is_open {
                    if self.build_path64(self.or(i).pts, self.reverse_solution, true, &mut path) {
                        open.push(path);
                    }
                } else {
                    self.clean_collinear(i);
                    if self.build_path64(self.or(i).pts, self.reverse_solution, false, &mut path) {
                        closed.push(path);
                    }
                }
            }
            Some((closed, open))
        } else {
            None
        };
        self.clean_up();
        result
    }
}
