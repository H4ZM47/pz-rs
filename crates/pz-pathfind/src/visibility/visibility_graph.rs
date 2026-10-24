//! `VisibilityGraph`, `Obstacle`, `Edge`, `EdgeRing`, `Intersection` and
//! `ClusterOutlineGrid`: the polygonal search graph built around each
//! cluster of vehicles.

use super::{ClusterId, ConnId, EdgeId, EdgeRingId, NodeId, ObstacleId, VGraphId, VehicleId};
use super::{
    VehicleRect, CXN_FLAG_BENDABLE, CXN_FLAG_CAN_PATH, CXN_FLAG_TARGET_PATH, CXN_FLAG_THUMP,
    NODE_FLAG_CRAWL, NODE_FLAG_CRAWL_INTERIOR, NODE_FLAG_PERIMETER, NODE_FLAG_STAIR,
};
use crate::square::bits::*;
use crate::util::{coorddivision, fastfloor, Dir};
use crate::world::{ChunkId, SquareId, World};

/// `ImmutableRectF`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    pub fn intersects(&self, o: &RectF) -> bool {
        self.x < o.x + o.w && self.x + self.w > o.x && self.y < o.y + o.h && self.y + self.h > o.y
    }
}

/// `Intersection`: where two obstacle edges cross.
#[derive(Clone, Copy, Debug)]
pub struct Intersection {
    pub edge1: EdgeId,
    pub edge2: EdgeId,
    pub dist1: f32,
    pub dist2: f32,
    pub node: NodeId,
}

/// `Edge`: one side of an obstacle polygon (or of a graph perimeter, with
/// no obstacle and no ring).
#[derive(Clone, Debug)]
pub struct Edge {
    pub node1: NodeId,
    pub node2: NodeId,
    pub obstacle: Option<ObstacleId>,
    pub edge_ring: Option<EdgeRingId>,
    pub normal: (f32, f32),
    pub intersections: Vec<Intersection>,
}

/// `EdgeRing`: a closed polygon outline.
#[derive(Clone, Debug, Default)]
pub struct EdgeRing {
    pub edges: Vec<EdgeId>,
}

/// `Obstacle`: a polygon (with holes) the paths must go around.
#[derive(Clone, Debug)]
pub struct Obstacle {
    pub vehicle: Option<VehicleId>,
    pub outer: EdgeRingId,
    pub inner: Vec<EdgeRingId>,
    pub bounds: RectF,
    pub node_crawl_front: Option<NodeId>,
    pub node_crawl_rear: Option<NodeId>,
    pub crawl_nodes: Vec<NodeId>,
}

/// `VisibilityGraph`.
#[derive(Clone, Debug)]
pub struct VisibilityGraph {
    pub created: bool,
    pub cluster: ClusterId,
    pub nodes: Vec<NodeId>,
    pub edges: Vec<EdgeId>,
    pub obstacles: Vec<ObstacleId>,
    pub intersect_nodes: Vec<NodeId>,
    pub perimeter_nodes: Vec<NodeId>,
    pub perimeter_edges: Vec<EdgeId>,
    pub overlapped_chunks: Vec<ChunkId>,
}

/// `AdjustStartEndNodeData`: how `getPointOutsideObstacles` moved a start
/// or goal point onto an obstacle edge.
#[derive(Clone, Debug, Default)]
pub struct AdjustStartEndNodeData {
    pub graph: Option<VGraphId>,
    pub obstacle: Option<ObstacleId>,
    pub node: Option<NodeId>,
    pub new_edge: Option<EdgeId>,
    pub is_node_new: bool,
}

/// `ClosestPointOnEdge`.
#[derive(Clone, Copy, Debug)]
struct ClosestPoint {
    edge: Option<EdgeId>,
    node: Option<NodeId>,
    x: f32,
    y: f32,
    dist_sq: f64,
}

impl Default for ClosestPoint {
    fn default() -> Self {
        ClosestPoint {
            edge: None,
            node: None,
            x: 0.0,
            y: 0.0,
            dist_sq: f64::MAX,
        }
    }
}

/// Segment test of `VisibilityGraph::isVisible` / `intersects`: segment
/// `a`-`b` against segment `c`-`d`, with the native operand order and
/// fused operations.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lines_intersect(
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    cx: f32,
    cy: f32,
    dx_: f32,
    dy_: f32,
) -> bool {
    let dx = bx - ax;
    let dy = by - ay;
    let ex = dx_ - cx;
    let ey = dy_ - cy;
    let denom = (-(ex as f64)).mul_add(dy as f64, (dx * ey) as f64);
    if denom == 0.0 {
        return false;
    }
    let ox = ax - cx;
    let oy = ay - cy;
    let t = dx.mul_add(oy, -(ox * dy)) as f64 / denom;
    if !(0.0..=1.0).contains(&t) {
        return false;
    }
    let u = ex.mul_add(oy, -(ox * ey)) as f64 / denom;
    (0.0..=1.0).contains(&u)
}

/// `VisibilityGraph::getIntersection` math for segments 1-2 and 3-4:
/// `(ua, ub, x, y)`.
#[allow(clippy::too_many_arguments)]
fn segment_intersection(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
) -> Option<(f64, f64, f32, f32)> {
    let a = y4 - y3;
    let b = x2 - x1;
    let c = x4 - x3;
    let d = y2 - y1;
    let denom = a.mul_add(b, -(d * c));
    if denom == 0.0 {
        return None;
    }
    let denom = denom as f64;
    let oy = y1 - y3;
    let ox = x1 - x3;
    let ua = c.mul_add(oy, -(ox * a)) as f64 / denom;
    if !(0.0..=1.0).contains(&ua) {
        return None;
    }
    let ub = b.mul_add(oy, -(ox * d)) as f64 / denom;
    if !(0.0..=1.0).contains(&ub) {
        return None;
    }
    let ix = ua.mul_add(b as f64, x1 as f64) as f32;
    let iy = ua.mul_add(d as f64, y1 as f64) as f32;
    Some((ua, ub, ix, iy))
}

/// `ClusterOutline`.
#[derive(Clone, Copy, Default)]
struct ClusterOutline {
    w: bool,
    n: bool,
    e: bool,
    s: bool,
    tw: bool,
    tn: bool,
    te: bool,
    ts: bool,
    inner: bool,
    inner_corner: bool,
    start: bool,
}

/// `ClusterOutlineGrid`: traces the outline of the union of a cluster's
/// (expanded) vehicle rects.
struct ClusterOutlineGrid {
    w: i32,
    h: i32,
    cells: Vec<ClusterOutline>,
}

impl ClusterOutlineGrid {
    fn new(w: i32, h: i32) -> Self {
        ClusterOutlineGrid {
            w,
            h,
            cells: vec![ClusterOutline::default(); (w.max(0) * h.max(0)) as usize],
        }
    }
    fn get(&mut self, x: i32, y: i32) -> Option<&mut ClusterOutline> {
        if x < 0 || x >= self.w || y < 0 || y >= self.h {
            return None;
        }
        Some(&mut self.cells[(x + y * self.w) as usize])
    }
    fn peek(&self, x: i32, y: i32) -> Option<ClusterOutline> {
        if x < 0 || x >= self.w || y < 0 || y >= self.h {
            return None;
        }
        Some(self.cells[(x + y * self.w) as usize])
    }
    fn is_inner(&self, x: i32, y: i32) -> bool {
        self.peek(x, y).is_some_and(|c| c.start || c.inner)
    }
    fn can_trace_w(&self, x: i32, y: i32) -> bool {
        self.peek(x, y).is_some_and(|c| c.inner && c.w && !c.tw)
    }
    fn can_trace_n(&self, x: i32, y: i32) -> bool {
        self.peek(x, y).is_some_and(|c| c.inner && c.n && !c.tn)
    }
    fn can_trace_e(&self, x: i32, y: i32) -> bool {
        self.peek(x, y).is_some_and(|c| c.inner && c.e && !c.te)
    }
    fn can_trace_s(&self, x: i32, y: i32) -> bool {
        self.peek(x, y).is_some_and(|c| c.inner && c.s && !c.ts)
    }
}

/// Which of `ClusterOutlineGrid::trace_W/N/E/S` a trace step is.
#[derive(Clone, Copy)]
enum TraceDir {
    W,
    N,
    E,
    S,
}

impl World {
    // ---- edges ------------------------------------------------------

