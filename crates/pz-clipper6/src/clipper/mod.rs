//! Port of Angus Johnson's Clipper 6.4.2 (`ClipperLib`, C++ edition) as
//! configured in the game's pathfinding library: 64-bit `cInt`, `use_lines`
//! on, `use_xyz` and `use_deprecated` off.
//!
//! Copyright (c) Angus Johnson 2010-2017. Distributed under the Boost
//! Software License, Version 1.0 (see `LICENSE-BOOST` in this crate).
//!
//! # Floating point
//!
//! The reference library was compiled by clang for arm64 with the default
//! `-ffp-contract=on`: inside a single C++ expression, `a * b + c`,
//! `c - a * b` and `a * b - c * d` are fused into one multiply-add (the
//! left-hand product is fused, a right-hand product is rounded first).
//! Products that cross a function boundary (such as the argument of the
//! inline `Round()`) are never fused. Every fused site is written here with
//! `fma` (`f64::mul_add`). Trigonometric functions go to the platform `libm`, like the
//! original.
//!
//! # Memory model
//!
//! Edges, output points and output records live in arenas indexed by
//! `usize` (a pointer in the original). Nothing is freed during an
//! `execute`, so stale links behave like the original's still-allocated
//! memory, which keeps the algorithm's observable behaviour identical.

mod engine;
mod offset;

pub use engine::Clipper;
pub use offset::ClipperOffset;

use std::fmt;

/// Fused multiply-add, `a * b + c` with a single rounding.
#[inline(always)]
pub(crate) fn fma(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

/// `cInt Round(double)`: round half away from zero, then truncate to
/// 64 bits (`fcvtzs`: saturating, NaN -> 0, same as Rust's `as`).
#[inline(always)]
pub(crate) fn round(val: f64) -> i64 {
    if val < 0.0 {
        (val - 0.5) as i64
    } else {
        (val + 0.5) as i64
    }
}

/// Coordinates up to this magnitude use plain 64-bit slope tests.
pub const LO_RANGE: i64 = 0x3FFF_FFFF;
/// Largest coordinate magnitude accepted by `add_path`.
pub const HI_RANGE: i64 = 0x3FFF_FFFF_FFFF_FFFF;

/// `ClipperLib::IntPoint` (`cInt` = `long long`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct IntPoint {
    pub x: i64,
    pub y: i64,
}

impl IntPoint {
    /// `IntPoint(x, y)`.
    #[inline]
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }
}

/// `ClipperLib::Path`.
pub type Path = Vec<IntPoint>;
/// `ClipperLib::Paths`.
pub type Paths = Vec<Path>;

/// `ClipperLib::DoublePoint`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DoublePoint {
    pub x: f64,
    pub y: f64,
}

impl DoublePoint {
    #[inline]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// `ClipperLib::IntRect`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntRect {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

/// `ClipperLib::ClipType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipType {
    Intersection,
    Union,
    Difference,
    Xor,
}

/// `ClipperLib::PolyType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PolyType {
    #[default]
    Subject,
    Clip,
}

/// `ClipperLib::PolyFillType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PolyFillType {
    #[default]
    EvenOdd,
    NonZero,
    Positive,
    Negative,
}

/// `ClipperLib::JoinType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum JoinType {
    #[default]
    Square,
    Round,
    Miter,
}

/// `ClipperLib::EndType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EndType {
    #[default]
    ClosedPolygon,
    ClosedLine,
    OpenButt,
    OpenSquare,
    OpenRound,
}

/// `ClipperLib::InitOptions::ioReverseSolution`.
pub const IO_REVERSE_SOLUTION: i32 = 1;
/// `ClipperLib::InitOptions::ioStrictlySimple`.
pub const IO_STRICTLY_SIMPLE: i32 = 2;
/// `ClipperLib::InitOptions::ioPreserveCollinear`.
pub const IO_PRESERVE_COLLINEAR: i32 = 4;

/// `ClipperLib::clipperException`: the message the original throws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipperException(pub &'static str);

impl fmt::Display for ClipperException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ClipperException {}

// ---------------------------------------------------------------------------
// PolyTree / PolyNode
// ---------------------------------------------------------------------------

/// Index of a node inside a [`PolyTree`]; [`PolyTree::ROOT`] is the tree
/// itself (the C++ `PolyTree` object, which is also a `PolyNode`).
pub type PolyNodeId = usize;

