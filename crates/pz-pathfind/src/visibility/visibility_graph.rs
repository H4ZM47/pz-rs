//! `VisibilityGraph`, `Obstacle`, `Edge` and `EdgeRing`.

use super::{ClusterId, EdgeId, EdgeRingId, NodeId, ObstacleId, VGraphId, VehicleId};
use crate::world::{ChunkId, SquareId, World};

/// `Edge`: one side of an obstacle polygon.
#[derive(Clone, Debug)]
pub struct Edge {
    pub node1: NodeId,
    pub node2: NodeId,
    pub obstacle: Option<ObstacleId>,
    pub edge_ring: EdgeRingId,
    pub normal: (f32, f32),
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
    pub bounds: (f32, f32, f32, f32),
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
    pub intersect_nodes: Vec<NodeId>,
    pub perimeter_nodes: Vec<NodeId>,
    pub edges: Vec<EdgeId>,
    pub obstacles: Vec<ObstacleId>,
    pub overlapped_chunks: Vec<ChunkId>,
}

impl World {
    /// `Edge::hasNode`.
    pub(crate) fn edge_has_node(&self, e: EdgeId, n: NodeId) -> bool {
        let edge = &self.vis.edges[e];
        edge.node1 == n || edge.node2 == n
    }

    /// `Obstacle::hasNode`.
    pub(crate) fn obstacle_has_node(&self, o: ObstacleId, n: NodeId) -> bool {
        let ob = &self.vis.obstacles[o];
        std::iter::once(ob.outer)
            .chain(ob.inner.iter().copied())
            .any(|r| {
                self.vis.edge_rings[r]
                    .edges
                    .iter()
                    .any(|&e| self.edge_has_node(e, n))
            })
    }

    /// `Obstacle::hasAdjacentNodes`.
    pub(crate) fn obstacle_has_adjacent_nodes(&self, o: ObstacleId, a: NodeId, b: NodeId) -> bool {
        let ob = &self.vis.obstacles[o];
        std::iter::once(ob.outer)
            .chain(ob.inner.iter().copied())
            .any(|r| {
                self.vis.edge_rings[r]
                    .edges
                    .iter()
                    .any(|&e| self.edge_has_node(e, a) && self.edge_has_node(e, b))
            })
    }

    /// `VisibilityGraph::create`.
    pub(crate) fn graph_create(&mut self, g: VGraphId) {
        self.vis.graphs_pool[g].created = true;
    }

    /// `PolygonalMap2::getVisGraphAt(x, y, z, expand)`.
    pub(crate) fn get_vis_graph_at(
        &self,
        _x: f32,
        _y: f32,
        _z: i32,
        _expand: i32,
    ) -> Option<VGraphId> {
        None
    }

    /// `PolygonalMap2::getVisGraphForSquare(square, expand)`.
    pub(crate) fn get_vis_graph_for_square(&self, _s: SquareId, _expand: i32) -> Option<VGraphId> {
        None
    }
}

/// `AdjustStartEndNodeData`: how `getPointOutsideObstacles` moved a start
/// or goal point off an obstacle.
#[derive(Clone, Debug, Default)]
pub struct AdjustStartEndNodeData {
    pub graph: Option<VGraphId>,
    pub node: Option<NodeId>,
    pub new_edge: Option<EdgeId>,
    pub is_node_new: bool,
}

impl World {
    /// `PolygonalMap2::getVisibilityGraphsOverlappingChunk`.
    pub(crate) fn get_visibility_graphs_overlapping_chunk(
        &self,
        _c: ChunkId,
        _level: i32,
        _out: &mut Vec<VGraphId>,
    ) {
    }

    /// `PolygonalMap2::getVisibilityGraphsAdjacentToChunk`.
    pub(crate) fn get_visibility_graphs_adjacent_to_chunk(
        &self,
        _c: ChunkId,
        _level: i32,
        _out: &mut Vec<VGraphId>,
    ) {
    }

    /// `VisibilityGraph::getPointOutsideObstacles`.
    pub(crate) fn graph_get_point_outside_obstacles(
        &mut self,
        _g: VGraphId,
        _x: f32,
        _y: f32,
        _z: f32,
        _data: &mut AdjustStartEndNodeData,
    ) -> i32 {
        0
    }

    /// `VisibilityGraph::addNode`.
    pub(crate) fn graph_add_node(&mut self, _g: VGraphId, _n: NodeId) {}

    /// `VisibilityGraph::removeNode`.
    pub(crate) fn graph_remove_node(&mut self, _g: VGraphId, _n: NodeId) {}

    /// `VisibilityGraph::release`.
    pub(crate) fn graph_release(&mut self, _g: VGraphId) {}

    /// Undo the edge split made by `getPointOutsideObstacles`.
    pub(crate) fn undo_adjust_start_end(&mut self, _data: &AdjustStartEndNodeData) {}

    /// `PolygonalMap2::createVisibilityGraphs`.
    pub(crate) fn create_visibility_graphs(&mut self) {}
}