    /// `Edge::alloc()->init(node1, node2, obstacle, ring)`.
    pub(crate) fn edge_alloc(
        &mut self,
        n1: NodeId,
        n2: NodeId,
        obstacle: Option<ObstacleId>,
        ring: Option<EdgeRingId>,
    ) -> EdgeId {
        let (a, b) = (self.node(n1), self.node(n2));
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = dx.mul_add(dx, dy * dy).sqrt();
        let (nx, ny) = if len >= 0.001 {
            (dx / len, dy / len)
        } else {
            (0.0, 0.0)
        };
        // Rotation by a quarter turn with cos computed in single precision.
        let c = f32::from_bits(0xb33b_bd2e);
        let normal = (nx.mul_add(c, -ny), nx + ny * c);
        let e = self.vis.edges.alloc(Edge {
            node1: n1,
            node2: n2,
            obstacle,
            edge_ring: ring,
            normal,
            intersections: Vec::new(),
        });
        self.node_mut(n1).edges.push(e);
        self.node_mut(n2).edges.push(e);
        e
    }

    /// `Edge::hasNode`.
    pub(crate) fn edge_has_node(&self, e: EdgeId, n: NodeId) -> bool {
        let edge = &self.vis.edges[e];
        edge.node1 == n || edge.node2 == n
    }

    /// `Edge::split`: split `e` at `node`, returning the new second half.
    pub(crate) fn edge_split(&mut self, e: EdgeId, node: NodeId) -> EdgeId {
        let (n1, n2, obstacle, ring) = {
            let ed = &self.vis.edges[e];
            (ed.node1, ed.node2, ed.obstacle, ed.edge_ring)
        };
        let new = self.edge_alloc(node, n2, obstacle, ring);
        if let Some(r) = ring {
            let edges = &mut self.vis.edge_rings[r].edges;
            let i = edges
                .iter()
                .position(|&x| x == e)
                .map(|i| i + 1)
                .unwrap_or(0);
            edges.insert(i, new);
        }
        self.break_connection_between(n1, n2);
        let list = &mut self.node_mut(n2).edges;
        if let Some(i) = list.iter().position(|&x| x == e) {
            list.remove(i);
        }
        self.vis.edges[e].node2 = node;
        self.node_mut(node).edges.push(e);
        new
    }

    /// Closest point on segment `1-2` to `3`, and which end point it is
    /// (1 or 2; 0 for an interior point).
    fn project(x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32) -> ((f64, f64), u8) {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let u = (x3 - x1).mul_add(dx, (y3 - y1) * dy) / dx.mul_add(dx, dy * dy);
        if u <= 0.0 {
            ((x1 as f64, y1 as f64), 1)
        } else if u >= 1.0 {
            ((x2 as f64, y2 as f64), 2)
        } else {
            let u = u as f64;
            (
                (
                    u.mul_add(dx as f64, x1 as f64),
                    u.mul_add(dy as f64, y1 as f64),
                ),
                0,
            )
        }
    }

    /// `Edge::isPointOn`.
    fn edge_is_point_on(&self, e: EdgeId, x3: f32, y3: f32) -> bool {
        let ed = &self.vis.edges[e];
        if !self.node_is_connected_to(ed.node1, ed.node2) {
            return false;
        }
        let (a, b) = (self.node(ed.node1), self.node(ed.node2));
        let (xu, yu) = Self::project(a.x, a.y, b.x, b.y, x3, y3).0;
        let ddx = x3 as f64 - xu;
        let ddy = y3 as f64 - yu;
        ddx.mul_add(ddx, ddy * ddy) < 1.0e-6
    }

    /// `Edge::getClosestPointOnEdge`.
    fn edge_closest_point(&self, e: EdgeId, x3: f32, y3: f32, out: &mut ClosestPoint) {
        let ed = &self.vis.edges[e];
        if !self.node_is_connected_to(ed.node1, ed.node2) {
            return;
        }
        let (a, b) = (self.node(ed.node1), self.node(ed.node2));
        let ((xu, yu), which) = Self::project(a.x, a.y, b.x, b.y, x3, y3);
        let ddx = x3 as f64 - xu;
        let ddy = y3 as f64 - yu;
        let dist = ddx.mul_add(ddx, ddy * ddy);
        if dist < out.dist_sq {
            out.x = xu as f32;
            out.y = yu as f32;
            out.dist_sq = dist;
            out.edge = Some(e);
            out.node = match which {
                1 => Some(ed.node1),
                2 => Some(ed.node2),
                _ => None,
            };
        }
    }

    // ---- edge rings / obstacles ------------------------------------

    fn ring_edges(&self, r: EdgeRingId) -> &[EdgeId] {
        &self.vis.edge_rings[r].edges
    }

    /// `EdgeRing::isPointInPolygon_WindingNumber`: 0 outside, 1 inside,
    /// 2 on an edge (only tested with flag 0x10).
    fn ring_point_in_polygon(&self, r: EdgeRingId, x: f32, y: f32, flags: i32) -> u8 {
        let mut wn = 0i32;
        for &e in self.ring_edges(r) {
            if flags & 0x10 != 0 && self.edge_is_point_on(e, x, y) {
                return 2;
            }
            let ed = &self.vis.edges[e];
            let (a, b) = (self.node(ed.node1), self.node(ed.node2));
            let is_left = (b.x - a.x).mul_add(y - a.y, -((b.y - a.y) * (x - a.x)));
            if a.y <= y {
                if b.y > y && is_left > 0.0 {
                    wn += 1;
                }
            } else if b.y <= y && is_left < 0.0 {
                wn -= 1;
            }
        }
        u8::from(wn != 0)
    }

    fn obstacle_rings(&self, o: ObstacleId) -> Vec<EdgeRingId> {
        let ob = &self.vis.obstacles[o];
        std::iter::once(ob.outer)
            .chain(ob.inner.iter().copied())
            .collect()
    }

    /// `Obstacle::hasNode`.
    pub(crate) fn obstacle_has_node(&self, o: ObstacleId, n: NodeId) -> bool {
        self.obstacle_rings(o)
            .iter()
            .any(|&r| self.ring_edges(r).iter().any(|&e| self.edge_has_node(e, n)))
    }

    /// `Obstacle::hasAdjacentNodes`.
    pub(crate) fn obstacle_has_adjacent_nodes(&self, o: ObstacleId, a: NodeId, b: NodeId) -> bool {
        self.obstacle_rings(o).iter().any(|&r| {
            self.ring_edges(r)
                .iter()
                .any(|&e| self.edge_has_node(e, a) && self.edge_has_node(e, b))
        })
    }

    /// `Obstacle::isPointInside(x, y, flags)`.
    fn obstacle_is_point_inside(&self, o: ObstacleId, x: f32, y: f32, flags: i32) -> bool {
        let ob = &self.vis.obstacles[o];
        if self.ring_point_in_polygon(ob.outer, x, y, flags) != 1 {
            return false;
        }
        ob.inner
            .iter()
            .all(|&r| self.ring_point_in_polygon(r, x, y, flags) == 0)
    }

    /// `Obstacle::isNodeInsideOf`.
    pub(crate) fn obstacle_is_node_inside_of(&self, o: ObstacleId, n: NodeId) -> bool {
        if self.obstacle_has_node(o, n) {
            return false;
        }
        let (x, y) = (self.node(n).x, self.node(n).y);
        if !self.vis.obstacles[o].bounds.contains_point(x, y) {
            return false;
        }
        self.obstacle_is_point_inside(o, x, y, 0)
    }

    /// `Obstacle::calcBounds`.
    fn obstacle_calc_bounds(&mut self, o: ObstacleId) {
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (-f32::MAX, -f32::MAX);
        for &e in self.ring_edges(self.vis.obstacles[o].outer) {
            let n = self.node(self.vis.edges[e].node1);
            if n.x < min_x {
                min_x = n.x;
            }
            if n.y < min_y {
                min_y = n.y;
            }
            if max_x < n.x {
                max_x = n.x;
            }
            if max_y < n.y {
                max_y = n.y;
            }
        }
        self.vis.obstacles[o].bounds = RectF {
            x: min_x + -0.01,
            y: min_y + -0.01,
            w: (max_x - min_x) + 0.02,
            h: (max_y - min_y) + 0.02,
        };
    }

    fn obstacle_alloc(&mut self, vehicle: Option<VehicleId>) -> ObstacleId {
        let outer = self.vis.edge_rings.alloc(EdgeRing::default());
        self.vis.obstacles.alloc(Obstacle {
            vehicle,
            outer,
            inner: Vec::new(),
            bounds: RectF::default(),
            node_crawl_front: None,
            node_crawl_rear: None,
            crawl_nodes: Vec::new(),
        })
    }

    /// `Obstacle::release` (edges and rings; nodes belong to the graph).
    fn obstacle_release(&mut self, o: ObstacleId) {
        for r in self.obstacle_rings(o) {
            let edges = std::mem::take(&mut self.vis.edge_rings[r].edges);
            for e in edges {
                self.vis.edges.release(e);
            }
            self.vis.edge_rings.release(r);
        }
        self.vis.obstacles.release(o);
    }