/// `ClipperLib::PolyNode`.
#[derive(Clone, Debug, Default)]
pub struct PolyNode {
    /// `Contour`.
    pub contour: Path,
    /// `Childs`.
    pub childs: Vec<PolyNodeId>,
    /// `Parent` (`None` only for the root).
    pub parent: Option<PolyNodeId>,
    pub(crate) index: u32,
    pub(crate) is_open: bool,
}

/// `ClipperLib::PolyTree`. Nodes are stored in an arena; node
/// [`PolyTree::ROOT`] is the tree object itself.
#[derive(Clone, Debug)]
pub struct PolyTree {
    pub(crate) nodes: Vec<PolyNode>,
    pub(crate) all_nodes: Vec<PolyNodeId>,
}

impl Default for PolyTree {
    fn default() -> Self {
        Self::new()
    }
}

impl PolyTree {
    /// The tree object itself.
    pub const ROOT: PolyNodeId = 0;

    /// `PolyTree()`.
    pub fn new() -> Self {
        Self {
            nodes: vec![PolyNode::default()],
            all_nodes: Vec::new(),
        }
    }

    /// `PolyTree::Clear()`.
    pub fn clear(&mut self) {
        self.nodes.truncate(1);
        self.all_nodes.clear();
        self.nodes[Self::ROOT].childs.clear();
    }

    /// `PolyTree::GetFirst()`.
    pub fn get_first(&self) -> Option<PolyNodeId> {
        self.nodes[Self::ROOT].childs.first().copied()
    }

    /// `PolyTree::Total()`.
    pub fn total(&self) -> i32 {
        let mut result = self.all_nodes.len() as i32;
        if result > 0 && self.nodes[Self::ROOT].childs.first() != self.all_nodes.first() {
            result -= 1;
        }
        result
    }

    /// The node with the given id.
    #[inline]
    pub fn node(&self, id: PolyNodeId) -> &PolyNode {
        &self.nodes[id]
    }

    /// The node with the given id, mutably.
    #[inline]
    pub fn node_mut(&mut self, id: PolyNodeId) -> &mut PolyNode {
        &mut self.nodes[id]
    }

    /// `PolyNode::Contour`.
    #[inline]
    pub fn contour(&self, id: PolyNodeId) -> &Path {
        &self.nodes[id].contour
    }

    /// `PolyNode::ChildCount()`.
    #[inline]
    pub fn child_count(&self, id: PolyNodeId) -> i32 {
        self.nodes[id].childs.len() as i32
    }

    /// `PolyNode::Childs`.
    #[inline]
    pub fn childs(&self, id: PolyNodeId) -> &[PolyNodeId] {
        &self.nodes[id].childs
    }

    /// `PolyNode::Parent`.
    #[inline]
    pub fn parent(&self, id: PolyNodeId) -> Option<PolyNodeId> {
        self.nodes[id].parent
    }

    /// `PolyNode::GetNext()`: depth-first successor.
    pub fn get_next(&self, id: PolyNodeId) -> Option<PolyNodeId> {
        match self.nodes[id].childs.first() {
            Some(&c) => Some(c),
            None => self.get_next_sibling_up(id),
        }
    }

    /// `PolyNode::GetNextSiblingUp()`.
    pub fn get_next_sibling_up(&self, id: PolyNodeId) -> Option<PolyNodeId> {
        let node = &self.nodes[id];
        let parent = node.parent?;
        let pc = &self.nodes[parent].childs;
        if node.index as usize == pc.len().wrapping_sub(1) {
            self.get_next_sibling_up(parent)
        } else {
            Some(pc[node.index as usize + 1])
        }
    }

    /// `PolyNode::IsHole()`.
    pub fn is_hole(&self, id: PolyNodeId) -> bool {
        let mut result = true;
        let mut node = self.nodes[id].parent;
        while let Some(n) = node {
            result = !result;
            node = self.nodes[n].parent;
        }
        result
    }

    /// `PolyNode::IsOpen()`.
    #[inline]
    pub fn is_open(&self, id: PolyNodeId) -> bool {
        self.nodes[id].is_open
    }

    /// `PolyNode::AddChild()`.
    pub(crate) fn add_child(&mut self, parent: PolyNodeId, child: PolyNodeId) {
        let cnt = self.nodes[parent].childs.len() as u32;
        self.nodes[parent].childs.push(child);
        self.nodes[child].parent = Some(parent);
        self.nodes[child].index = cnt;
    }

    /// `new PolyNode()` + `AllNodes.push_back()`.
    pub(crate) fn new_node(&mut self) -> PolyNodeId {
        let id = self.nodes.len();
        self.nodes.push(PolyNode::default());
        self.all_nodes.push(id);
        id
    }
}

// ---------------------------------------------------------------------------
// Miscellaneous global functions
// ---------------------------------------------------------------------------

/// `ClipperLib::Orientation(const Path&)`.
pub fn orientation(poly: &[IntPoint]) -> bool {
    area(poly) >= 0.0
}

/// `ClipperLib::Area(const Path&)`.
pub fn area(poly: &[IntPoint]) -> f64 {
    let size = poly.len() as i32;
    if size < 3 {
        return 0.0;
    }
    let mut a = 0.0f64;
    let mut j = (size - 1) as usize;
    for i in 0..size as usize {
        let (pj, pi) = (poly[j], poly[i]);
        a = fma(pj.x as f64 + pi.x as f64, pj.y as f64 - pi.y as f64, a);
        j = i;
    }
    -a * 0.5
}

/// `ClipperLib::PointInPolygon(const IntPoint&, const Path&)`: 0 outside,
/// +1 inside, -1 on the boundary.
pub fn point_in_polygon(pt: IntPoint, path: &[IntPoint]) -> i32 {
    let mut result = 0;
    let cnt = path.len();
    if cnt < 3 {
        return 0;
    }
    let mut ip = path[0];
    for i in 1..=cnt {
        let ip_next = if i == cnt { path[0] } else { path[i] };
        if ip_next.y == pt.y
            && (ip_next.x == pt.x || (ip.y == pt.y && ((ip_next.x > pt.x) == (ip.x < pt.x))))
        {
            return -1;
        }
        if (ip.y < pt.y) != (ip_next.y < pt.y) {
            if ip.x >= pt.x {
                if ip_next.x > pt.x {
                    result = 1 - result;
                } else {
                    let d = pip_cross(ip, ip_next, pt);
                    if d == 0.0 {
                        return -1;
                    }
                    if (d > 0.0) == (ip_next.y > ip.y) {
                        result = 1 - result;
                    }
                }
            } else if ip_next.x > pt.x {
                let d = pip_cross(ip, ip_next, pt);
                if d == 0.0 {
                    return -1;
                }
                if (d > 0.0) == (ip_next.y > ip.y) {
                    result = 1 - result;
                }
            }
        }
        ip = ip_next;
    }
    result
}

/// `(double)(ip.X - pt.X) * (next.Y - pt.Y) - (double)(next.X - pt.X) * (ip.Y - pt.Y)`
#[inline(always)]
pub(crate) fn pip_cross(ip: IntPoint, next: IntPoint, pt: IntPoint) -> f64 {
    fma(
        ip.x.wrapping_sub(pt.x) as f64,
        next.y.wrapping_sub(pt.y) as f64,
        -(next.x.wrapping_sub(pt.x) as f64 * ip.y.wrapping_sub(pt.y) as f64),
    )
}

/// `ClipperLib::ReversePath()`.
pub fn reverse_path(p: &mut Path) {
    p.reverse();
}

/// `ClipperLib::ReversePaths()`.
pub fn reverse_paths(p: &mut Paths) {
    for path in p.iter_mut() {
        path.reverse();
    }
}

/// `ClipperLib::SimplifyPolygon()`.
pub fn simplify_polygon(in_poly: &Path, fill_type: PolyFillType) -> Paths {
    let mut c = Clipper::new(0);
    c.set_strictly_simple(true);
    let _ = c.add_path(in_poly, PolyType::Subject, true);
    let mut out = Paths::new();
    let _ = c.execute(ClipType::Union, &mut out, fill_type, fill_type);
    out
}

/// `ClipperLib::SimplifyPolygons()`.
pub fn simplify_polygons(in_polys: &Paths, fill_type: PolyFillType) -> Paths {
    let mut c = Clipper::new(0);
    c.set_strictly_simple(true);
    let _ = c.add_paths(in_polys, PolyType::Subject, true);
    let mut out = Paths::new();
    let _ = c.execute(ClipType::Union, &mut out, fill_type, fill_type);
    out
}