    /// `Obstacle::getClosestInteriorCrawlNode`.
    fn obstacle_closest_interior_crawl_node(
        &self,
        o: ObstacleId,
        x: f32,
        y: f32,
    ) -> Option<NodeId> {
        let mut best = None;
        let mut best_d = f32::MAX;
        let crawl = &self.vis.obstacles[o].crawl_nodes;
        for i in (0..crawl.len()).step_by(3) {
            let mid = crawl[i + 1];
            let n = self.node(mid);
            let dx = n.x - x;
            let dy = n.y - y;
            let d = dx.mul_add(dx, dy * dy);
            if d < best_d {
                best = Some(mid);
                best_d = d;
            }
        }
        best
    }

    /// `Obstacle::connectCrawlNode`.
    fn obstacle_connect_crawl_node(&mut self, g: VGraphId, o: ObstacleId, n1: NodeId, n2: NodeId) {
        if self.obstacle_is_node_inside_of(o, n2) {
            self.node_mut(n2).flags |= NODE_FLAG_CRAWL_INTERIOR;
            let (x, y) = (self.node(n2).x, self.node(n2).y);
            if let Some(n1) = self.obstacle_closest_interior_crawl_node(o, x, y) {
                if !self.node_is_connected_to(n1, n2) {
                    self.connect_two_nodes(n1, n2, 0);
                }
            }
            return;
        }
        if self.node(n1).ignore || self.node(n2).ignore || self.node_is_connected_to(n1, n2) {
            return;
        }
        if self.graph_is_visible(g, n1, n2) {
            self.connect_two_nodes(n1, n2, 0);
        }
    }

    /// `Obstacle::connectCrawlNodes`.
    fn obstacle_connect_crawl_nodes(&mut self, g: VGraphId, o: ObstacleId, other: ObstacleId) {
        let ob = &self.vis.obstacles[o];
        let (f, r) = (ob.node_crawl_front.unwrap(), ob.node_crawl_rear.unwrap());
        let ob2 = &self.vis.obstacles[other];
        let (f2, r2) = (ob2.node_crawl_front.unwrap(), ob2.node_crawl_rear.unwrap());
        self.obstacle_connect_crawl_node(g, o, f, f2);
        self.obstacle_connect_crawl_node(g, o, f, r2);
        self.obstacle_connect_crawl_node(g, o, r, f2);
        self.obstacle_connect_crawl_node(g, o, r, r2);
        let mine = self.vis.obstacles[o].crawl_nodes.clone();
        let theirs = self.vis.obstacles[other].crawl_nodes.clone();
        for i in (0..mine.len()).step_by(3) {
            let (l, rr) = (mine[i], mine[i + 2]);
            for j in (0..theirs.len()).step_by(3) {
                let (l2, r2) = (theirs[j], theirs[j + 2]);
                self.obstacle_connect_crawl_node(g, o, l, l2);
                self.obstacle_connect_crawl_node(g, o, l, r2);
                self.obstacle_connect_crawl_node(g, o, rr, l2);
                self.obstacle_connect_crawl_node(g, o, rr, r2);
            }
        }
    }

    /// `Obstacle::unsplit`.
    fn obstacle_unsplit(&mut self, node: NodeId, ring: EdgeRingId) {
        let edges = self.vis.edge_rings[ring].edges.clone();
        for (i, &e) in edges.iter().enumerate() {
            if self.vis.edges[e].node1 != node {
                continue;
            }
            if i > 0 {
                let prev = edges[i - 1];
                let n2 = self.vis.edges[e].node2;
                self.vis.edges[prev].node2 = n2;
                let list = &mut self.node_mut(n2).edges;
                if let Some(k) = list.iter().position(|&x| x == e) {
                    list.remove(k);
                }
                self.node_mut(n2).edges.push(prev);
                let (a, b) = (self.vis.edges[prev].node1, self.vis.edges[prev].node2);
                self.connect_two_nodes(a, b, 0);
            } else if edges.len() > 1 {
                let last_n2 = self.vis.edges[edges[edges.len() - 1]].node2;
                self.vis.edges[edges[i + 1]].node1 = last_n2;
            }
            self.vis.edges.release(e);
            self.vis.edge_rings[ring].edges.remove(i);
            break;
        }
    }

    /// Undo the edge split made by `getPointOutsideObstacles` (the
    /// `unsplit` loop at the end of `findPath`).
    pub(crate) fn undo_adjust_start_end(&mut self, data: &AdjustStartEndNodeData) {
        let node = match data.node {
            Some(n) => n,
            None => return,
        };
        let edges = self.node(node).edges.clone();
        for e in edges {
            if let Some(ring) = self.vis.edges[e].edge_ring {
                self.obstacle_unsplit(node, ring);
            }
        }
        if let (Some(g), Some(ne)) = (data.graph, data.new_edge) {
            let list = &mut self.vis.graphs_pool[g].edges;
            if let Some(i) = list.iter().position(|&x| x == ne) {
                list.remove(i);
            }
        }
    }

    // ---- graph queries ----------------------------------------------

    fn graph_rects(&self, g: VGraphId) -> &[VehicleRect] {
        &self.vis.clusters_pool[self.vis.graphs_pool[g].cluster].rects
    }

    fn graph_z(&self, g: VGraphId) -> i32 {
        self.vis.clusters_pool[self.vis.graphs_pool[g].cluster].z
    }

    /// `VisibilityGraph::contains(x, y, z, expand)`.
    pub(crate) fn graph_contains(&self, g: VGraphId, x: f32, y: f32, z: i32, expand: i32) -> bool {
        self.graph_rects(g).iter().any(|r| {
            x >= (r.x - expand) as f32
                && x < (r.right() + expand) as f32
                && y >= (r.y - expand) as f32
                && y < (r.bottom() + expand) as f32
                && z == r.z
        })
    }

    /// `PolygonalMap2::getVisGraphAt(x, y, z, expand)`.
    pub(crate) fn get_vis_graph_at(&self, x: f32, y: f32, z: i32, expand: i32) -> Option<VGraphId> {
        let c = self.get_chunk_from_square_pos(fastfloor(x), fastfloor(y))?;
        self.chunk(c)
            .visibility_graphs
            .iter()
            .copied()
            .find(|&g| self.graph_contains(g, x, y, z, expand))
    }

    /// `PolygonalMap2::getVisGraphForSquare(square, expand)`.
    pub(crate) fn get_vis_graph_for_square(&self, s: SquareId, expand: i32) -> Option<VGraphId> {
        let sq = self.sq(s);
        self.get_vis_graph_at(sq.x as f32 + 0.5, sq.y as f32 + 0.5, sq.z, expand)
    }

    /// `PolygonalMap2::getVisibilityGraphsOverlappingChunk`.
    pub(crate) fn get_visibility_graphs_overlapping_chunk(
        &self,
        c: ChunkId,
        level: i32,
        out: &mut Vec<VGraphId>,
    ) {
        for &g in &self.chunk(c).visibility_graphs {
            if self.graph_z(g) == level && !out.contains(&g) {
                out.push(g);
            }
        }
    }

    /// `PolygonalMap2::getVisibilityGraphsAdjacentToChunk`.
    pub(crate) fn get_visibility_graphs_adjacent_to_chunk(
        &self,
        c: ChunkId,
        level: i32,
        out: &mut Vec<VGraphId>,
    ) {
        let ch = self.chunk(c);
        let (wx, wy) = (ch.wx, ch.wy);
        let (x1, y1, x2, y2) = (
            ch.min_x() - 1,
            ch.min_y() - 1,
            ch.max_x() + 2,
            ch.max_y() + 2,
        );
        for d in Dir::ALL {
            let c2 = match self.get_chunk_from_chunk_pos(wx + d.dx(), wy + d.dy()) {
                Some(c2) => c2,
                None => continue,
            };
            for &g in &self.chunk(c2).visibility_graphs {
                if self.graph_z(g) != level || out.contains(&g) {
                    continue;
                }
                let b = self.cluster_bounds(self.vis.graphs_pool[g].cluster);
                if b.x - 1 < x2 && b.right() + 1 > x1 && b.y - 1 < y2 && b.bottom() + 1 > y1 {
                    out.push(g);
                }
            }
        }
    }

    // ---- visibility ---------------------------------------------------

    /// `VisibilityGraph::isVisible`.
    pub(crate) fn graph_is_visible(&self, g: VGraphId, n1: NodeId, n2: NodeId) -> bool {
        if self.node_shares_edge(n1, n2) {
            return !self.node_on_same_shape_but_does_not_share_an_edge(n1, n2);
        }
        if self.node_shares_shape(n1, n2) {
            return false;
        }
        let gr = &self.vis.graphs_pool[g];
        let (a, b) = (self.node(n1), self.node(n2));
        for &e in gr.edges.iter().chain(gr.perimeter_edges.iter()) {
            if self.edge_has_node(e, n1) || self.edge_has_node(e, n2) {
                continue;
            }
            let ed = &self.vis.edges[e];
            let (c, d) = (self.node(ed.node1), self.node(ed.node2));
            if lines_intersect(a.x, a.y, b.x, b.y, c.x, c.y, d.x, d.y) {
                return false;
            }
        }
        true
    }

    /// `VisibilityGraph::addNode`.
    pub(crate) fn graph_add_node(&mut self, g: VGraphId, n: NodeId) {
        if self.vis.graphs_pool[g].created && !self.node(n).ignore {
            let gr = &self.vis.graphs_pool[g];
            let all: Vec<NodeId> = gr
                .nodes
                .iter()
                .chain(gr.perimeter_nodes.iter())
                .copied()
                .collect();
            for n1 in all {
                if !self.node(n1).ignore && self.graph_is_visible(g, n1, n) {
                    self.connect_two_nodes(n, n1, 0);
                }
            }
        }
        self.vis.graphs_pool[g].nodes.push(n);
    }

    /// `VisibilityGraph::removeNode`.
    pub(crate) fn graph_remove_node(&mut self, g: VGraphId, n: NodeId) {
        let list = &mut self.vis.graphs_pool[g].nodes;
        if let Some(i) = list.iter().position(|&x| x == n) {
            list.remove(i);
        }
        while let Some(&c) = self.node(n).visible.last() {
            self.break_connection(c);
        }
    }

    /// `VisibilityGraph::getPointOutsideObstacles`: 1 if the point was
    /// inside an obstacle and was moved onto its nearest edge, 0 if it is
    /// not inside any obstacle.
    pub(crate) fn graph_get_point_outside_obstacles(
        &mut self,
        g: VGraphId,
        x: f32,
        y: f32,
        z: f32,
        data: &mut AdjustStartEndNodeData,
    ) -> i32 {
        let mut best = ClosestPoint::default();
        let obstacles = self.vis.graphs_pool[g].obstacles.clone();
        for o in obstacles {
            if !self.vis.obstacles[o].bounds.contains_point(x, y)
                || !self.obstacle_is_point_inside(o, x, y, 0)
            {
                continue;
            }
            let mut cp = ClosestPoint::default();
            for r in self.obstacle_rings(o) {
                for &e in self.ring_edges(r) {
                    self.edge_closest_point(e, x, y, &mut cp);
                }
            }
            if cp.edge.is_some() && cp.dist_sq < best.dist_sq {
                best = cp;
            }
        }
        let edge = match best.edge {
            Some(e) => e,
            None => return 0,
        };
        data.obstacle = self.vis.edges[edge].obstacle;
        match best.node {
            None => {
                let n = self.node_alloc(best.x, best.y, fastfloor(z));
                data.node = Some(n);
                data.new_edge = Some(self.edge_split(edge, n));
                data.is_node_new = true;
            }
            Some(n) => {
                data.node = Some(n);
                data.new_edge = None;
                data.is_node_new = false;
            }
        }
        data.graph = Some(g);
        if data.is_node_new {
            let ne = data.new_edge.unwrap();
            self.vis.graphs_pool[g].edges.push(ne);
            self.graph_add_node(g, data.node.unwrap());
        }
        1
    }

    // ---- clusters ----------------------------------------------------

    /// `VehicleCluster::bounds`.
    pub(crate) fn cluster_bounds(&self, c: ClusterId) -> VehicleRect {
        let cl = &self.vis.clusters_pool[c];
        if cl.rects.is_empty() {
            return VehicleRect {
                x: i32::MAX,
                y: i32::MAX,
                w: 1,
                h: 1,
                z: cl.z,
                ..Default::default()
            };
        }
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for r in &cl.rects {
            min_x = min_x.min(r.x);
            min_y = min_y.min(r.y);
            max_x = max_x.max(r.right());
            max_y = max_y.max(r.bottom());
        }
        VehicleRect {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
            z: cl.z,
            ..Default::default()
        }
    }

    /// `PolygonalMap2::createVehicleClusters` / `createVehicleCluster`.
    fn create_vehicle_clusters(&mut self) {
        self.vis.clusters.clear();
        let mut rects: Vec<VehicleRect> = Vec::new();
        for &v in &self.vis.vehicles {
            let mut r = self.vis.vehicles_pool[v].poly_plus_radius.aabb();
            r.vehicle = Some(v);
            rects.push(r);
        }
        // Clusters as lists of rect indices while grouping; `live` is the
        // cluster list in creation order with merged clusters removed.
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut live: Vec<usize> = Vec::new();
        let mut of: Vec<Option<usize>> = vec![None; rects.len()];
        let adjacent = |a: &VehicleRect, b: &VehicleRect| {
            a.x - 1 < b.right()
                && a.right() + 1 > b.x
                && a.y - 1 < b.bottom()
                && a.bottom() + 1 > b.y
        };
        for i in 0..rects.len() {
            for j in 0..rects.len() {
                if i == j || rects[i].z != rects[j].z {
                    continue;
                }
                if of[i].is_some() && of[i] == of[j] {
                    continue;
                }
                if !adjacent(&rects[i], &rects[j]) {
                    continue;
                }
                match (of[i], of[j]) {
                    (Some(ci), None) => {
                        of[j] = Some(ci);
                        groups[ci].push(j);
                    }
                    (Some(ci), Some(cj)) => {
                        live.retain(|&x| x != cj);
                        let moved = std::mem::take(&mut groups[cj]);
                        for &m in &moved {
                            of[m] = Some(ci);
                        }
                        groups[ci].extend(moved);
                    }
                    (None, Some(cj)) => {
                        of[i] = Some(cj);
                        groups[cj].push(i);
                    }
                    (None, None) => {
                        groups.push(vec![i, j]);
                        let ci = groups.len() - 1;
                        of[i] = Some(ci);
                        of[j] = Some(ci);
                        live.push(ci);
                    }
                }
            }
            if of[i].is_none() {
                groups.push(vec![i]);
                let ci = groups.len() - 1;
                of[i] = Some(ci);
                live.push(ci);
            }
        }
        for ci in live {
            let z = rects[groups[ci][0]].z;
            let cl = self.vis.clusters_pool.alloc(super::VehicleCluster {
                z,
                rects: groups[ci].iter().map(|&i| rects[i]).collect(),
            });
            self.vis.clusters.push(cl);
        }
    }

    // ---- building ----------------------------------------------------

    /// `PolygonalMap2::createVisibilityGraphs`. The graphs' obstacles and
    /// connections are built lazily by [`World::graph_create`].
    pub(crate) fn create_visibility_graphs(&mut self) {
        self.create_vehicle_clusters();
        let clusters = self.vis.clusters.clone();
        for cl in clusters {
            let g = self.vis.graphs_pool.alloc(VisibilityGraph {
                created: false,
                cluster: cl,
                nodes: Vec::new(),
                edges: Vec::new(),
                obstacles: Vec::new(),
                intersect_nodes: Vec::new(),
                perimeter_nodes: Vec::new(),
                perimeter_edges: Vec::new(),
                overlapped_chunks: Vec::new(),
            });
            self.graph_add_perimeter_edges(g);
            self.graph_init_overlapped_chunks(g);
            self.vis.graphs.push(g);
        }
        self.add_stair_nodes();
        self.add_can_path_nodes();
    }

    /// `VisibilityGraph::initOverlappedChunks`.
    fn graph_init_overlapped_chunks(&mut self, g: VGraphId) {
        let b = self.cluster_bounds(self.vis.graphs_pool[g].cluster);
        let min_cx = coorddivision(b.x - 1, 8);
        let min_cy = coorddivision(b.y - 1, 8);
        let max_cx = (b.right() as f32 / 8.0).ceil() as i32;
        let max_cy = (b.bottom() as f32 / 8.0).ceil() as i32;
        for cy in min_cy..max_cy {
            for cx in min_cx..max_cx {
                let c = match self.get_chunk_from_chunk_pos(cx, cy) {
                    Some(c) => c,
                    None => continue,
                };
                let ch = self.chunk(c);
                let (x1, y1, x2, y2) = (ch.min_x(), ch.min_y(), ch.max_x() + 1, ch.max_y() + 1);
                let hit = self.graph_rects(g).iter().any(|r| {
                    r.x - 1 < x2 && r.right() + 1 > x1 && r.y - 1 < y2 && r.bottom() + 1 > y1
                });
                if !hit {
                    continue;
                }
                self.vis.graphs_pool[g].overlapped_chunks.push(c);
                let list = &mut self.chunks[c].visibility_graphs;
                if !list.contains(&g) {
                    list.push(g);
                }
            }
        }
    }