/// `ClipperLib::DistanceFromLineSqrd()`.
fn distance_from_line_sqrd(pt: IntPoint, ln1: IntPoint, ln2: IntPoint) -> f64 {
    let a = ln1.y.wrapping_sub(ln2.y) as f64;
    let b = ln2.x.wrapping_sub(ln1.x) as f64;
    let mut c = fma(a, ln1.x as f64, b * ln1.y as f64);
    c = fma(a, pt.x as f64, b * pt.y as f64) - c;
    (c * c) / fma(a, a, b * b)
}

/// `ClipperLib::SlopesNearCollinear()`.
fn slopes_near_collinear(pt1: IntPoint, pt2: IntPoint, pt3: IntPoint, dist_sqrd: f64) -> bool {
    if pt1.x.wrapping_sub(pt2.x).wrapping_abs() > pt1.y.wrapping_sub(pt2.y).wrapping_abs() {
        if (pt1.x > pt2.x) == (pt1.x < pt3.x) {
            distance_from_line_sqrd(pt1, pt2, pt3) < dist_sqrd
        } else if (pt2.x > pt1.x) == (pt2.x < pt3.x) {
            distance_from_line_sqrd(pt2, pt1, pt3) < dist_sqrd
        } else {
            distance_from_line_sqrd(pt3, pt1, pt2) < dist_sqrd
        }
    } else if (pt1.y > pt2.y) == (pt1.y < pt3.y) {
        distance_from_line_sqrd(pt1, pt2, pt3) < dist_sqrd
    } else if (pt2.y > pt1.y) == (pt2.y < pt3.y) {
        distance_from_line_sqrd(pt2, pt1, pt3) < dist_sqrd
    } else {
        distance_from_line_sqrd(pt3, pt1, pt2) < dist_sqrd
    }
}

/// `ClipperLib::PointsAreClose()`.
fn points_are_close(pt1: IntPoint, pt2: IntPoint, dist_sqrd: f64) -> bool {
    let dx = pt1.x as f64 - pt2.x as f64;
    let dy = pt1.y as f64 - pt2.y as f64;
    fma(dx, dx, dy * dy) <= dist_sqrd
}

/// `ClipperLib::CleanPolygon(in, out, distance)` (default distance 1.415).
pub fn clean_polygon(in_poly: &[IntPoint], distance: f64) -> Path {
    let mut size = in_poly.len();
    if size == 0 {
        return Path::new();
    }
    // OutPt array: (pt, next, prev, idx)
    let mut pts: Vec<(IntPoint, usize, usize, i32)> =
        in_poly.iter().map(|&p| (p, 0, 0, 0)).collect();
    for i in 0..size {
        let n = (i + 1) % size;
        pts[i].1 = n;
        pts[n].2 = i;
    }
    let exclude = |pts: &mut Vec<(IntPoint, usize, usize, i32)>, op: usize| -> usize {
        let result = pts[op].2;
        let next = pts[op].1;
        pts[result].1 = next;
        pts[next].2 = result;
        pts[result].3 = 0;
        result
    };
    let dist_sqrd = distance * distance;
    let mut op = 0usize;
    while pts[op].3 == 0 && pts[op].1 != pts[op].2 {
        let (pt, next, prev, _) = pts[op];
        if points_are_close(pt, pts[prev].0, dist_sqrd) {
            op = exclude(&mut pts, op);
            size -= 1;
        } else if points_are_close(pts[prev].0, pts[next].0, dist_sqrd) {
            exclude(&mut pts, next);
            op = exclude(&mut pts, op);
            size = size.wrapping_sub(2);
        } else if slopes_near_collinear(pts[prev].0, pt, pts[next].0, dist_sqrd) {
            op = exclude(&mut pts, op);
            size -= 1;
        } else {
            pts[op].3 = 1;
            op = next;
        }
    }
    if size < 3 {
        size = 0;
    }
    let mut out = Vec::with_capacity(size);
    for _ in 0..size {
        out.push(pts[op].0);
        op = pts[op].1;
    }
    out
}

/// `ClipperLib::CleanPolygons()`.
pub fn clean_polygons(in_polys: &Paths, distance: f64) -> Paths {
    in_polys
        .iter()
        .map(|p| clean_polygon(p, distance))
        .collect()
}