    /// `VisibilityGraph::addPerimeterEdges`.
    fn graph_add_perimeter_edges(&mut self, g: VGraphId) {
        let cl = self.vis.graphs_pool[g].cluster;
        let mut b = self.cluster_bounds(cl);
        b.x -= 1;
        b.y -= 1;
        b.w += 2;
        b.h += 2;
        let z = self.vis.clusters_pool[cl].z;
        let mut cog = ClusterOutlineGrid::new(b.w, b.h);
        for r in self.vis.clusters_pool[cl].rects.clone() {
            for y in r.y - 1..r.y + r.h + 1 {
                for x in r.x - 1..r.x + r.w + 1 {
                    if let Some(c) = cog.get(x - b.x, y - b.y) {
                        c.inner = true;
                    }
                }
            }
        }
        for y in 0..b.h {
            for x in 0..b.w {
                if !cog.peek(x, y).unwrap().inner {
                    continue;
                }
                let (w, n, e, s) = (
                    !cog.is_inner(x - 1, y),
                    !cog.is_inner(x, y - 1),
                    !cog.is_inner(x + 1, y),
                    !cog.is_inner(x, y + 1),
                );
                let c = cog.get(x, y).unwrap();
                c.w |= w;
                c.n |= n;
                c.e |= e;
                c.s |= s;
            }
        }
        for y in 0..b.h {
            for x in 0..b.w {
                let f = cog.peek(x, y).unwrap();
                if f.w || f.n || f.e || f.s || f.inner_corner {
                    if let Some(s) = self.get_square(b.x + x, b.y + y, z) {
                        if !self.sq(s).is_non_thumpable_solid(self) && !self.sq(s).has(ALL_STAIRS) {
                            let n = self.get_node_for_square(s);
                            self.node_mut(n).flags |= NODE_FLAG_PERIMETER;
                            self.node_mut(n).graphs.push(g);
                            self.vis.graphs_pool[g].perimeter_nodes.push(n);
                        }
                    }
                }
                if !(f.n && f.w && f.inner) || f.tw || f.tn || f.te || f.ts {
                    continue;
                }
                let nodes = self.cog_trace(&mut cog, x, y, z);
                for i in 0..nodes.len() - 1 {
                    let (n1, n2) = (nodes[i], nodes[i + 1]);
                    self.node_mut(n1).x += b.x as f32;
                    self.node_mut(n1).y += b.y as f32;
                    let e = self.edge_alloc(n1, n2, None, None);
                    self.vis.graphs_pool[g].perimeter_edges.push(e);
                }
                let last = *nodes.last().unwrap();
                if last != nodes[0] {
                    self.node_mut(last).x += b.x as f32;
                    self.node_mut(last).y += b.y as f32;
                }
            }
        }
    }

    /// `ClusterOutlineGrid::trace`.
    fn cog_trace(&mut self, cog: &mut ClusterOutlineGrid, x: i32, y: i32, z: i32) -> Vec<NodeId> {
        let first = self.node_alloc(x as f32, y as f32, z);
        let mut nodes = vec![first];
        cog.get(x, y).unwrap().start = true;
        let mut step = Some((TraceDir::N, x, y, false));
        while let Some((dir, cx, cy, extend)) = step {
            step = self.cog_trace_step(cog, dir, cx, cy, extend, &mut nodes, z);
        }
        let last = *nodes.last().unwrap();
        let (n1, nn) = (self.node(first), self.node(last));
        if (n1.x + 0.1) as i32 == (nn.x + 0.1) as i32 && (n1.y + 0.1) as i32 == (nn.y + 0.1) as i32
        {
            let len = nodes.len();
            if last != first {
                self.node_release(last);
            }
            nodes[len - 1] = first;
        }
        nodes
    }

    /// One `trace_W/N/E/S` call; returns the call it makes next.
    #[allow(clippy::too_many_arguments)]
    fn cog_trace_step(
        &mut self,
        cog: &mut ClusterOutlineGrid,
        dir: TraceDir,
        x: i32,
        y: i32,
        extend: bool,
        nodes: &mut Vec<NodeId>,
        z: i32,
    ) -> Option<(TraceDir, i32, i32, bool)> {
        let (px, py) = match dir {
            TraceDir::W => (x, y),
            TraceDir::N => (x + 1, y),
            TraceDir::E => (x + 1, y + 1),
            TraceDir::S => (x, y + 1),
        };
        if extend {
            let last = *nodes.last().unwrap();
            self.node_mut(last).x = px as f32;
            self.node_mut(last).y = py as f32;
        } else {
            nodes.push(self.node_alloc(px as f32, py as f32, z));
        }
        let c = cog.get(x, y).unwrap();
        match dir {
            TraceDir::W => {
                c.tw = true;
                if cog.can_trace_s(x - 1, y - 1) {
                    cog.get(x, y - 1).unwrap().inner_corner = true;
                    Some((TraceDir::S, x - 1, y - 1, false))
                } else if cog.can_trace_w(x, y - 1) {
                    Some((TraceDir::W, x, y - 1, true))
                } else if cog.can_trace_n(x, y) {
                    Some((TraceDir::N, x, y, false))
                } else {
                    None
                }
            }
            TraceDir::N => {
                c.tn = true;
                if cog.can_trace_w(x + 1, y - 1) {
                    cog.get(x + 1, y).unwrap().inner_corner = true;
                    Some((TraceDir::W, x + 1, y - 1, false))
                } else if cog.can_trace_n(x + 1, y) {
                    Some((TraceDir::N, x + 1, y, true))
                } else if cog.can_trace_e(x, y) {
                    Some((TraceDir::E, x, y, false))
                } else {
                    None
                }
            }
            TraceDir::E => {
                c.te = true;
                if cog.can_trace_n(x + 1, y + 1) {
                    cog.get(x, y + 1).unwrap().inner_corner = true;
                    Some((TraceDir::N, x + 1, y + 1, false))
                } else if cog.can_trace_e(x, y + 1) {
                    Some((TraceDir::E, x, y + 1, true))
                } else if cog.can_trace_s(x, y) {
                    Some((TraceDir::S, x, y, false))
                } else {
                    None
                }
            }
            TraceDir::S => {
                c.ts = true;
                if cog.can_trace_e(x - 1, y + 1) {
                    cog.get(x - 1, y).unwrap().inner_corner = true;
                    Some((TraceDir::E, x - 1, y + 1, false))
                } else if cog.can_trace_s(x - 1, y) {
                    Some((TraceDir::S, x - 1, y, true))
                } else if cog.can_trace_w(x, y) {
                    Some((TraceDir::W, x, y, false))
                } else {
                    None
                }
            }
        }
    }

    /// Squares of a graph's cluster bounds grown by `grow` on the west and
    /// north and to `right + extra` / `bottom + extra` (shared by
    /// `getStairSquares` and `getCanPathSquares`).
    fn graph_squares(
        &self,
        g: VGraphId,
        grow: i32,
        extra: i32,
        pred: impl Fn(&World, SquareId) -> bool,
        out: &mut Vec<SquareId>,
    ) {
        let b = self.cluster_bounds(self.vis.graphs_pool[g].cluster);
        let z = self.graph_z(g);
        for y in b.y - grow..b.bottom() + extra {
            for x in b.x - grow..b.right() + extra {
                if let Some(s) = self.get_square(x, y, z) {
                    if pred(self, s) && !out.contains(&s) {
                        out.push(s);
                    }
                }
            }
        }
    }

    /// Mark a node ignored when it lies inside any of the graph's
    /// obstacles.
    fn ignore_if_inside_obstacles(&mut self, n: NodeId, g: VGraphId) {
        let obstacles = self.vis.graphs_pool[g].obstacles.clone();
        for o in obstacles {
            if self.obstacle_is_node_inside_of(o, n) {
                self.node_mut(n).ignore = true;
            }
        }
    }

    /// `PolygonalMap2::addStairNodes`.
    fn add_stair_nodes(&mut self) {
        let mut squares = Vec::new();
        for g in self.vis.graphs.clone() {
            self.graph_squares(
                g,
                4,
                1,
                |w, s| w.sq(s).has(STAIR_TW | STAIR_TN),
                &mut squares,
            );
        }
        for s in squares {
            let sq = self.sq(s).clone();
            let mut ends = None;
            if sq.has(STAIR_TW) {
                ends = Some([
                    self.get_square(sq.x - 1, sq.y, sq.z + 1),
                    self.get_square(sq.x + 1, sq.y, sq.z),
                    self.get_square(sq.x + 2, sq.y, sq.z),
                    self.get_square(sq.x + 3, sq.y, sq.z),
                ]);
            }
            if sq.has(STAIR_TN) {
                ends = Some([
                    self.get_square(sq.x, sq.y - 1, sq.z + 1),
                    self.get_square(sq.x, sq.y + 1, sq.z),
                    self.get_square(sq.x, sq.y + 2, sq.z),
                    self.get_square(sq.x, sq.y + 3, sq.z),
                ]);
            }
            let (top, bottom) = match ends {
                Some([Some(t), Some(_), Some(_), Some(b)]) => (t, b),
                _ => continue,
            };
            for end in [top, bottom] {
                let node = match self.get_vis_graph_for_square(end, 0) {
                    None => self.get_node_for_square(end),
                    Some(g) => {
                        let n = self.node_alloc_square(end);
                        self.ignore_if_inside_obstacles(n, g);
                        self.node_mut(n).graphs.push(g);
                        self.graph_add_node(g, n);
                        self.square_to_node.insert(end.raw(), n);
                        n
                    }
                };
                self.node_mut(node).flags |= NODE_FLAG_STAIR;
            }
        }
    }

    /// `PolygonalMap2::getOrCreateCanPathNode` (the native version tests
    /// every obstacle).
    fn get_or_create_can_path_node(&mut self, s: SquareId) -> NodeId {
        let g = self.get_vis_graph_for_square(s, 0);
        let n = self.get_node_for_square(s);
        if let Some(g) = g {
            if !self.vis.graphs_pool[g].nodes.contains(&n) {
                self.ignore_if_inside_obstacles(n, g);
                self.graph_add_node(g, n);
            }
        }
        n
    }

    /// `PolygonalMap2::addCanPathNodes`, with the native connection flags
    /// (thump, target-path and bendable).
    fn add_can_path_nodes(&mut self) {
        let mut squares = Vec::new();
        for g in self.vis.graphs.clone() {
            self.graph_squares(
                g,
                1,
                1,
                |w, s| w.sq(s).is_can_path_w(w) || w.sq(s).is_can_path_n(w),
                &mut squares,
            );
        }
        let ok = |w: &World, s: SquareId| {
            let sq = w.sq(s);
            !sq.is_non_thumpable_solid(w) && !sq.has(ALL_STAIRS) && sq.has(SOLID_FLOOR)
        };
        for s1 in squares {
            if !ok(self, s1) {
                continue;
            }
            let (x, y, z) = (self.sq(s1).x, self.sq(s1).y, self.sq(s1).z);
            for (west, dx, dy, thump, target, bend) in [
                (true, -1, 0, THUMP_W, TARGET_PATH_W, BENDABLE_W),
                (false, 0, -1, THUMP_N, TARGET_PATH_N, BENDABLE_N),
            ] {
                let can = if west {
                    self.sq(s1).is_can_path_w(self)
                } else {
                    self.sq(s1).is_can_path_n(self)
                };
                if !can {
                    continue;
                }
                let s2 = match self.get_square(x + dx, y + dy, z) {
                    Some(s2) if ok(self, s2) => s2,
                    _ => continue,
                };
                let n1 = self.get_or_create_can_path_node(s1);
                let n2 = self.get_or_create_can_path_node(s2);
                let mut flags = CXN_FLAG_CAN_PATH;
                if self.sq(s1).has(thump | THUMPABLE) || self.sq(s2).has(THUMPABLE) {
                    flags |= CXN_FLAG_THUMP;
                }
                if self.sq(s1).has(target) {
                    flags |= CXN_FLAG_TARGET_PATH;
                }
                if self.sq(s1).has(bend) {
                    flags |= CXN_FLAG_BENDABLE;
                }
                self.connect_two_nodes(n1, n2, flags);
            }
        }
    }

    /// `VisibilityGraph::release`.
    pub(crate) fn graph_release(&mut self, g: VGraphId) {
        let gr = self.vis.graphs_pool[g].clone();
        let square_nodes: std::collections::HashSet<NodeId> =
            self.square_to_node.values().copied().collect();
        for &n in &gr.nodes {
            if !square_nodes.contains(&n) {
                self.node_release(n);
            }
        }
        for &e in &gr.perimeter_edges {
            let n1 = self.vis.edges[e].node1;
            self.node_release(n1);
            self.vis.edges.release(e);
        }
        for &o in &gr.obstacles {
            self.obstacle_release(o);
        }
        self.vis.clusters_pool.release(gr.cluster);
        for &c in &gr.overlapped_chunks {
            self.chunks[c].visibility_graphs.retain(|&x| x != g);
        }
        self.vis.graphs_pool.release(g);
    }

    /// `VisibilityGraph::create`: build the obstacles and the node
    /// connections.
    pub(crate) fn graph_create(&mut self, g: VGraphId) {
        let rects = self.graph_rects(g).to_vec();
        for r in rects {
            if let Some(v) = r.vehicle {
                self.graph_add_edges_for_vehicle(g, v);
            }
        }
        self.graph_add_world_obstacles(g);
        for o in self.vis.graphs_pool[g].obstacles.clone() {
            if self.vis.obstacles[o].vehicle.is_some() {
                continue;
            }
            for r in self.obstacle_rings(o) {
                self.graph_split_world_obstacle_edges(g, r);
            }
        }
        self.graph_check_edge_intersection(g);
        self.graph_check_nodes_in_obstacles(g);
        self.graph_calculate_node_visibility(g);
        self.graph_connect_vehicle_crawl_nodes(g);
        self.vis.graphs_pool[g].created = true;
    }

    /// `VisibilityGraph::addEdgesForVehicle`.
    fn graph_add_edges_for_vehicle(&mut self, g: VGraphId, v: VehicleId) {
        let veh = self.vis.vehicles_pool[v].clone();
        let p = veh.poly_plus_radius;
        let z = fastfloor(p.z);
        let fr = self.node_alloc(p.x1, p.y1, z);
        let fl = self.node_alloc(p.x2, p.y2, z);
        let rl = self.node_alloc(p.x3, p.y3, z);
        let rr = self.node_alloc(p.x4, p.y4, z);
        let o = self.obstacle_alloc(Some(v));
        self.vis.graphs_pool[g].obstacles.push(o);
        let outer = self.vis.obstacles[o].outer;
        let e_front = self.edge_alloc(fr, fl, Some(o), Some(outer));
        let e_left = self.edge_alloc(fl, rl, Some(o), Some(outer));
        let e_rear = self.edge_alloc(rl, rr, Some(o), Some(outer));
        let mut e_right = self.edge_alloc(rr, fr, Some(o), Some(outer));
        self.vis.edge_rings[outer]
            .edges
            .extend([e_front, e_left, e_rear, e_right]);
        self.obstacle_calc_bounds(o);
        self.vis.graphs_pool[g].nodes.extend([fr, fl, rl, rr]);
        self.vis.graphs_pool[g]
            .edges
            .extend([e_front, e_left, e_rear, e_right]);
        if veh.up_vector_dot < 0.95 {
            return;
        }
        let front = self.node_alloc((p.x1 + p.x2) * 0.5, (p.y1 + p.y2) * 0.5, z);
        let rear = self.node_alloc((p.x3 + p.x4) * 0.5, (p.y3 + p.y4) * 0.5, z);
        self.node_mut(front).flags |= NODE_FLAG_CRAWL;
        self.node_mut(rear).flags |= NODE_FLAG_CRAWL;
        self.vis.obstacles[o].node_crawl_front = Some(front);
        self.vis.obstacles[o].node_crawl_rear = Some(rear);
        self.vis.graphs_pool[g].nodes.extend([front, rear]);
        let c1 = self.edge_split(e_front, front);
        let c3 = self.edge_split(e_rear, rear);
        self.vis.graphs_pool[g].edges.extend([c1, c3]);
        self.vis.obstacles[o].crawl_nodes.clear();
        let lerp = |a: f32, b: f32, t: f32| a + t * (b - a);
        for &frac in &veh.crawl_offsets {
            let left = self.node_alloc(lerp(p.x3, p.x2, frac), lerp(p.y3, p.y2, frac), z);
            self.node_mut(left).flags |= NODE_FLAG_CRAWL;
            let right = self.node_alloc(lerp(p.x4, p.x1, frac), lerp(p.y4, p.y1, frac), z);
            self.node_mut(right).flags |= NODE_FLAG_CRAWL;
            let (l, r) = (self.node(left).clone(), self.node(right).clone());
            let mid = self.node_alloc((l.x + r.x) * 0.5, (l.y + r.y) * 0.5, z);
            self.node_mut(mid).flags |= NODE_FLAG_CRAWL | NODE_FLAG_CRAWL_INTERIOR;
            self.vis.obstacles[o].crawl_nodes.extend([left, mid, right]);
            self.vis.graphs_pool[g].nodes.extend([left, mid, right]);
            let l2 = self.edge_split(e_left, left);
            e_right = self.edge_split(e_right, right);
            self.vis.graphs_pool[g].edges.extend([l2, e_right]);
        }
    }