fn minkowski(poly: &[IntPoint], path: &[IntPoint], is_sum: bool, is_closed: bool) -> Paths {
    let delta = usize::from(is_closed);
    let poly_cnt = poly.len();
    let path_cnt = path.len();
    let mut pp: Paths = Vec::with_capacity(path_cnt);
    for pi in path {
        let p: Path = poly
            .iter()
            .map(|q| {
                if is_sum {
                    IntPoint::new(pi.x.wrapping_add(q.x), pi.y.wrapping_add(q.y))
                } else {
                    IntPoint::new(pi.x.wrapping_sub(q.x), pi.y.wrapping_sub(q.y))
                }
            })
            .collect();
        pp.push(p);
    }
    let mut solution = Paths::new();
    for i in 0..(path_cnt.wrapping_sub(1).wrapping_add(delta)) {
        for j in 0..poly_cnt {
            let mut quad = vec![
                pp[i % path_cnt][j % poly_cnt],
                pp[(i + 1) % path_cnt][j % poly_cnt],
                pp[(i + 1) % path_cnt][(j + 1) % poly_cnt],
                pp[i % path_cnt][(j + 1) % poly_cnt],
            ];
            if !orientation(&quad) {
                quad.reverse();
            }
            solution.push(quad);
        }
    }
    solution
}

/// `ClipperLib::MinkowskiSum(const Path&, const Path&, Paths&, bool)`.
pub fn minkowski_sum(pattern: &Path, path: &Path, path_is_closed: bool) -> Paths {
    let mut solution = minkowski(pattern, path, true, path_is_closed);
    let mut c = Clipper::new(0);
    let _ = c.add_paths(&solution, PolyType::Subject, true);
    let _ = c.execute(
        ClipType::Union,
        &mut solution,
        PolyFillType::NonZero,
        PolyFillType::NonZero,
    );
    solution
}

/// `ClipperLib::MinkowskiSum(const Path&, const Paths&, Paths&, bool)`.
pub fn minkowski_sum_paths(pattern: &Path, paths: &Paths, path_is_closed: bool) -> Paths {
    let mut c = Clipper::new(0);
    for p in paths {
        let tmp = minkowski(pattern, p, true, path_is_closed);
        let _ = c.add_paths(&tmp, PolyType::Subject, true);
        if path_is_closed {
            let d = pattern[0];
            let tmp2: Path = p
                .iter()
                .map(|q| IntPoint::new(q.x.wrapping_add(d.x), q.y.wrapping_add(d.y)))
                .collect();
            let _ = c.add_path(&tmp2, PolyType::Clip, true);
        }
    }
    let mut solution = Paths::new();
    let _ = c.execute(
        ClipType::Union,
        &mut solution,
        PolyFillType::NonZero,
        PolyFillType::NonZero,
    );
    solution
}

/// `ClipperLib::MinkowskiDiff()`.
pub fn minkowski_diff(poly1: &Path, poly2: &Path) -> Paths {
    let mut solution = minkowski(poly1, poly2, false, true);
    let mut c = Clipper::new(0);
    let _ = c.add_paths(&solution, PolyType::Subject, true);
    let _ = c.execute(
        ClipType::Union,
        &mut solution,
        PolyFillType::NonZero,
        PolyFillType::NonZero,
    );
    solution
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeType {
    Any,
    Closed,
}

fn add_poly_node_to_paths(tree: &PolyTree, id: PolyNodeId, nt: NodeType, paths: &mut Paths) {
    let node = tree.node(id);
    let matched = if nt == NodeType::Closed {
        !node.is_open
    } else {
        true
    };
    if !node.contour.is_empty() && matched {
        paths.push(node.contour.clone());
    }
    for &c in &node.childs {
        add_poly_node_to_paths(tree, c, nt, paths);
    }
}

/// `ClipperLib::PolyTreeToPaths()`.
pub fn poly_tree_to_paths(tree: &PolyTree) -> Paths {
    let mut paths = Paths::new();
    add_poly_node_to_paths(tree, PolyTree::ROOT, NodeType::Any, &mut paths);
    paths
}

/// `ClipperLib::ClosedPathsFromPolyTree()`.
pub fn closed_paths_from_poly_tree(tree: &PolyTree) -> Paths {
    let mut paths = Paths::new();
    add_poly_node_to_paths(tree, PolyTree::ROOT, NodeType::Closed, &mut paths);
    paths
}

/// `ClipperLib::OpenPathsFromPolyTree()`.
pub fn open_paths_from_poly_tree(tree: &PolyTree) -> Paths {
    tree.childs(PolyTree::ROOT)
        .iter()
        .filter(|&&c| tree.is_open(c))
        .map(|&c| tree.contour(c).clone())
        .collect()
}