    /// `VisibilityGraph::addWorldObstaclesClipper`.
    fn graph_add_world_obstacles(&mut self, g: VGraphId) {
        let cl = self.vis.graphs_pool[g].cluster;
        let mut b = self.cluster_bounds(cl);
        b.x -= 1;
        b.y -= 1;
        b.w += 2;
        b.h += 2;
        let z = self.vis.clusters_pool[cl].z;
        let r = crate::line_clear_collide::RADIUS;
        let mut clip = std::mem::take(&mut self.vis.clip);
        clip.clear();
        for y in b.y..b.bottom() {
            for x in b.x..b.right() {
                let s = match self.get_square(x, y, z) {
                    Some(s) => s,
                    None => continue,
                };
                let sq = self.sq(s);
                if !self.graph_contains(g, sq.x as f32 + 0.5, sq.y as f32 + 0.5, sq.z, 1) {
                    continue;
                }
                let (fx, fy) = (x as f32, y as f32);
                if sq.has(ALL_STAIRS) || sq.is_really_solid(self) {
                    clip.add_aabb(fx - r, fy - r, r + (x + 1) as f32, r + (y + 1) as f32);
                }
                let mut col_w =
                    sq.has(COLLIDE_W) || sq.is_sloped_surface_edge_blocked(self, Dir::W);
                if let Some(w) = self.adjacent_square(s, Dir::W) {
                    col_w |= self.sq(w).is_sloped_surface_edge_blocked(self, Dir::E);
                }
                if col_w {
                    clip.add_aabb(fx - r, fy - r, r + fx, r + (y + 1) as f32);
                }
                let mut col_n =
                    sq.has(COLLIDE_N) || sq.is_sloped_surface_edge_blocked(self, Dir::N);
                if let Some(n) = self.adjacent_square(s, Dir::N) {
                    col_n |= self.sq(n).is_sloped_surface_edge_blocked(self, Dir::S);
                }
                if col_n {
                    clip.add_aabb(fx - r, fy - r, r + (x + 1) as f32, r + fy);
                }
            }
        }
        let polys = clip.generate_polygons();
        self.vis.clip = clip;
        for poly in polys {
            let o = self.obstacle_alloc(None);
            self.graph_edges_from_polygon(g, &poly.outer, o, true, z);
            for hole in &poly.holes {
                self.graph_edges_from_polygon(g, hole, o, false, z);
            }
            self.obstacle_calc_bounds(o);
            self.vis.graphs_pool[g].obstacles.push(o);
            for r in self.obstacle_rings(o) {
                let edges = self.vis.edge_rings[r].edges.clone();
                self.vis.graphs_pool[g].edges.extend(edges);
            }
        }
    }

    /// `VisibilityGraph::getEdgesFromPolygon` (the native code walks the
    /// points in reverse).
    fn graph_edges_from_polygon(
        &mut self,
        g: VGraphId,
        pts: &[(f32, f32)],
        o: ObstacleId,
        outer: bool,
        z: i32,
    ) {
        if pts.len() < 3 {
            return;
        }
        let ring = if outer {
            self.vis.obstacles[o].outer
        } else {
            let r = self.vis.edge_rings.alloc(EdgeRing::default());
            self.vis.obstacles[o].inner.push(r);
            r
        };
        let first = self.vis.graphs_pool[g].nodes.len();
        for &(x, y) in pts.iter().rev() {
            let n = self.node_alloc(x, y, z);
            self.vis.graphs_pool[g].nodes.push(n);
        }
        let count = self.vis.graphs_pool[g].nodes.len();
        for j in first..count - 1 {
            let (n1, n2) = (
                self.vis.graphs_pool[g].nodes[j],
                self.vis.graphs_pool[g].nodes[j + 1],
            );
            let (x, y, nz) = (self.node(n1).x, self.node(n1).y, self.node(n1).z);
            if !self.graph_contains(g, x, y, nz, 0) {
                self.node_mut(n1).ignore = true;
            }
            let e = self.edge_alloc(n1, n2, Some(o), Some(ring));
            self.vis.edge_rings[ring].edges.push(e);
        }
        let (n1, n2) = (
            self.vis.graphs_pool[g].nodes[count - 1],
            self.vis.graphs_pool[g].nodes[first],
        );
        let e = self.edge_alloc(n1, n2, Some(o), Some(ring));
        self.vis.edge_rings[ring].edges.push(e);
    }

    /// `VisibilityGraph::trySplit`.
    fn graph_try_split(&self, g: VGraphId, e: EdgeId, r: &VehicleRect, split: &mut Vec<i32>) {
        let ed = &self.vis.edges[e];
        let (a, b) = (self.node(ed.node1), self.node(ed.node2));
        let z = self.graph_z(g);
        if (a.x - b.x).abs() > (a.y - b.y).abs() {
            let (left, right) = (a.x.min(b.x), a.x.max(b.x));
            let ey = a.y;
            for (v, off) in [(r.left(), -0.5), (r.right(), 0.5)] {
                if (v as f32) > left
                    && (v as f32) < right
                    && (r.top() as f32) < ey
                    && (r.bottom() as f32) > ey
                    && !split.contains(&v)
                    && !self.graph_contains(g, v as f32 + off, ey, z, 0)
                {
                    split.push(v);
                }
            }
        } else {
            let (top, bottom) = (a.y.min(b.y), a.y.max(b.y));
            let ex = a.x;
            for (v, off) in [(r.top(), -0.5), (r.bottom(), 0.5)] {
                if (v as f32) > top
                    && (v as f32) < bottom
                    && (r.left() as f32) < ex
                    && (r.right() as f32) > ex
                    && !split.contains(&v)
                    && !self.graph_contains(g, ex, v as f32 + off, z, 0)
                {
                    split.push(v);
                }
            }
        }
    }

    /// `VisibilityGraph::splitWorldObstacleEdges`.
    fn graph_split_world_obstacle_edges(&mut self, g: VGraphId, ring: EdgeRingId) {
        let z = self.graph_z(g);
        let rects = self.graph_rects(g).to_vec();
        let n = self.vis.edge_rings[ring].edges.len();
        for j in (0..n).rev() {
            let e = self.vis.edge_rings[ring].edges[j];
            let mut split = Vec::new();
            for r in &rects {
                self.graph_try_split(g, e, r, &mut split);
            }
            if split.is_empty() {
                continue;
            }
            split.sort_unstable();
            let ed = self.vis.edges[e].clone();
            let (a, b) = (self.node(ed.node1).clone(), self.node(ed.node2).clone());
            let horizontal = (a.x - b.x).abs() > (a.y - b.y).abs();
            let ascending = if horizontal { a.x < b.x } else { a.y < b.y };
            if ascending {
                split.reverse();
            }
            for v in split {
                let node = if horizontal {
                    self.node_alloc(v as f32, a.y, z)
                } else {
                    self.node_alloc(a.x, v as f32, z)
                };
                let ne = self.edge_split(e, node);
                self.vis.graphs_pool[g].nodes.push(node);
                self.vis.graphs_pool[g].edges.push(ne);
            }
        }
    }

    /// `VisibilityGraph::checkEdgeIntersection`.
    fn graph_check_edge_intersection(&mut self, g: VGraphId) {
        let obstacles = self.vis.graphs_pool[g].obstacles.clone();
        for i in 0..obstacles.len() {
            for &o2 in &obstacles[i + 1..] {
                let o1 = obstacles[i];
                if !self.vis.obstacles[o1]
                    .bounds
                    .intersects(&self.vis.obstacles[o2].bounds)
                {
                    continue;
                }
                let (out1, out2) = (self.vis.obstacles[o1].outer, self.vis.obstacles[o2].outer);
                let in1 = self.vis.obstacles[o1].inner.clone();
                let in2 = self.vis.obstacles[o2].inner.clone();
                self.graph_check_ring_intersection(g, out1, out2);
                for &r2 in &in2 {
                    self.graph_check_ring_intersection(g, out1, r2);
                }
                for &r1 in &in1 {
                    self.graph_check_ring_intersection(g, r1, out2);
                    for &r2 in &in2 {
                        self.graph_check_ring_intersection(g, r1, r2);
                    }
                }
            }
        }
        for o in obstacles {
            for r in self.obstacle_rings(o) {
                self.graph_check_edge_intersection_split(g, r);
            }
        }
    }

    /// The edge-pair loop of `checkEdgeIntersection` for two rings.
    fn graph_check_ring_intersection(&mut self, g: VGraphId, r1: EdgeRingId, r2: EdgeRingId) {
        let edges1 = self.vis.edge_rings[r1].edges.clone();
        let edges2 = self.vis.edge_rings[r2].edges.clone();
        for &e1 in &edges1 {
            for &e2 in &edges2 {
                let (a1, a2) = (self.vis.edges[e1].node1, self.vis.edges[e1].node2);
                if self.edge_has_node(e2, a1) || self.edge_has_node(e2, a2) {
                    continue;
                }
                let (p1, p2) = (self.node(a1).clone(), self.node(a2).clone());
                let b1 = self.node(self.vis.edges[e2].node1).clone();
                let b2 = self.node(self.vis.edges[e2].node2).clone();
                if !lines_intersect(p1.x, p1.y, p2.x, p2.y, b1.x, b1.y, b2.x, b2.y) {
                    continue;
                }
                let (ua, ub, ix, iy) =
                    match segment_intersection(p1.x, p1.y, p2.x, p2.y, b1.x, b1.y, b2.x, b2.y) {
                        Some(v) => v,
                        None => continue,
                    };
                let node = self.node_alloc(ix, iy, p1.z);
                let it = Intersection {
                    edge1: e1,
                    edge2: e2,
                    dist1: ua as f32,
                    dist2: ub as f32,
                    node,
                };
                self.vis.edges[e1].intersections.push(it);
                self.vis.edges[e2].intersections.push(it);
                self.vis.graphs_pool[g].nodes.push(node);
                self.vis.graphs_pool[g].intersect_nodes.push(node);
            }
        }
    }

    /// `checkEdgeIntersectionSplit`: split each edge at its intersections,
    /// farthest first.
    fn graph_check_edge_intersection_split(&mut self, g: VGraphId, ring: EdgeRingId) {
        let n = self.vis.edge_rings[ring].edges.len();
        for i in (0..n).rev() {
            let e = self.vis.edge_rings[ring].edges[i];
            if self.vis.edges[e].intersections.is_empty() {
                continue;
            }
            let mut its = std::mem::take(&mut self.vis.edges[e].intersections);
            let dist = |it: &Intersection| if it.edge1 == e { it.dist1 } else { it.dist2 };
            its.sort_by(|a, b| {
                dist(a)
                    .partial_cmp(&dist(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for it in its.iter().rev() {
                let ne = self.edge_split(e, it.node);
                self.vis.graphs_pool[g].edges.push(ne);
            }
            self.vis.edges[e].intersections = its;
        }
    }

    /// `VisibilityGraph::checkNodesInObstacles`.
    fn graph_check_nodes_in_obstacles(&mut self, g: VGraphId) {
        let gr = self.vis.graphs_pool[g].clone();
        for &n in gr.nodes.iter().chain(gr.perimeter_nodes.iter()) {
            if gr
                .obstacles
                .iter()
                .any(|&o| self.obstacle_is_node_inside_of(o, n))
            {
                self.node_mut(n).ignore = true;
            }
        }
    }

    /// `VisibilityGraph::calculateNodeVisibility`.
    fn graph_calculate_node_visibility(&mut self, g: VGraphId) {
        let gr = &self.vis.graphs_pool[g];
        let all: Vec<NodeId> = gr
            .nodes
            .iter()
            .chain(gr.perimeter_nodes.iter())
            .copied()
            .collect();
        let skip = |w: &World, n: NodeId| {
            let node = w.node(n);
            node.ignore || node.square.is_some_and(|s| w.sq(s).has(ALL_STAIRS))
        };
        for i in 0..all.len() {
            let n1 = all[i];
            if skip(self, n1) {
                continue;
            }
            for &n2 in &all[i + 1..] {
                if skip(self, n2)
                    || (self.node(n1).has_flag(NODE_FLAG_PERIMETER)
                        && self.node(n2).has_flag(NODE_FLAG_PERIMETER))
                {
                    continue;
                }
                if self.node_is_connected_to(n1, n2) {
                    continue;
                }
                if self.graph_is_visible(g, n1, n2) {
                    self.connect_two_nodes(n1, n2, 0);
                }
            }
        }
    }

    /// `VisibilityGraph::connectVehicleCrawlNodes`.
    fn graph_connect_vehicle_crawl_nodes(&mut self, g: VGraphId) {
        let z = self.graph_z(g);
        let obstacles = self.vis.graphs_pool[g].obstacles.clone();
        for i in 0..obstacles.len() {
            let o = obstacles[i];
            let ob = self.vis.obstacles[o].clone();
            if ob.vehicle.is_none() || ob.node_crawl_front.is_none() {
                continue;
            }
            let cn = &ob.crawl_nodes;
            for j in (0..cn.len()).step_by(3) {
                let (l, m, r) = (cn[j], cn[j + 1], cn[j + 2]);
                self.connect_two_nodes(l, m, 0);
                self.connect_two_nodes(r, m, 0);
                if j + 3 < cn.len() {
                    self.connect_two_nodes(m, cn[j + 4], 0);
                }
            }
            if !cn.is_empty() {
                self.connect_two_nodes(ob.node_crawl_front.unwrap(), cn[cn.len() - 2], 0);
                self.connect_two_nodes(ob.node_crawl_rear.unwrap(), cn[1], 0);
                self.graph_connect_crawl_can_path(g, o, z);
            }
            for &o2 in &obstacles[i + 1..] {
                let ob2 = &self.vis.obstacles[o2];
                if ob2.vehicle.is_none() || ob2.node_crawl_front.is_none() {
                    continue;
                }
                self.obstacle_connect_crawl_nodes(g, o, o2);
                self.obstacle_connect_crawl_nodes(g, o2, o);
            }
        }
    }

    /// The can-path part of `connectVehicleCrawlNodes`: reroute door,
    /// window and fence connections from squares under a vehicle through
    /// its crawl nodes.
    fn graph_connect_crawl_can_path(&mut self, g: VGraphId, o: ObstacleId, z: i32) {
        let b = self.vis.obstacles[o].bounds;
        let min_x = fastfloor(b.x);
        let min_y = fastfloor(b.y);
        let max_x = (b.x + b.w).ceil() as i32;
        let max_y = (b.y + b.h).ceil() as i32;
        for y in min_y..max_y {
            for x in min_x..max_x {
                let s = match self.get_square(x, y, z) {
                    Some(s) => s,
                    None => continue,
                };
                if !self.obstacle_is_point_inside(o, x as f32 + 0.5, y as f32 + 0.5, 0) {
                    continue;
                }
                let inside = self.get_node_for_square(s);
                let mut k = self.node(inside).visible.len();
                while k > 0 {
                    k -= 1;
                    let c: ConnId = self.node(inside).visible[k];
                    let cxn = self.vis.connections[c];
                    if !cxn.has(CXN_FLAG_CAN_PATH) {
                        continue;
                    }
                    let outside = cxn.other(inside);
                    let (ix, iy) = (self.node(inside).x, self.node(inside).y);
                    let mid = self.obstacle_closest_interior_crawl_node(o, ix, iy);
                    let outer = self.vis.edge_rings[self.vis.obstacles[o].outer]
                        .edges
                        .clone();
                    for e in outer {
                        let ed = self.vis.edges[e].clone();
                        let (p1, p2) = (self.node(ed.node1).clone(), self.node(ed.node2).clone());
                        let (q1, q2) = (self.node(cxn.node1).clone(), self.node(cxn.node2).clone());
                        let (_, _, sx, sy) = match segment_intersection(
                            p1.x, p1.y, p2.x, p2.y, q1.x, q1.y, q2.x, q2.y,
                        ) {
                            Some(v) => v,
                            None => continue,
                        };
                        let split = self.node_alloc(sx, sy, z);
                        self.node_mut(split).flags |= NODE_FLAG_CRAWL;
                        let connected = self.node_is_connected_to(ed.node1, ed.node2);
                        let ne = self.edge_split(e, split);
                        if connected {
                            let (a, b) = (self.vis.edges[e].node1, self.vis.edges[e].node2);
                            self.connect_two_nodes(a, b, 0);
                            let (a, b) = (self.vis.edges[ne].node1, self.vis.edges[ne].node2);
                            self.connect_two_nodes(a, b, 0);
                        }
                        self.vis.graphs_pool[g].edges.push(ne);
                        self.vis.graphs_pool[g].nodes.push(split);
                        self.connect_two_nodes(
                            outside,
                            split,
                            (cxn.flags & CXN_FLAG_THUMP) | CXN_FLAG_CAN_PATH,
                        );
                        if let Some(mid) = mid {
                            self.connect_two_nodes(split, mid, 0);
                        }
                        break;
                    }
                    self.break_connection(c);
                }
            }
        }
    }
}
